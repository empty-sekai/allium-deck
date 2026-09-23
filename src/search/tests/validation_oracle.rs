//! Deterministic WL / finale validation over the full ordered feasible set.
//!
//! `ExactOracle` has its own enumeration, public-set deduplication and canonical
//! comparator. It never calls production bounds, dominance, placement or tracker
//! code. Only the concrete leaf evaluator is shared: this matrix proves search
//! equivalence to those scoring semantics, not the game's scoring formulas.
//!
//! Every context is exhaustively enumerated once at K=100. Its prefixes are the
//! independent references for all four production K values, including each
//! returned leader, member order and cultivation variant. No score tolerance or
//! Top-1-only comparison is used.
use super::*;
use std::collections::BTreeSet;

const TOP_K: [usize; 4] = [1, 8, 30, 100];
const DEFAULT_SEED: u64 = 0x574C_F1A1_2026_0922;
const PROFILES: [&str; 3] = ["random", "variants", "ties"];
const SCENES: [&str; 4] = ["wl-multi", "wl-solo", "final-multi", "final-solo"];
const CONSTRAINTS: [&str; 6] = [
    "auto",
    "forced-leader",
    "fixed-character",
    "fixed-card",
    "fixed-card-and-character",
    "absent-leader",
];
const ORDERS: [LiveSkillOrder; 4] = [
    LiveSkillOrder::Average,
    LiveSkillOrder::Best,
    LiveSkillOrder::Worst,
    LiveSkillOrder::Specific,
];

#[derive(Default, serde::Serialize)]
struct MatrixCounts {
    contexts: u64,
    searches: u64,
    searches_by_k: [u64; 4],
    oracle_assignments: u64,
    oracle_accepted: u64,
    oracle_rejected: u64,
    oracle_rows: u64,
    compared_rows: u64,
    complete_searches: u64,
    empty_contexts: u64,
    full_top_100_searches: u64,
    tied_adjacent_oracle_rows: u64,
    support_exclusion_rows: u64,
    nonmonotone_attribute_contexts: u64,
    constrained_contexts: u64,
    variant_contexts: u64,
}

impl MatrixCounts {
    fn add(&mut self, other: &Self) {
        self.contexts += other.contexts;
        self.searches += other.searches;
        for (total, part) in self.searches_by_k.iter_mut().zip(other.searches_by_k) {
            *total += part;
        }
        self.oracle_assignments += other.oracle_assignments;
        self.oracle_accepted += other.oracle_accepted;
        self.oracle_rejected += other.oracle_rejected;
        self.oracle_rows += other.oracle_rows;
        self.compared_rows += other.compared_rows;
        self.complete_searches += other.complete_searches;
        self.empty_contexts += other.empty_contexts;
        self.full_top_100_searches += other.full_top_100_searches;
        self.tied_adjacent_oracle_rows += other.tied_adjacent_oracle_rows;
        self.support_exclusion_rows += other.support_exclusion_rows;
        self.nonmonotone_attribute_contexts += other.nonmonotone_attribute_contexts;
        self.constrained_contexts += other.constrained_contexts;
        self.variant_contexts += other.variant_contexts;
    }
}

/// A reproducibility checksum, not a cryptographic integrity claim.
struct AuditDigest(u64);

