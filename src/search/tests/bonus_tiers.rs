//! Exact bonus tiers against the independent ordered-deck oracle.
//!
//! Every comparison is the complete per-tier result sequence, including slot
//! assignment and cultivation variant, never only scores.
use super::*;
use crate::search::evaluate::resolve_total_bonus;

#[derive(Clone, Copy, Debug)]
struct TierCard {
    char_id: u8,
    attr: u8,
    units: u8,
    /// Units whose power profile is the second one.
    second_profile: u8,
    game_id: u16,
    /// Resolved power by `profile * 4 + member key`.
    powers: [u32; 8],
    skill: SkillSlot,
    skill_min: u8,
    skill_max: u8,
    base_x10: u16,
    limited_x10: u16,
}

fn tiered_pool(cards: &[TierCard]) -> CardPool {
    let mut builder = PoolBuilder::new(cards.len() as u16);
    builder.add_unit_count_skill(UnitCountSkill {
        unit: 0,
        score_up: [10, 20, 30, 40, 50],
    });
    builder.add_diff_skill(DiffSkill {
        base: 12,
        increment: 6,
    });
    builder.add_ref_skill(RefSkill { rate: 50, max: 30 });
    for (index, card) in cards.iter().enumerate() {
        let dense = index as u16;
        let mut values = [0u16; 8];
        let mut lut = 0u32;
        for (slot, &power) in card.powers.iter().enumerate() {
            values[slot] = power as u16;
            lut |= ((power >> 16) & 3) << (slot * 2);
        }
        let mut power_max = 0;
        for unit in 0..6u8 {
            if card.units & (1 << unit) == 0 {
                continue;
            }
            let profile = usize::from(card.second_profile & (1 << unit) != 0);
            if profile == 1 {
                lut |= 1 << (16 + unit);
            }
            power_max = card.powers[profile * 4..profile * 4 + 4]
                .iter()
                .copied()
                .fold(power_max, u32::max);
            builder.mark_unit(unit, dense);
        }
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        builder.set_power_max(dense, power_max);
        builder.set_skill(dense, card.skill);
        builder.set_skill_min(dense, card.skill_min);
        builder.set_skill_max(dense, card.skill_max);
        builder.set_event_bonus(
            dense,
            EventBonusExact::from_x10(card.base_x10, card.limited_x10),
        );
        builder.set_char_id(dense, card.char_id);
        builder.set_attr(dense, card.attr);
        builder.set_unit_mask(dense, card.units);
        builder.set_game_id(dense, card.game_id);
        builder.mark_char(card.char_id, dense);
        builder.mark_attr(card.attr, dense);
    }
    builder.freeze()
}

#[derive(Clone, Copy)]
struct CardShape {
    count: usize,
    characters: u8,
    /// Extra dense variants sharing a public card id.
    variants: usize,
    /// Bonus granularity in tenths.
    step_x10: u16,
    limited: bool,
}

fn random_tier_cards(rng: &mut ExactLcg, shape: CardShape) -> Vec<TierCard> {
    let mut cards = Vec::with_capacity(shape.count + shape.variants);
    for index in 0..shape.count {
        let char_id = (index % usize::from(shape.characters)) as u8 + 1;
        let home = char_id % 3;
        let units = if rng.range(0, 5) == 0 {
            (1 << home) | (1 << 5)
        } else {
            1 << home
        };
        let second_profile = if units & (1 << 5) != 0 && rng.range(0, 2) == 0 {
            1 << 5
        } else {
            0
        };
        let base = 700 + rng.range(0, 1900);
        let mut powers = [0u32; 8];
        for profile in 0..2 {
            let own = base + profile as u32 * rng.range(0, 150);
            let attr = rng.range(0, 220);
            let unit = rng.range(0, 220);
            powers[profile * 4] = own;
            powers[profile * 4 + 1] = own + attr;
            powers[profile * 4 + 2] = own + unit;
            powers[profile * 4 + 3] = own + attr + unit + rng.range(0, 40);
        }
        let (skill, skill_min, skill_max) = match rng.range(0, 10) {
            0 => (
                SkillSlot {
                    skill_type: 1,
                    value: 1,
                },
                10,
                50,
            ),
            1 => (
                SkillSlot {
                    skill_type: 2,
                    value: 1,
                },
                12,
                24,
            ),
            2 => (
                SkillSlot {
                    skill_type: 3,
                    value: 1,
                },
                15,
                45,
            ),
            _ => {
                let value = 20 + rng.range(0, 100) as u8;
                (
                    SkillSlot {
                        skill_type: 0,
                        value,
                    },
                    value,
                    value,
                )
            }
        };
        let base_x10 = rng.range(0, 7) as u16 * shape.step_x10;
        let limited_x10 = if shape.limited && rng.range(0, 5) < 2 {
            rng.range(1, 4) as u16 * shape.step_x10
        } else {
            0
        };
        cards.push(TierCard {
            char_id,
            attr: rng.range(0, 3) as u8,
            units,
            second_profile,
            game_id: 30_000 + index as u16,
            powers,
            skill,
            skill_min,
            skill_max,
            base_x10,
            limited_x10,
        });
    }
    for _ in 0..shape.variants {
        let source = rng.range(0, shape.count as u32) as usize;
        let mut variant = cards[source];
        for power in &mut variant.powers {
            *power = (*power + rng.range(0, 300)).saturating_sub(150);
        }
        if variant.skill.skill_type == 0 {
            variant.skill.value = 20 + rng.range(0, 100) as u8;
            variant.skill_min = variant.skill.value;
            variant.skill_max = variant.skill.value;
        }
        variant.base_x10 = rng.range(0, 7) as u16 * shape.step_x10;
        cards.push(variant);
    }
    cards
}

