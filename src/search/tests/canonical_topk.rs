//! Deterministic Top-K is a public-card-set contract, not traversal order.
use super::*;

fn equal_score_pool() -> CardPool {
    let mut cards = randomized_exact_cards(0xCA00, 6, 6);
    for (card, game_id) in cards.iter_mut().zip([60, 10, 50, 20, 40, 30]) {
        card.game_id = game_id;
        card.power = 1_000;
        card.power_max = 1_000;
        card.skill = SkillSlot {
            skill_type: 0,
            value: 100,
        };
        card.skill_max = 100;
        card.base_bonus = 0;
        card.limited_bonus = 0;
        card.attr = 0;
        card.unit_mask = 1;
    }
    build_pool(&cards)
}

const TIED_SETS: [[u16; 5]; 6] = [
    [10, 20, 30, 40, 50],
    [10, 20, 30, 40, 60],
    [10, 20, 30, 50, 60],
    [10, 20, 40, 50, 60],
    [10, 30, 40, 50, 60],
    [20, 30, 40, 50, 60],
];

#[test]
fn canonical_topk_public_sets_match_every_solver_family_and_limit() {
    let pool = equal_score_pool();
    for (target, minimize, final_chapter) in [
        (ScoreTarget::Score, false, false),
        (ScoreTarget::Power, false, false),
        (ScoreTarget::Power, true, false),
        (ScoreTarget::Skill, false, false),
        (ScoreTarget::Bonus, false, false),
        (ScoreTarget::Mysekai, false, false),
        (ScoreTarget::Score, false, true),
    ] {
        let mut ctx = ready_ctx(&pool, target);
        ctx.minimize = minimize;
        if target == ScoreTarget::Mysekai {
            ctx.live_type = LiveType::Mysekai;
        }
        if final_chapter {
            ctx.is_final_chapter = true;
            ctx.is_world_bloom = true;
            ctx.live_type = LiveType::Multi;
            ctx.live_skill_order = LiveSkillOrder::Average;
            ctx.event_type = Some(EventType::WorldBloom);
        }
        for top_k in [1, 2, 3, 6, 8, 30, 100] {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let (actual, stats) = search_instrumented(&pool, &ctx, &params);
            assert!(!stats.deadline_hit);
            let keys = actual
                .iter()
                .map(|deck| deck.game_card_set_key(&pool))
                .collect::<Vec<_>>();
            assert_eq!(
                keys,
                TIED_SETS[..top_k.min(TIED_SETS.len())],
                "target={target:?} minimize={minimize} final={final_chapter} k={top_k}"
            );
        }
    }
}

#[test]
fn canonical_topk_mysekai_ties_use_resolved_and_capped_power_not_upper_bounds() {
    let mut builder = PoolBuilder::new(6);
    for dense in 0..6u16 {
        let values = if dense == 0 {
            [1_000, 1_000, 1_000, 50_000, 1_000, 1_000, 1_000, 50_000]
        } else {
            [2_000; 8]
        };
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, 0);
        builder.set_power_max(dense, if dense == 0 { 50_000 } else { 2_000 });
        builder.set_game_id(dense, dense + 1);
        builder.set_char_id(dense, dense as u8 + 1);
        let attr = dense as u8 % 5;
        builder.set_attr(dense, attr);
        builder.set_unit_mask(dense, 1 << attr);
        builder.mark_char(dense as u8 + 1, dense);
        builder.mark_attr(attr, dense);
        builder.mark_unit(attr, dense);
    }
    let pool = builder.freeze();
    for cap in [None, Some(9_000)] {
        let mut ctx = ready_ctx(&pool, ScoreTarget::Mysekai);
        ctx.live_type = LiveType::Mysekai;
        ctx.power_total_cap = cap;
        for top_k in [1, 3, 6, 30] {
            let actual = search_exact(
                &pool,
                &ctx,
                &SearchParams {
                    top_k,
                    timeout_ms: 0,
                },
            );
            let expected = if cap.is_none() {
                [2, 3, 4, 5, 6]
            } else {
                [1, 2, 3, 4, 5]
            };
            assert_eq!(
                actual[0].game_card_set_key(&pool),
                expected,
                "cap={cap:?} k={top_k}"
            );
            let powers = actual
                .iter()
                .map(|deck| {
                    summarize_deck(&pool, &ctx, &deck.cards)
                        .unwrap()
                        .total_power
                })
                .collect::<Vec<_>>();
            assert!(powers.windows(2).all(|pair| pair[0] >= pair[1]));
        }
    }
}

#[test]
fn canonical_topk_dominance_must_preserve_tied_public_identity() {
    let mut cards = randomized_exact_cards(0xCA01, 6, 5);
    for (card, id) in cards.iter_mut().zip([900, 2, 3, 4, 5, 1]) {
        card.game_id = id;
        card.power = 1_000;
        card.power_max = 1_000;
        card.skill = SkillSlot {
            skill_type: 0,
            value: 100,
        };
        card.skill_max = 100;
        card.base_bonus = 0;
        card.limited_bonus = 0;
        card.attr = 0;
        card.unit_mask = 1;
    }
    cards[0].power = 2_000;
    cards[0].power_max = 2_000;
    let pool = build_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.base_score = 0.0;
    for top_k in [1, 2, 8, 30] {
        let actual = search_exact(
            &pool,
            &ctx,
            &SearchParams {
                top_k,
                timeout_ms: 0,
            },
        );
        assert_eq!(actual[0].score, 0);
        assert_eq!(
            actual[0].game_card_set_key(&pool),
            [1, 2, 3, 4, 5],
            "k={top_k}"
        );
    }
}