impl AuditDigest {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn record(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn matrix_cards(seed: u64, profile: &str) -> Vec<TestCard> {
    // The tie box has 102 distinct legal five-card sets, so K=100 is exercised
    // both as a filled ranking and, under fixed constraints, as an oversized K.
    let (count, characters) = if profile == "ties" { (10, 7) } else { (9, 6) };
    let mut cards = randomized_exact_cards(seed, count, characters);
    for (index, card) in cards.iter_mut().enumerate() {
        // Public-ID order deliberately disagrees with dense / numeric order.
        card.game_id = 4_000 + ((index * 7) % count) as u16 * 13;
        card.limited_bonus = if profile == "variants" {
            (index % 3) as u8 * 10
        } else {
            20
        };
        if profile == "ties" {
            card.power = 1_000;
            card.power_max = 1_000;
            card.skill = SkillSlot {
                skill_type: 0,
                value: 50,
            };
            card.skill_max = 50;
            card.base_bonus = 10;
            card.attr = (seed % 5) as u8;
            card.unit_mask = 1;
        }
    }
    if profile == "variants" {
        let mut variant = cards[0];
        // Identity / character / attribute / unit remain those of the same
        // public card. Numeric states trade power for skill and are exclusive.
        variant.power += 37;
        variant.power_max = variant.power;
        variant.skill.value = variant.skill.value.saturating_sub(9);
        variant.skill_max = variant.skill.value;
        cards.push(variant);
    }
    let mut rng = ExactLcg(seed ^ 0x5A11_FF1E);
    for index in (1..cards.len()).rev() {
        let other = rng.range(0, (index + 1) as u32) as usize;
        cards.swap(index, other);
    }
    cards
}

fn matrix_support(pool: &CardPool, seed: u64, leader: u8, tied: bool) -> SupportDeck {
    let ids = pool
        .indices()
        .map(|card| pool.game_id(card))
        .collect::<BTreeSet<_>>();
    let mut cards = ids
        .into_iter()
        .map(|id| {
            let bonus = if tied {
                1.25
            } else {
                let rank = (u64::from(id) * 17 + seed % 31 + u64::from(leader) * 7) % 29;
                8.0 + rank as f64 * 0.25
            };
            (id, bonus)
        })
        .collect::<Vec<_>>();
    // Reserves survive even when all five main-deck IDs occupy support slots.
    for index in 0..DECK_SIZE {
        cards.push((60_000 + index as u16, if tied { 1.25 } else { 1.0 }));
    }
    cards.sort_unstable_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    SupportDeck { cards, count: 4 }
}

fn card_for_character(pool: &CardPool, character: u8) -> u16 {
    pool.indices()
        .filter(|&card| pool.char_id(card) == character)
        .map(|card| pool.game_id(card))
        .min()
        .expect("matrix fixture contains the requested character")
}

fn matrix_context(
    pool: &CardPool,
    seed: u64,
    profile: &str,
    scene: &str,
    order: LiveSkillOrder,
    constraint: &str,
) -> SearchContext {
    let tied = profile == "ties";
    let mut ctx = ready_ctx(pool, ScoreTarget::Score);
    ctx.is_world_bloom = true;
    ctx.is_final_chapter = scene.starts_with("final-");
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.live_type = if scene.ends_with("solo") {
        LiveType::Solo
    } else {
        LiveType::Multi
    };
    ctx.best_skill_as_leader = false;
    ctx.live_skill_order = order;
    ctx.specific_skill_order = (order == LiveSkillOrder::Specific).then_some([4, 1, 3, 0, 2]);
    ctx.skill_scores = [
        [0.07, 0.19, 0.41, 0.31, 0.13, 0.37],
        [0.11, 0.09, 0.43, 0.13, 0.31, 0.27],
        [0.17, 0.23, 0.11, 0.29, 0.05, 0.31],
    ];
    ctx.base_score = 1.125;
    ctx.base_score_auto = 0.75;
    ctx.fever_score = 0.2;
    ctx.music_rate_pct = 115;
    ctx.boost_rate_pct = 500;
    // Nonmonotone tables ensure a larger reachable attribute count is never
    // substituted for max(bonus) over the full reachable union-state set.
    ctx.diff_attr_bonus = if seed & 1 == 0 {
        [0, 13, 71, 5, 43, 19]
    } else {
        [0, 29, 7, 83, 11, 41]
    };
    ctx.support_deck = matrix_support(pool, seed, 0, tied);
    if ctx.is_final_chapter {
        ctx.card_bonus_count_limit = 4;
        ctx.support_decks_by_character = (0..=26)
            .map(|leader| matrix_support(pool, seed, leader, tied))
            .collect();
        for card in pool.indices() {
            let id = u64::from(pool.game_id(card));
            if !tied {
                ctx.leader_honor_bonus_x10[card.raw()] = ((id + seed % 37) % 19) as u16;
                ctx.leader_limit_bonus_x10[card.raw()] = ((id * 3 + seed % 23) % 17) as u16;
            }
        }
    }
    if !tied && seed & 2 != 0 {
        ctx.power_total_cap = Some(9_000);
    }
    let support_max = std::iter::once(&ctx.support_deck)
        .chain(ctx.support_decks_by_character.iter())
        .map(|support| {
            support
                .cards
                .iter()
                .take(support.count as usize)
                .map(|row| row.1)
                .sum::<f64>()
        })
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    ctx.extra_bonus_ub = support_max + u32::from(*ctx.diff_attr_bonus.iter().max().unwrap());
    match constraint {
        "auto" => {}
        "forced-leader" => ctx.forced_leader_character_id = Some(2),
        "fixed-character" => ctx.fixed_character_ids = vec![1],
        "fixed-card" => ctx.fixed_card_ids = vec![card_for_character(pool, 2)],
        "fixed-card-and-character" => {
            ctx.fixed_card_ids = vec![card_for_character(pool, 1)];
            ctx.fixed_character_ids = vec![3];
        }
        "absent-leader" => ctx.forced_leader_character_id = Some(26),
        _ => unreachable!("unknown matrix constraint"),
    }
    ctx
}

fn compare_rows(
    pool: &CardPool,
    ctx: &SearchContext,
    actual: &[DeckResult],
    expected: &[DeckResult],
    label: &str,
) {
    if actual != expected {
        eprintln!(
            "VALIDATION_ORACLE_MISMATCH {}",
            serde_json::json!({
                "case": label,
                "forced_leader_character_id": ctx.forced_leader_character_id,
                "fixed_card_ids": ctx.fixed_card_ids,
                "fixed_character_ids": ctx.fixed_character_ids,
                "actual": actual.iter().map(|row| audit_row(pool, ctx, row)).collect::<Vec<_>>(),
                "expected": expected.iter().map(|row| audit_row(pool, ctx, row)).collect::<Vec<_>>(),
            })
        );
    }
    assert_eq!(actual.len(), expected.len(), "{label}: Top-K cardinality");
    let mut seen = BTreeSet::new();
    for (rank, (got, want)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(got.score, want.score, "{label}: rank={rank} exact score");
        assert_eq!(
            got.game_card_set_key(pool),
            want.game_card_set_key(pool),
            "{label}: rank={rank} canonical public set"
        );
        assert_eq!(
            pool.game_id(got.cards[0]),
            pool.game_id(want.cards[0]),
            "{label}: rank={rank} leader identity"
        );
        assert_eq!(
            got.cards.map(|card| pool.game_id(card)),
            want.cards.map(|card| pool.game_id(card)),
            "{label}: rank={rank} complete canonical slot order"
        );
        assert_eq!(
            got.cards, want.cards,
            "{label}: rank={rank} cultivation variants"
        );
        assert_eq!(
            evaluate::leaf_evaluate_checked(pool, ctx, &got.cards),
            Some(got.score),
            "{label}: rank={rank} exact leaf re-evaluation"
        );
        assert!(
            seen.insert(got.game_card_set_key(pool)),
            "{label}: duplicate public set"
        );
        assert!(
            got.game_card_set_key(pool)
                .windows(2)
                .all(|pair| pair[0] != pair[1]),
            "{label}: two cultivation variants of one card entered one deck"
        );
    }
}

fn audit_row(pool: &CardPool, ctx: &SearchContext, row: &DeckResult) -> serde_json::Value {
    let ids = row.cards.map(|card| pool.game_id(card));
    let leader = row.cards[0];
    let support = ctx.support_deck_for_leader(pool.char_id(leader));
    let support_after_exclusion = support
        .cards
        .iter()
        .filter(|(id, _)| !ids.contains(id))
        .take(support.count as usize)
        .copied()
        .collect::<Vec<_>>();
    serde_json::json!({
        "score": row.score,
        "dense": row.cards.map(|card| card.raw()),
        "ordered_ids": ids,
        "characters": row.cards.map(|card| pool.char_id(card)),
        "leader_character": pool.char_id(leader),
        "leader_honor_x10": ctx.leader_honor_bonus_x10_at(leader.raw()),
        "leader_limited_x10": ctx.leader_limit_bonus_x10_at(leader.raw()),
        "total_bonus": evaluate::resolve_total_bonus(pool, ctx, &row.cards),
        "support_after_exclusion": support_after_exclusion,
        "leaf_score": evaluate::leaf_evaluate_checked(pool, ctx, &row.cards),
        "display_ordered_ids": summarize_deck(pool, ctx, &row.cards)
            .map(|summary| summary.ordered_cards.map(|card| pool.game_id(card))),
    })
}

#[test]
fn validation_oracle_final_forced_leader_survives_top_k_reconstruction() {
    // Five mandatory public cards leave exactly one set. Character 3 has an
    // attractive leader-only bonus, but the request fixes character 1 as leader.
    // Top-1 skips reconstruction. Top-8 used to rotate character 3 into slot 0
    // because reconstruction checked fixed slots but omitted forced-leader.
    let cards = (1..=5u8)
        .map(|character| skill_card(100 + u16::from(character), character, 1_000, 10))
        .collect::<Vec<_>>();
    let pool = build_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.is_final_chapter = true;
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.live_type = LiveType::Multi;
    ctx.live_skill_order = LiveSkillOrder::Average;
    ctx.best_skill_as_leader = false;
    ctx.forced_leader_character_id = Some(1);
    ctx.leader_honor_bonus_x10[2] = 500;
    let (oracle, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
        top_k: 100,
        timeout_ms: 0,
    });
    assert_eq!(oracle.len(), 1);
    let mut first = None;
    for top_k in TOP_K {
        let outcome = crate::search::search(
            &pool,
            &ctx,
            &SearchParams {
                top_k,
                timeout_ms: 0,
            },
        );
        assert_eq!(outcome.completion(), SearchCompletion::Complete);
        compare_rows(
            &pool,
            &ctx,
            &outcome.results,
            &oracle,
            &format!("five-card forced-final leader top_k={top_k}"),
        );
        for row in &outcome.results {
            assert_eq!(pool.char_id(row.cards[0]), 1);
        }
        let top = outcome.results[0];
        assert_eq!(
            *first.get_or_insert(top),
            top,
            "Top-K must not change Top-1"
        );
    }
}

#[test]
fn validation_oracle_final_conflicting_leader_roles_have_no_result() {
    // Exercise both grouped Final and the mixed-bonus fallback. A forced
    // character cannot override a different fixed card or character in slot 0.
    for profile in ["random", "variants"] {
        let pool = build_pool(&matrix_cards(DEFAULT_SEED, profile));
        for fixed_card in [false, true] {
            for order in ORDERS {
                let mut ctx = matrix_context(
                    &pool,
                    DEFAULT_SEED,
                    profile,
                    "final-multi",
                    order,
                    "forced-leader",
                );
                if fixed_card {
                    ctx.fixed_card_ids = vec![card_for_character(&pool, 1)];
                } else {
                    ctx.fixed_character_ids = vec![1];
                }
                let (expected, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
                    top_k: 100,
                    timeout_ms: 0,
                });
                assert!(expected.is_empty());
                for top_k in TOP_K {
                    let outcome = crate::search::search(
                        &pool,
                        &ctx,
                        &SearchParams {
                            top_k,
                            timeout_ms: 0,
                        },
                    );
                    assert_eq!(outcome.completion(), SearchCompletion::Complete);
                    assert!(
                        outcome.results.is_empty(),
                        "conflicting Final roles profile={profile} fixed_card={fixed_card} order={order:?} top_k={top_k}"
                    );
                }
            }
        }
    }
}

