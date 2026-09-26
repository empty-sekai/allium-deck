//! Permanent counterexamples from the 2026-09-26 proof audit.
//! The oracle shares the leaf evaluator, but not pruning, placement or tracking.
use super::*;

fn assert_complete_oracle(pool: &CardPool, context: &SearchContext, k: usize) {
    let params = SearchParams {
        top_k: k,
        timeout_ms: 0,
    };
    let actual = search(pool, context, &params);
    let (expected, _) = ExactOracle::new(pool, context).search(&params);
    assert_eq!(actual.completion(), SearchCompletion::Complete);
    assert_eq!(
        actual.results, expected,
        "target={:?} k={k}",
        context.target
    );
}

#[test]
fn maximum_public_id_participates_in_canonical_top_k() {
    // Before the fix, rank 20 had the right score but the wrong public set:
    // [101,103,104,106,65535], instead of [101,102,104,107,65535].
    let rows = [
        (101, 107),
        (106, 107),
        (104, 105),
        (102, 109),
        (107, 108),
        (105, 101),
        (103, 110),
        (65535, 105),
    ];
    let cards: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(i, &(id, power))| {
            let mut card = skill_card(id, i as u8 + 1, power, (power - 80) as u8);
            card.attr = 1;
            card.unit_mask = 1;
            card
        })
        .collect();
    let pool = build_pool(&cards);
    for target in [ScoreTarget::Power, ScoreTarget::Skill, ScoreTarget::Mysekai] {
        let mut context = ready_ctx(&pool, target);
        if target == ScoreTarget::Mysekai {
            context.live_type = LiveType::Mysekai;
        }
        for k in [1, 3, 8, 20, 40, 56] {
            assert_complete_oracle(&pool, &context, k);
        }
        if target == ScoreTarget::Power {
            // Also reach the generic numeric solver, in both directions.
            context.power_total_cap = Some(10_000);
            for minimize in [false, true] {
                context.minimize = minimize;
                for k in [1, 20, 56] {
                    assert_complete_oracle(&pool, &context, k);
                }
            }
        }
    }
}

#[test]
fn maximum_public_id_randomized_ties_match_ordered_oracle() {
    let mut state = 0xA110_2026_0926_u64;
    for _case in 0..24 {
        let cards: Vec<_> = (0..8_u16)
            .map(|i| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let power = 100 + ((state >> 32) % 12) as u32;
                let id = if i == 7 {
                    u16::MAX
                } else {
                    101 + ((i * 5) % 7)
                };
                let mut card = skill_card(id, i as u8 + 1, power, (power - 80) as u8);
                card.attr = 1;
                card.unit_mask = 1;
                card
            })
            .collect();
        let pool = build_pool(&cards);
        for target in [ScoreTarget::Power, ScoreTarget::Skill] {
            let context = ready_ctx(&pool, target);
            for k in [1, 8, 20, 56] {
                assert_complete_oracle(&pool, &context, k);
            }
        }
    }
}

fn cancellation_pool(base_a_x10: u16) -> CardPool {
    let ids = [100_u16, 200, 1001, 1006, 1009, 1011];
    let mut builder = PoolBuilder::new(ids.len() as u16);
    for (i, &id) in ids.iter().enumerate() {
        let dense = i as u16;
        let character = if i < 2 { 1 } else { i as u8 };
        let (values, lut) = encode_power(90_000);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        builder.set_power_max(dense, 90_000);
        builder.set_skill(
            dense,
            SkillSlot {
                skill_type: 0,
                value: 100,
            },
        );
        builder.set_skill_min(dense, 100);
        builder.set_skill_max(dense, 100);
        builder.set_skill_reference(dense, 100);
        builder.set_event_bonus(
            dense,
            EventBonusExact::from_x10(
                if i == 0 {
                    base_a_x10
                } else if i == 1 {
                    625
                } else {
                    0
                },
                0,
            ),
        );
        builder.set_game_id(dense, id);
        builder.set_char_id(dense, character);
        builder.set_attr(dense, 1);
        builder.set_unit_mask(dense, 1);
        builder.mark_char(character, dense);
        builder.mark_attr(1, dense);
        builder.mark_unit(0, dense);
    }
    builder.freeze()
}