#[test]
fn canonical_topk_same_game_variants_share_one_set_and_a_stable_representative() {
    let mut cards = randomized_exact_cards(0xCA02, 6, 5);
    for (card, id) in cards.iter_mut().zip([1, 2, 3, 4, 5, 1]) {
        card.game_id = id;
        card.power = 1_000;
        card.power_max = 1_000;
        card.skill = SkillSlot {
            skill_type: 0,
            value: 100,
        };
        card.skill_max = 100;
        card.base_bonus = 0;
        card.limited_bonus = 0;
        card.attr = 0;
        card.unit_mask = 1;
    }
    cards[5].power = 2_000;
    cards[5].power_max = 2_000;
    let pool = build_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.base_score = 0.0;
    for top_k in [1, 2, 8, 30] {
        let actual = search_exact(
            &pool,
            &ctx,
            &SearchParams {
                top_k,
                timeout_ms: 0,
            },
        );
        assert_eq!(actual.len(), 1);
        assert!(
            actual[0].cards.contains(&CardIdx::new(0)),
            "k={top_k}: equal variants use their stable prepared-pool ordinal"
        );
        assert!(!actual[0].cards.contains(&CardIdx::new(5)));
    }
}

#[test]
fn canonical_legal_assignments_match_independent_oracle() {
    let pool = equal_score_pool();
    for target in [
        ScoreTarget::Power,
        ScoreTarget::Skill,
        ScoreTarget::Score,
        ScoreTarget::Mysekai,
    ] {
        for fixed in [false, true] {
            let mut ctx = ready_ctx(&pool, target);
            if target == ScoreTarget::Mysekai {
                ctx.live_type = LiveType::Mysekai;
            }
            if fixed {
                ctx.fixed_card_ids = vec![50];
                ctx.fixed_character_ids = vec![1];
            }
            for top_k in [1, 3, 30] {
                let params = SearchParams {
                    top_k,
                    timeout_ms: 0,
                };
                let actual = search_exact(&pool, &ctx, &params);
                let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
                assert_eq!(
                    actual, expected,
                    "target={target:?} fixed={fixed} k={top_k}"
                );
            }
        }
    }
}

#[test]
fn power_scenarios_include_every_representable_character_group() {
    let pool = build_pool(&five_unique_cards()); // includes character zero
    let ctx = ready_ctx(&pool, ScoreTarget::Power);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    assert_eq!(expected.len(), 1);
    assert_eq!(search_exact(&pool, &ctx, &params), expected);
}

#[test]
fn final_chapter_numeric_targets_choose_their_leader_like_the_oracle() {
    // Composition-dependent skills resolve below their maximum, so the
    // leader of a set is not fixed by the order its cards were picked in.
    let pool = build_special_exact_pool();
    for (target, minimize) in [
        (ScoreTarget::Skill, false),
        (ScoreTarget::Power, false),
        (ScoreTarget::Power, true),
    ] {
        for live_type in [LiveType::Solo, LiveType::Multi, LiveType::Auto] {
            for forced_leader in [None, Some(3)] {
                let mut ctx = ready_ctx(&pool, target);
                ctx.minimize = minimize;
                ctx.is_final_chapter = true;
                ctx.is_world_bloom = true;
                ctx.event_type = Some(EventType::WorldBloom);
                ctx.live_type = live_type;
                ctx.live_skill_order = LiveSkillOrder::Average;
                ctx.forced_leader_character_id = forced_leader;
                for top_k in [1, 5, 30] {
                    let params = SearchParams {
                        top_k,
                        timeout_ms: 0,
                    };
                    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
                    assert_eq!(
                        search_exact(&pool, &ctx, &params),
                        expected,
                        "target={target:?} minimize={minimize} live={live_type:?} \
                         leader={forced_leader:?} k={top_k}"
                    );
                }
            }
        }
    }
}

#[test]
fn final_chapter_skill_leader_is_not_the_first_picked_card() {
    // The unit-count card has the largest skill maximum but resolves to its
    // one-member entry here, so another member is the best leader.
    let mut cards = vec![TestCard {
        char_id: 1,
        attr: 0,
        unit_mask: 1 << 1,
        game_id: 101,
        power: 1_000,
        skill: SkillSlot {
            skill_type: 1,
            value: 1,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 1_000,
        skill_max: 50,
    }];
    for (offset, skill) in [40u8, 30, 20, 20].into_iter().enumerate() {
        let mut card = skill_card(102 + offset as u16, 2 + offset as u8, 1_000, skill);
        card.unit_mask = 1 << 2;
        cards.push(card);
    }
    let pool = build_pool(&cards);
    for live_type in [LiveType::Solo, LiveType::Multi, LiveType::Auto] {
        let mut ctx = ready_ctx(&pool, ScoreTarget::Skill);
        ctx.is_final_chapter = true;
        ctx.is_world_bloom = true;
        ctx.event_type = Some(EventType::WorldBloom);
        ctx.live_type = live_type;
        let params = SearchParams {
            top_k: 1,
            timeout_ms: 0,
        };
        let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
        let actual = search_exact(&pool, &ctx, &params);
        assert_eq!(actual, expected, "live={live_type:?}");
        assert_eq!(pool.game_id(actual[0].cards[0]), 102, "live={live_type:?}");
    }
}