fn run_matrix(seed_count: usize, start_seed: u64, mode: &str) {
    let mut total = MatrixCounts::default();
    let mut digest = AuditDigest::new();
    for index in 0..seed_count {
        let seed = start_seed.wrapping_add(index as u64);
        let mut counts = MatrixCounts::default();
        for profile in PROFILES {
            let cards = matrix_cards(seed, profile);
            let pool = build_pool(&cards);
            for scene in SCENES {
                for order in ORDERS {
                    for constraint in CONSTRAINTS {
                        let ctx = matrix_context(&pool, seed, profile, scene, order, constraint);
                        let label = format!(
                            "seed=0x{seed:016x} profile={profile} scene={scene} order={order:?} constraint={constraint}"
                        );
                        digest.record(label.as_bytes());
                        let (oracle, stats) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
                            top_k: 100,
                            timeout_ms: 0,
                        });
                        counts.contexts += 1;
                        counts.oracle_assignments += stats.candidates;
                        counts.oracle_accepted += stats.evaluated;
                        counts.oracle_rejected += stats.invalid;
                        counts.oracle_rows += oracle.len() as u64;
                        counts.empty_contexts += u64::from(oracle.is_empty());
                        counts.constrained_contexts += u64::from(constraint != "auto");
                        counts.variant_contexts += u64::from(profile == "variants");
                        counts.nonmonotone_attribute_contexts += 1;
                        counts.tied_adjacent_oracle_rows += oracle
                            .windows(2)
                            .filter(|rows| rows[0].score == rows[1].score)
                            .count()
                            as u64;
                        for row in &oracle {
                            let ordered_ids = row.cards.map(|card| pool.game_id(card));
                            let support = ctx.support_deck_for_leader(pool.char_id(row.cards[0]));
                            counts.support_exclusion_rows += u64::from(
                                support
                                    .cards
                                    .iter()
                                    .take(support.count as usize)
                                    .any(|&(id, _)| ordered_ids.contains(&id)),
                            );
                            digest.record(&row.score.to_le_bytes());
                            for (card, id) in row.cards.iter().zip(ordered_ids) {
                                digest.record(&id.to_le_bytes());
                                digest.record(&(card.raw() as u64).to_le_bytes());
                            }
                        }
                        for (k_index, top_k) in TOP_K.into_iter().enumerate() {
                            let outcome = crate::search::search(
                                &pool,
                                &ctx,
                                &SearchParams {
                                    top_k,
                                    timeout_ms: 0,
                                },
                            );
                            let check_label = format!("{label} top_k={top_k}");
                            assert_eq!(
                                outcome.completion(),
                                SearchCompletion::Complete,
                                "{check_label}: partial result cannot certify this comparison"
                            );
                            assert!(!outcome.stats.deadline_hit, "{check_label}: deadline flag");
                            compare_rows(
                                &pool,
                                &ctx,
                                &outcome.results,
                                &oracle[..top_k.min(oracle.len())],
                                &check_label,
                            );
                            counts.searches += 1;
                            counts.searches_by_k[k_index] += 1;
                            counts.complete_searches += 1;
                            counts.compared_rows += outcome.results.len() as u64;
                            counts.full_top_100_searches +=
                                u64::from(top_k == 100 && outcome.results.len() == 100);
                        }
                    }
                }
            }
        }
        assert_eq!(counts.contexts, 288);
        assert_eq!(counts.searches, 1_152);
        assert_eq!(counts.complete_searches, counts.searches);
        assert!(counts.oracle_assignments > 1_000);
        assert!(counts.full_top_100_searches > 0);
        assert!(counts.tied_adjacent_oracle_rows > 0);
        assert!(counts.support_exclusion_rows > 0);
        assert!(counts.empty_contexts > 0);
        eprintln!(
            "VALIDATION_ORACLE_SEED {}",
            serde_json::json!({
                "schema": 1, "mode": mode, "index": index, "seed": format!("0x{seed:016x}"),
                "counts": counts, "running_oracle_fnv1a64": format!("{:016x}", digest.0),
            })
        );
        total.add(&counts);
    }
    eprintln!(
        "VALIDATION_ORACLE_SUMMARY {}",
        serde_json::json!({
            "schema": 1, "mode": mode, "seeds": seed_count,
            "start_seed": format!("0x{start_seed:016x}"), "profiles": PROFILES,
            "scenes": SCENES, "constraints": CONSTRAINTS,
            "orders": ["average", "best", "worst", "specific"], "top_k": TOP_K,
            "oracle": "ExactOracle independent complete ordered enumeration; shared leaf evaluator",
            "counts": total, "oracle_fnv1a64": format!("{:016x}", digest.0),
        })
    );
}