fn cancellation_context(pool: &CardPool) -> SearchContext {
    let mut context = ready_ctx(pool, ScoreTarget::Mysekai);
    context.live_type = LiveType::Mysekai;
    context.is_world_bloom = true;
    context.is_final_chapter = true;
    context.event_type = Some(EventType::WorldBloom);
    context.support_deck = SupportDeck {
        cards: vec![
            (100, 7.05),
            (200, 5.45),
            (300, 2.05),
            (301, 0.8),
            (302, 0.5),
        ],
        count: 4,
    };
    context.extra_bonus_ub = 16;
    context.forced_leader_character_id = Some(2);
    context.fixed_character_ids = vec![2];
    context.support_decks_by_character = vec![context.support_deck.clone(); 27];
    context
}

#[test]
fn support_compensation_preserves_actual_floating_point_top_one() {
    let pool = cancellation_pool(641);
    let context = cancellation_context(&pool);
    let deck_a = [
        CardIdx::new(2),
        CardIdx::new(0),
        CardIdx::new(3),
        CardIdx::new(4),
        CardIdx::new(5),
    ];
    let deck_b = [
        CardIdx::new(2),
        CardIdx::new(1),
        CardIdx::new(3),
        CardIdx::new(4),
        CardIdx::new(5),
    ];
    // Equal over decimal arithmetic, unequal under the unchanged evaluator.
    assert_eq!(leaf_evaluate(&pool, &context, &deck_a), 1728);
    assert_eq!(leaf_evaluate(&pool, &context, &deck_b), 1729);
    let dominance = eliminate_dominated(&pool, &context);
    assert_eq!(dominance.after, dominance.before);
    for k in [1, 2, 8] {
        assert_complete_oracle(&pool, &context, k);
    }
    for target in [ScoreTarget::Score, ScoreTarget::Bonus, ScoreTarget::Mysekai] {
        for final_chapter in [false, true] {
            let mut other = context.clone();
            other.target = target;
            other.is_final_chapter = final_chapter;
            other.live_type = if target == ScoreTarget::Mysekai {
                LiveType::Mysekai
            } else {
                LiveType::Multi
            };
            other.skill_scores[1] = [0.2; 6];
            for k in [1, 2] {
                assert_complete_oracle(&pool, &other, k);
            }
        }
    }
}

#[test]
fn strict_support_surplus_still_allows_certified_dominance() {
    let pool = cancellation_pool(642);
    let context = cancellation_context(&pool);
    let dominance = eliminate_dominated(&pool, &context);
    assert_eq!(dominance.after, dominance.before - 1);
    for k in [1, 2] {
        assert_complete_oracle(&pool, &context, k);
    }
}

#[test]
fn malformed_support_profiles_do_not_certify_dominance() {
    let pool = cancellation_pool(642);
    for malformed in [
        vec![(100, 7.05), (100, 5.45)],
        vec![(100, 1.0), (200, 2.0)],
        vec![(100, f64::NAN)],
        vec![(100, -1.0)],
    ] {
        let mut context = cancellation_context(&pool);
        context.support_deck.cards = malformed;
        context.support_decks_by_character = vec![context.support_deck.clone(); 27];
        let dominance = eliminate_dominated(&pool, &context);
        assert_eq!(dominance.after, dominance.before);
    }
}

#[test]
fn event_upper_bound_remains_monotone_outside_the_legal_leaf_range() {
    use crate::search::objective::ObjectiveBound;
    let pool = cancellation_pool(641);
    let mut context = ready_ctx(&pool, ScoreTarget::Score);
    context.event_type = Some(EventType::Marathon);
    context.music_rate_pct = 2_000_000;
    for live_type in [LiveType::Solo, LiveType::Multi, LiveType::Cheerful] {
        context.live_type = live_type;
        let objective = ObjectiveBound::from_context(&context);
        let mut previous = 0;
        for live in [0, 1_800_000, 100_000_000, 1_000_000_000, i32::MAX] {
            let current = objective.calc_event_point_bound(live, 0);
            assert!(
                current >= previous,
                "{live_type:?}: {live} => {current} < {previous}"
            );
            previous = current;
        }
        assert_eq!(previous, i32::MAX);
    }
}

#[test]
fn unrepresentable_joint_coefficients_disable_only_the_optional_bound() {
    use crate::search::objective::{LiveProduct, ObjectiveBound};
    let pool = cancellation_pool(641);
    let mut context = ready_ctx(&pool, ScoreTarget::Score);
    context.live_type = LiveType::Multi;
    context.skill_scores[1] = [1e15; 6];
    assert!(
        ObjectiveBound::from_context(&context)
            .live_product()
            .is_none()
    );
    let product = LiveProduct {
        honor: 0,
        intercept: 1,
        leader: 1,
        skill: 1,
        constant: i64::MAX,
        divisor: 1,
        floor: 0,
    };
    assert_eq!(product.live(i128::MAX, 2), u32::MAX);
}