/// Integer totals some ordered deck of the pool reaches, spread over the range.
fn reachable_integer_tiers(pool: &CardPool, ctx: &SearchContext, wanted: usize) -> Vec<i32> {
    let cards = pool.indices().collect::<Vec<_>>();
    let mut totals = std::collections::BTreeSet::new();
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    fn visit(
        pool: &CardPool,
        ctx: &SearchContext,
        cards: &[CardIdx],
        depth: usize,
        deck: &mut [CardIdx; DECK_SIZE],
        totals: &mut std::collections::BTreeSet<i32>,
    ) {
        if depth == DECK_SIZE {
            let total = resolve_total_bonus(pool, ctx, deck);
            if total.fract() == 0.0 && total >= 0.0 {
                totals.insert(total as i32);
            }
            return;
        }
        for &card in cards {
            if deck[..depth]
                .iter()
                .any(|&other| pool.game_id(other) == pool.game_id(card))
            {
                continue;
            }
            deck[depth] = card;
            visit(pool, ctx, cards, depth + 1, deck, totals);
        }
    }
    visit(pool, ctx, &cards, 0, &mut deck, &mut totals);
    let totals = totals.into_iter().collect::<Vec<_>>();
    if totals.is_empty() {
        return Vec::new();
    }
    (0..wanted)
        .map(|index| totals[index * (totals.len() - 1) / wanted.max(2).saturating_sub(1).max(1)])
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum Scene {
    Additive,
    Limited(usize),
    WorldBloom,
    Final(usize),
    Challenge,
}

#[derive(Clone, Copy, Debug)]
enum Constraint {
    Free,
    FixedCard,
    FixedCharacter,
    FixedCardAndCharacter,
    ForcedLeader,
}

fn scene_context(
    pool: &CardPool,
    rng: &mut ExactLcg,
    scene: Scene,
    constraint: Constraint,
) -> SearchContext {
    let mut ctx = ready_ctx(pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::Marathon);
    ctx.live_type = match rng.range(0, 4) {
        0 => LiveType::Solo,
        1 => LiveType::Auto,
        2 => LiveType::Multi,
        _ => {
            ctx.event_type = Some(EventType::CheerfulCarnival);
            LiveType::Multi
        }
    };
    ctx.live_skill_order = match rng.range(0, 4) {
        0 => LiveSkillOrder::Best,
        1 => LiveSkillOrder::Worst,
        2 => LiveSkillOrder::Average,
        _ => LiveSkillOrder::Specific,
    };
    ctx.specific_skill_order =
        (ctx.live_skill_order == LiveSkillOrder::Specific).then_some([4, 2, 0, 3, 1]);
    ctx.best_skill_as_leader = rng.range(0, 3) != 0;
    ctx.skill_reference_strategy = match rng.range(0, 3) {
        0 => SkillReferenceStrategy::Max,
        1 => SkillReferenceStrategy::Min,
        _ => SkillReferenceStrategy::Average,
    };
    ctx.base_score = 1.0 + f64::from(rng.range(0, 40)) / 100.0;
    ctx.base_score_auto = 0.8 + f64::from(rng.range(0, 30)) / 100.0;
    ctx.fever_score = f64::from(rng.range(0, 30)) / 100.0;
    for mode in &mut ctx.skill_scores {
        for rate in mode.iter_mut() {
            *rate = f64::from(rng.range(3, 25)) / 100.0;
        }
    }
    if ctx.live_type == LiveType::Multi && rng.range(0, 3) == 0 {
        ctx.multi_teammate_power = Some(8_000 + rng.range(0, 4_000) as i32);
        ctx.multi_teammate_score_up = Some(60 + rng.range(0, 60) as i32);
    }
    if rng.range(0, 6) == 0 {
        ctx.multi_live_score_up_lower_bound = Some(f64::from(rng.range(60, 140)));
    }
    ctx.honor_bonus = rng.range(0, 400);
    let first = CardIdx::new(rng.range(0, pool.count() as u32) as u16);
    let other_character = pool
        .indices()
        .map(|card| pool.char_id(card))
        .find(|&character| character != pool.char_id(first))
        .unwrap_or(pool.char_id(first));
    match scene {
        Scene::Additive => {}
        Scene::Limited(cap) => ctx.card_bonus_count_limit = cap,
        Scene::WorldBloom | Scene::Final(_) => {
            ctx.event_type = Some(EventType::WorldBloom);
            ctx.is_world_bloom = true;
            ctx.diff_attr_bonus = [0, 0, 1, 3, rng.range(0, 8) as u16, 2];
            ctx.support_deck = tier_support_deck(pool, rng);
            if let Scene::Final(cap) = scene {
                ctx.is_final_chapter = true;
                ctx.card_bonus_count_limit = cap;
                ctx.best_skill_as_leader = false;
                ctx.power_total_cap = (rng.range(0, 2) == 0).then(|| 7_000 + rng.range(0, 3_000));
                ctx.support_decks_by_character = vec![SupportDeck::default(); 32];
                for character in 1..=4usize {
                    if rng.range(0, 2) == 0 {
                        ctx.support_decks_by_character[character] = tier_support_deck(pool, rng);
                    }
                }
                for card in pool.indices() {
                    ctx.leader_honor_bonus_x10[card.raw()] = rng.range(0, 4) as u16 * 5;
                    ctx.leader_limit_bonus_x10[card.raw()] = rng.range(0, 3) as u16 * 10;
                }
            }
        }
        Scene::Challenge => {
            ctx.enforce_char_uniqueness = false;
            ctx.live_type = if rng.range(0, 2) == 0 {
                LiveType::Challenge
            } else {
                LiveType::ChallengeAuto
            };
        }
    }
    match constraint {
        Constraint::Free => {}
        Constraint::FixedCard => ctx.fixed_card_ids = vec![pool.game_id(first)],
        Constraint::FixedCharacter => ctx.fixed_character_ids = vec![pool.char_id(first)],
        Constraint::FixedCardAndCharacter => {
            ctx.fixed_card_ids = vec![pool.game_id(first)];
            ctx.fixed_character_ids = vec![other_character];
        }
        Constraint::ForcedLeader => ctx.forced_leader_character_id = Some(pool.char_id(first)),
    }
    ctx
}

fn tier_support_deck(pool: &CardPool, rng: &mut ExactLcg) -> SupportDeck {
    // Half-percent steps; steps that need finer ticks (0.35, 0.125); and a
    // step no tick scale represents, which keeps the outward rounding.
    let step = [0.5, 0.35, 0.125, 1.0 / 3.0][rng.range(0, 4) as usize];
    let mut cards = Vec::new();
    for card in pool.indices() {
        if rng.range(0, 3) == 0 {
            cards.push((pool.game_id(card), f64::from(rng.range(0, 7)) * step));
        }
    }
    cards.push((65_000, 1.5));
    cards.sort_unstable_by(|left, right| right.1.total_cmp(&left.1));
    cards.dedup_by_key(|entry| entry.0);
    // An occasional unordered list keeps the deck-level terms unbounded.
    if rng.range(0, 8) == 0 {
        cards.reverse();
    }
    SupportDeck {
        count: rng.range(1, 4) as u8,
        cards,
    }
}

fn check_tiers(pool: &CardPool, ctx: &SearchContext, targets: &[i32], top_k: usize, label: &str) {
    let params = SearchParams {
        top_k,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(pool, ctx).search_bonus_targets(&params, targets);
    // Attribute-limited views and tier certificates from the first node
    // exercise what production searches build only in large requests.
    for eager_bonus_tiers in [false, true] {
        let configuration = tuning::SearchTuning {
            eager_bonus_tiers,
            ..Default::default()
        };
        let (actual, stats) = tuning::with_tuning(configuration, || {
            search_bonus_targets(pool, ctx, &params, targets)
        });
        let label = format!("{label} eager_bonus_tiers={eager_bonus_tiers}");
        assert!(!stats.deadline_hit, "{label}: unexpected timeout");
        if actual != expected {
            for (name, rows) in [("solver", &actual), ("oracle", &expected)] {
                eprintln!("{label} {name}:");
                for row in rows.iter() {
                    eprintln!(
                        "  bonus={} live={} ids={:?} dense={:?}",
                        resolve_total_bonus(pool, ctx, &row.cards),
                        row.score as u32,
                        row.cards.map(|card| pool.game_id(card)),
                        row.cards.map(|card| card.raw()),
                    );
                }
            }
        }
        assert_eq!(actual, expected, "{label}");
    }
}

fn run_matrix(seed: u64, cases: u64, count: usize) -> u64 {
    let mut compared = 0;
    for case in 0..cases {
        let mut rng = ExactLcg(seed ^ case.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let scene = match case % 8 {
            0 | 1 => Scene::Additive,
            2 => Scene::Limited(1 + (case as usize / 8) % 4),
            3 => Scene::Limited((case as usize / 8) % 2 * 4),
            4 => Scene::WorldBloom,
            5 => Scene::Final([4, 2, 1, 5][(case as usize / 8) % 4]),
            6 => Scene::Challenge,
            _ => Scene::Limited(2),
        };
        let challenge = matches!(scene, Scene::Challenge);
        let shape = CardShape {
            count: if challenge { count - 1 } else { count },
            characters: if challenge {
                1 + (case / 8 % 2) as u8
            } else {
                5 + (case % 3) as u8
            },
            variants: (case % 3) as usize,
            step_x10: [10, 5, 50, 25][(case / 3 % 4) as usize],
            limited: !matches!(scene, Scene::Additive) || case % 2 == 0,
        };
        let cards = random_tier_cards(&mut rng, shape);
        let pool = tiered_pool(&cards);
        let constraint = if challenge {
            [Constraint::Free, Constraint::FixedCard][(case / 16 % 2) as usize]
        } else {
            [
                Constraint::Free,
                Constraint::FixedCard,
                Constraint::FixedCharacter,
                Constraint::FixedCardAndCharacter,
                Constraint::ForcedLeader,
                Constraint::Free,
            ][(case / 8 % 6) as usize]
        };
        let ctx = scene_context(&pool, &mut rng, scene, constraint);
        let mut targets = reachable_integer_tiers(&pool, &ctx, 3);
        // An unreachable tier, a duplicate and a negative tier.
        targets.push(targets.iter().copied().max().unwrap_or(0) + 7);
        if let Some(&first) = targets.first() {
            targets.push(first);
        }
        targets.push(-3);
        let top_k = [1, 3, 30][(case % 3) as usize];
        check_tiers(
            &pool,
            &ctx,
            &targets,
            top_k,
            &format!("case {case} {scene:?} {constraint:?} k={top_k} targets={targets:?}"),
        );
        compared += 1;
    }
    compared
}

#[test]
fn bonus_tiers_match_ordered_oracle_across_scenes() {
    let compared = run_matrix(0x7153_B0B0_2026_0924, 48, 8);
    assert_eq!(compared, 48);
}

#[test]
#[ignore = "exhaustive tier matrix; run with --release"]
fn long_bonus_tiers_match_ordered_oracle_across_scenes() {
    let mut compared = run_matrix(0x7153_B0B0_2026_0925, 1600, 10);
    compared += run_matrix(0x7153_B0B0_2026_0926, 800, 11);
    eprintln!("BONUS_TIER_MATRIX compared={compared}");
    assert_eq!(compared, 2400);
}

#[test]
fn bonus_tier_limited_counting_keeps_every_first_n_subset() {
    // Four positive limited amounts, cap two, one fixed limited card: the
    // fixed slot always counts first and the free cards fill the one
    // remaining place in every order.
    let mut rng = ExactLcg(0x11);
    let mut cards = random_tier_cards(
        &mut rng,
        CardShape {
            count: 7,
            characters: 7,
            variants: 0,
            step_x10: 10,
            limited: false,
        },
    );
    for (index, card) in cards.iter_mut().enumerate() {
        card.base_x10 = 10;
        card.limited_x10 = [30, 50, 70, 110, 0, 0, 0][index];
    }
    let pool = tiered_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::Marathon);
    ctx.card_bonus_count_limit = 2;
    ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
    let targets = (5..=25).collect::<Vec<_>>();
    for top_k in [1, 4] {
        check_tiers(&pool, &ctx, &targets, top_k, "fixed limited first");
    }
    ctx.fixed_card_ids.clear();
    for top_k in [1, 4] {
        check_tiers(&pool, &ctx, &targets, top_k, "free limited subsets");
    }
}

#[test]
fn bonus_tier_unreachable_and_empty_requests_return_nothing() {
    let mut rng = ExactLcg(0x22);
    let cards = random_tier_cards(
        &mut rng,
        CardShape {
            count: 8,
            characters: 6,
            variants: 1,
            step_x10: 50,
            limited: false,
        },
    );
    let pool = tiered_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::Marathon);
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };
    // Every card bonus is a multiple of 5%, so 1% and 10001% are unreachable.
    for targets in [vec![], vec![-1], vec![1], vec![10_001], vec![1, 10_001]] {
        let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &targets);
        assert!(!stats.deadline_hit);
        assert!(actual.is_empty(), "targets {targets:?}");
    }
}