fn environment_u64(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(value) => value
            .strip_prefix("0x")
            .map_or_else(|| value.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
            .unwrap_or_else(|error| panic!("{name}={value:?}: {error}")),
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("{name}: {error}"),
    }
}

#[test]
fn validation_oracle_wl_final_matrix() {
    run_matrix(1, DEFAULT_SEED, "regular");
}

#[test]
#[ignore = "extended exhaustive WL/finale matrix; run release with --ignored --nocapture"]
fn validation_oracle_extended() {
    let seeds = usize::try_from(environment_u64("ALLIUM_VALIDATION_ORACLE_SEEDS", 8))
        .expect("seed count fits usize");
    assert!(seeds > 0, "ALLIUM_VALIDATION_ORACLE_SEEDS must be positive");
    let start = environment_u64("ALLIUM_VALIDATION_ORACLE_START_SEED", DEFAULT_SEED);
    run_matrix(seeds, start, "extended");
}

#[test]
fn validation_oracle_final_fallback_uses_leader_support_for_bounds() {
    // Mixed limited bonuses route Final through the slot-aware DFS. Its support
    // bound must use the current leader's support table, not the global fallback.
    // Removing the cultivation variant preserves the counterexample: the issue
    // is leader-dependent support, not same-public-ID deduplication.
    for remove_variant in [false, true] {
        let mut cards = matrix_cards(DEFAULT_SEED, "variants");
        if remove_variant {
            let mut seen = BTreeSet::new();
            cards.retain(|card| seen.insert(card.game_id));
        }
        let pool = build_pool(&cards);
        let ctx = matrix_context(
            &pool,
            DEFAULT_SEED,
            "variants",
            "final-multi",
            LiveSkillOrder::Average,
            "auto",
        );
        assert!(placement::bonus_order_observable(
            &pool,
            &ctx,
            &pool.indices().collect::<Vec<_>>()
        ));
        let (expected, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
            top_k: 100,
            timeout_ms: 0,
        });
        for top_k in TOP_K {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            for direct_unseeded in [false, true] {
                let (actual, stats) = if direct_unseeded {
                    let suffix = SuffixBound::build(&pool, &ctx);
                    let mut budget = crate::search::budget::SearchBudget::from_params(&params);
                    dfs::dfs_search_with_budget(
                        &pool,
                        &ctx,
                        &suffix,
                        &params,
                        Vec::new(),
                        0,
                        &mut budget,
                    )
                } else {
                    search_instrumented(&pool, &ctx, &params)
                };
                assert_eq!(stats.completion(), SearchCompletion::Complete);
                compare_rows(
                    &pool,
                    &ctx,
                    &actual,
                    &expected[..top_k.min(expected.len())],
                    &format!(
                        "Final fallback support remove_variant={remove_variant} unseeded={direct_unseeded} top_k={top_k}"
                    ),
                );
            }
        }
    }
}

