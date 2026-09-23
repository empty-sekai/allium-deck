//! exact score contracts.
use super::*;
use crate::search::objective::ObjectiveBound;

#[test]
fn search_leaf_evaluate_encodes_targets() {
    let pool = build_pool(&five_unique_cards());
    let deck = collect_first_five(&pool);

    let power_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Power), &deck);
    assert_eq!(power_value, 1500);

    let skill_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Skill), &deck);
    assert_eq!(skill_value, 700);

    let score_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Score), &deck);
    assert_eq!(score_value, ((6000u64) << 32) | 6000u64);
}

#[test]
fn score_noevent_live_ceiling_is_identical_to_the_packed_score_order() {
    let pool = build_pool(&five_unique_cards());
    let search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    assert!(!search_ctx.has_event());
    let suffix = SuffixBound::build(&pool, &search_ctx);

    for power in [0, 100, 1_500, 50_000, 500_000] {
        for skill in [0, 25, 100, 500] {
            for leader in [0, 30, 120, 500] {
                let numerator = suffix
                    .objective()
                    .score_noevent_live_numerator_ceiling(power, skill, leader);
                let live = suffix
                    .objective()
                    .score_noevent_live_ceiling(power, skill, leader);
                let packed = suffix.objective().ceiling(power, 0, skill, leader);
                assert_eq!(
                    numerator / 1_000_000,
                    live as i64,
                    "pre-division bound must preserve floor semantics"
                );
                assert_eq!(packed, ((live as u64) << 32) | live as u64);
                assert!(numerator >= ObjectiveBound::score_noevent_threshold_numerator(live));
                assert!(
                    numerator
                        < ObjectiveBound::score_noevent_threshold_numerator(live.saturating_add(1))
                );
            }
        }
    }
}

#[test]
fn search_leaf_evaluate_score_path_consumes_music_skill_tables() {
    let pool = build_pool(&five_unique_cards());
    let deck = collect_first_five(&pool);
    let mut search_ctx = ctx(ScoreTarget::Score);
    search_ctx.skill_scores[0] = [10.0; 6];

    let score_value = leaf_evaluate(&pool, &search_ctx, &deck);
    assert_eq!(score_value, ((126000u64) << 32) | 126000u64);
}

#[test]
fn search_multi_score_up_lower_bound_filters_invalid_decks() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.multi_live_score_up_lower_bound = Some(1_000.0);
    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert!(results.is_empty());
}

#[test]
fn search_dfs_matches_bruteforce_for_best_deck() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 300,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 301,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 302,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 303,
            power: 400,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 400,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 304,
            power: 500,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 500,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 305,
            power: 50,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 50,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.leader_honor_bonus_x10 = vec![0; pool.count()];
    search_ctx.leader_limit_bonus_x10 = vec![0; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let results = dfs_search_exact(&pool, &search_ctx, &suffix, &params);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let mut brute = 0u64;
    let mut a = 0usize;
    while a < pool.count() {
        let c0 = crate::pool::CardIdx::new(a as u16);
        let mut b = a + 1;
        while b < pool.count() {
            let c1 = crate::pool::CardIdx::new(b as u16);
            let mut c = b + 1;
            while c < pool.count() {
                let c2 = crate::pool::CardIdx::new(c as u16);
                let mut d = c + 1;
                while d < pool.count() {
                    let c3 = crate::pool::CardIdx::new(d as u16);
                    let mut e = d + 1;
                    while e < pool.count() {
                        let c4 = crate::pool::CardIdx::new(e as u16);
                        let score = leaf_evaluate(&pool, &search_ctx, &[c0, c1, c2, c3, c4]);
                        if score > brute {
                            brute = score;
                        }
                        e += 1;
                    }
                    d += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }

    assert_eq!(best, brute);
}

#[test]
fn search_dfs_score_noevent_does_not_break_before_higher_skill_same_power_state() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 500,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 10,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 10,
        },
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 500,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 20,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 20,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 501,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 502,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 503,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 504,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.skill_scores[1] = [10.0; 6];
    search_ctx.leader_honor_bonus_x10 = vec![0; pool.count()];
    search_ctx.leader_limit_bonus_x10 = vec![0; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let results = dfs_search_exact(&pool, &search_ctx, &suffix, &params);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let lower = leaf_evaluate(
        &pool,
        &search_ctx,
        &[
            crate::pool::CardIdx::new(0),
            crate::pool::CardIdx::new(2),
            crate::pool::CardIdx::new(3),
            crate::pool::CardIdx::new(4),
            crate::pool::CardIdx::new(5),
        ],
    );
    let higher = leaf_evaluate(
        &pool,
        &search_ctx,
        &[
            crate::pool::CardIdx::new(1),
            crate::pool::CardIdx::new(2),
            crate::pool::CardIdx::new(3),
            crate::pool::CardIdx::new(4),
            crate::pool::CardIdx::new(5),
        ],
    );

    assert!(higher > lower);
    assert_eq!(best, higher);
}

#[test]
fn search_dfs_score_noevent_matches_bruteforce_with_monotonic_break() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 600,
            power: 220,
            skill: SkillSlot {
                skill_type: 0,
                value: 30,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 220,
            skill_max: 30,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 601,
            power: 210,
            skill: SkillSlot {
                skill_type: 0,
                value: 28,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 210,
            skill_max: 28,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 602,
            power: 205,
            skill: SkillSlot {
                skill_type: 0,
                value: 25,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 205,
            skill_max: 25,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 603,
            power: 190,
            skill: SkillSlot {
                skill_type: 0,
                value: 24,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 190,
            skill_max: 24,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 604,
            power: 180,
            skill: SkillSlot {
                skill_type: 0,
                value: 22,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 180,
            skill_max: 22,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 605,
            power: 80,
            skill: SkillSlot {
                skill_type: 0,
                value: 3,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 80,
            skill_max: 3,
        },
        TestCard {
            char_id: 6,
            attr: 0,
            unit_mask: 1,
            game_id: 606,
            power: 70,
            skill: SkillSlot {
                skill_type: 0,
                value: 2,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 70,
            skill_max: 2,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.skill_scores[1] = [10.0; 6];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let seed = warm_start::warm_start_best(&pool, &search_ctx);
    let (results, stats) = dfs::dfs_search_instrumented(&pool, &search_ctx, &suffix, &params, seed);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
    let _ = stats;
}

#[test]
fn search_dfs_core_matches_bruteforce_for_three_cards_choose_two() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 360,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 361,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 362,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let power_ctx = ctx(ScoreTarget::Power);
    let suffix = SuffixBound::build(&pool, &power_ctx);
    let results = dfs::dfs_search_power_len_for_test(&pool, &suffix, 2, 1, &power_ctx);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let mut brute = 0u64;
    let mut left = 0usize;
    while left < pool.count() {
        let mut right = left + 1;
        while right < pool.count() {
            let score = pool.power_max(crate::pool::CardIdx::new(left as u16)) as u64
                + pool.power_max(crate::pool::CardIdx::new(right as u16)) as u64;
            if score > brute {
                brute = score;
            }
            right += 1;
        }
        left += 1;
    }

    assert_eq!(best, brute);
}

#[test]
fn search_warm_start_returns_non_zero_incumbent() {
    let pool = build_pool(&five_unique_cards());
    assert!(warm_start(&pool, &ctx(ScoreTarget::Power)) > 0);
}
