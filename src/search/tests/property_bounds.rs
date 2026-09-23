//! property bounds contracts.
use super::*;

#[test]
fn search_suffix_bound_is_sound_and_zero_pool_is_zero() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 1,
        unit_mask: 1,
        game_id: 105,
        power: 50,
        skill: SkillSlot {
            skill_type: 0,
            value: 5,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 50,
        skill_max: 5,
    });
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Score);
    let suffix = SuffixBound::build(&pool, &search_ctx);

    let selected = pool.card_idx(0).unwrap_or(crate::pool::CardIdx::new(0));
    let mut used = UsedSet::new();
    used.insert(pool.char_id(selected));
    let partial = PartialDeck {
        power: pool.power_max(selected),
        skill: pool.skill_max(selected) as u32,
        bonus: pool.event_bonus_exact(selected).base_ceil(),
        max_skill: pool.skill_max(selected),
        limited_count: 0,
    };

    let upper = suffix.upper_bound_with_depth(1, &used, &partial);
    let mut best_real = 0u64;
    let mut i = 1usize;
    while i < pool.count() {
        let c1 = crate::pool::CardIdx::new(i as u16);
        let mut j = i + 1;
        while j < pool.count() {
            let c2 = crate::pool::CardIdx::new(j as u16);
            let mut k = j + 1;
            while k < pool.count() {
                let c3 = crate::pool::CardIdx::new(k as u16);
                let mut l = k + 1;
                while l < pool.count() {
                    let c4 = crate::pool::CardIdx::new(l as u16);
                    let deck = [selected, c1, c2, c3, c4];
                    let score = leaf_evaluate(&pool, &search_ctx, &deck);
                    if score > best_real {
                        best_real = score;
                    }
                    l += 1;
                }
                k += 1;
            }
            j += 1;
        }
        i += 1;
    }
    assert!(upper >= best_real);

    let zero_cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 200,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 201,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 202,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 203,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 204,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
    ];
    let zero_pool = build_pool(&zero_cards);
    let zero_suffix = SuffixBound::build(&zero_pool, &ctx(ScoreTarget::Power));
    assert_eq!(
        zero_suffix.upper_bound_with_depth(0, &UsedSet::new(), &PartialDeck::default()),
        0
    );
}

#[test]
fn fractional_average_ceiling_bounds_the_leaf_and_complete_ordered_top_k() {
    let cards: Vec<_> = (0..7u8)
        .map(|index| TestCard {
            char_id: index + 1,
            attr: index % 5,
            unit_mask: 1,
            game_id: 100 + u16::from(index),
            power: 100_000,
            skill: SkillSlot {
                skill_type: 0,
                value: u8::from(index == 0),
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100_000,
            skill_max: u8::from(index == 0),
        })
        .collect();
    let pool = build_pool(&cards);
    let deck = core::array::from_fn(|index| CardIdx::new(index as u16));
    // Both divisions used to drop 0.75 of a one-millionth rate. At the
    // 336000 power cap this costs 1.008 live points: the old "ceiling" was
    // 1344000 while the actual leaf was 1344001, with the same EP=167.
    let rates = [
        [0.000375, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.000075],
    ];
    for live_type in [LiveType::Solo, LiveType::Auto] {
        for final_chapter in [false, true] {
            for power_cap in [None, Some(336_000)] {
                for forced_leader in [None, Some(1)] {
                    for rate in rates {
                        let mut context = ready_ctx(&pool, ScoreTarget::Score);
                        context.live_type = live_type;
                        context.event_type = Some(EventType::WorldBloom);
                        context.is_world_bloom = true;
                        context.is_final_chapter = final_chapter;
                        context.forced_leader_character_id = forced_leader;
                        context.base_score = 1.0;
                        context.base_score_auto = 1.0;
                        context.fever_score = 0.0;
                        context.live_skill_order = LiveSkillOrder::Average;
                        context.power_total_cap = power_cap;
                        context.skill_scores = [[0.0; 6]; 3];
                        context.skill_scores[if live_type == LiveType::Auto { 2 } else { 0 }] =
                            rate;
                        let actual = evaluate::leaf_evaluate_checked(&pool, &context, &deck)
                            .expect("five distinct characters form a legal leaf");
                        let suffix = SuffixBound::build(&pool, &context);
                        let upper = suffix.ceiling(500_000, 0, 1, 1);
                        assert!(upper >= actual, "ceiling={upper} leaf={actual}");
                        if power_cap.is_some() {
                            assert_eq!(actual, (167u64 << 32) | 1_344_001);
                        }
                        for top_k in [1, 8, 100] {
                            let params = SearchParams {
                                top_k,
                                timeout_ms: 0,
                            };
                            let outcome = search(&pool, &context, &params);
                            assert_eq!(outcome.completion(), SearchCompletion::Complete);
                            let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
                            // DeckResult equality checks complete length, rank,
                            // exact score, every ordered slot and dense variant.
                            assert_eq!(
                                outcome.results, expected,
                                "live={live_type:?} final={final_chapter} cap={power_cap:?} leader={forced_leader:?} rates={rate:?} K={top_k}"
                            );
                        }
                    }
                }
            }
        }
    }
}