#[test]
fn validation_oracle_final_skill_peak_bounds_match_orders_and_leader_modes() {
    let seed = 0x574C_F1A1_2026_0937;
    let pool = build_pool(&matrix_cards(seed, "random"));
    for live_type in [LiveType::Solo, LiveType::Auto] {
        for order in [
            LiveSkillOrder::Best,
            LiveSkillOrder::Worst,
            LiveSkillOrder::Specific,
        ] {
            for constraint in ["forced-leader", "auto"] {
                let mut ctx =
                    matrix_context(&pool, seed, "random", "final-solo", order, constraint);
                ctx.live_type = live_type;
                let (expected, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
                    top_k: 100,
                    timeout_ms: 0,
                });
                assert!(!expected.is_empty());
                for top_k in TOP_K {
                    let params = SearchParams {
                        top_k,
                        timeout_ms: 0,
                    };
                    let mut unbounded = None;
                    for bounds in [false, true] {
                        let configuration = tuning::SearchTuning {
                            bounds,
                            ..Default::default()
                        };
                        let outcome = tuning::with_tuning(configuration, || {
                            crate::search::search(&pool, &ctx, &params)
                        });
                        assert_eq!(outcome.completion(), SearchCompletion::Complete);
                        let label = format!(
                            "seed0937 Final skill peak live={live_type:?} order={order:?} constraint={constraint} top_k={top_k} bounds={bounds}"
                        );
                        compare_rows(
                            &pool,
                            &ctx,
                            &outcome.results,
                            &expected[..top_k.min(expected.len())],
                            &label,
                        );
                        if bounds {
                            assert_eq!(
                                outcome.results.as_slice(),
                                unbounded.as_deref().unwrap(),
                                "{label}: bounds change canonical Top-K"
                            );
                        } else {
                            unbounded = Some(outcome.results);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn validation_oracle_wl_variant_reconstruction_seed_0947_matches_complete_top_k() {
    let seed = 0x574C_F1A1_2026_0947;
    let pool = build_pool(&matrix_cards(seed, "variants"));
    // Cover generic reconstruction, both Final member passes, and auto-leader
    // rotation. Bound correctness is checked separately by the larger matrix.
    for scene in ["wl-solo", "final-solo"] {
        for constraint in ["forced-leader", "auto"] {
            let ctx = matrix_context(
                &pool,
                seed,
                "variants",
                scene,
                LiveSkillOrder::Worst,
                constraint,
            );
            let (expected, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
                top_k: 100,
                timeout_ms: 0,
            });
            assert!(!expected.is_empty());
            for top_k in TOP_K {
                for dominance in [false, true] {
                    let configuration = tuning::SearchTuning {
                        dominance,
                        ..Default::default()
                    };
                    let outcome = tuning::with_tuning(configuration, || {
                        crate::search::search(
                            &pool,
                            &ctx,
                            &SearchParams {
                                top_k,
                                timeout_ms: 0,
                            },
                        )
                    });
                    assert_eq!(outcome.completion(), SearchCompletion::Complete);
                    compare_rows(
                        &pool,
                        &ctx,
                        &outcome.results,
                        &expected[..top_k.min(expected.len())],
                        &format!(
                            "seed0947 scene={scene} constraint={constraint} K={top_k} dominance={dominance}"
                        ),
                    );
                }
            }
        }
    }
}

#[test]
fn validation_oracle_singleton_root_still_needs_other_member_variants() {
    let card = |game_id, char_id, power, skill| TestCard {
        game_id,
        char_id,
        power,
        skill: SkillSlot {
            skill_type: 0,
            value: skill,
        },
        attr: 0,
        unit_mask: 1,
        base_bonus: 0,
        limited_bonus: 0,
        power_max: power,
        skill_max: skill,
    };
    let pool = build_pool(&[
        card(10, 1, 100, 100), // Singleton root A.
        card(11, 1, 100, 0),   // Dominated a: replacing A changes the best B state.
        card(20, 2, 200, 0),   // B1 wins with A.
        card(20, 2, 100, 30),  // B2 wins with a.
        card(30, 3, 100, 0),
        card(40, 4, 100, 0),
        card(50, 5, 100, 0),
    ]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.best_skill_as_leader = false;
    ctx.forced_leader_character_id = Some(3);
    ctx.live_skill_order = LiveSkillOrder::Average;
    // A single coefficient avoids adding five rounded 0.2 contributions.
    ctx.skill_scores[0] = [5.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let deck = |a, b| {
        [
            CardIdx::new(a),
            CardIdx::new(b),
            CardIdx::new(4),
            CardIdx::new(5),
            CardIdx::new(6),
        ]
    };
    for (a, b, live) in [(0, 2, 4800u32), (0, 3, 4600), (1, 2, 2400), (1, 3, 2600)] {
        let score = evaluate::leaf_evaluate_checked(&pool, &ctx, &deck(a, b)).unwrap();
        assert_eq!(score, (u64::from(live) << 32) | u64::from(live));
    }
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&SearchParams {
        top_k: 8,
        timeout_ms: 0,
    });
    assert_eq!(expected.len(), 2);
    assert_eq!(expected[0].score as u32, 4800);
    assert_eq!(expected[1].score as u32, 2600);
    assert!(expected[0].cards.contains(&CardIdx::new(2)));
    assert!(expected[1].cards.contains(&CardIdx::new(3)));
    for bounds in [false, true] {
        for dominance in [false, true] {
            let configuration = tuning::SearchTuning {
                bounds,
                dominance,
                ..Default::default()
            };
            let outcome = tuning::with_tuning(configuration, || {
                crate::search::search(
                    &pool,
                    &ctx,
                    &SearchParams {
                        top_k: 8,
                        timeout_ms: 0,
                    },
                )
            });
            assert_eq!(outcome.completion(), SearchCompletion::Complete);
            compare_rows(
                &pool,
                &ctx,
                &outcome.results,
                &expected,
                &format!(
                    "singleton root, another member changes variant, bounds={bounds}, dominance={dominance}"
                ),
            );
        }
    }
}
