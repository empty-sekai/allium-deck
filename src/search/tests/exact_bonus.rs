//! exact bonus contracts.
use super::*;

#[test]
fn search_power_scenarios_matches_bruteforce_with_unit_and_attr_bonuses() {
    let mut builder = PoolBuilder::new(8);
    for dense in 0..8u16 {
        let grouped = dense < 5;
        let values = if grouped {
            [100, 200, 300, 1_000, 100, 200, 300, 1_000]
        } else {
            [500; 8]
        };
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, 0);
        builder.set_char_id(dense, (dense + 1) as u8);
        builder.set_attr(dense, if grouped { 1 } else { (dense - 4) as u8 });
        builder.set_unit_mask(dense, if grouped { 1 << 1 } else { 1 << (dense - 4) });
        builder.set_game_id(dense, dense + 100);
        builder.set_power_max(dense, if grouped { 1_000 } else { 500 });
        builder.mark_char((dense + 1) as u8, dense);
        builder.mark_attr(if grouped { 1 } else { (dense - 4) as u8 }, dense);
        builder.mark_unit(if grouped { 1 } else { (dense - 4) as u8 }, dense);
    }
    let pool = builder.freeze();
    let search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    let expected = brute_force_best(&pool, &search_ctx);
    let actual = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );

    assert_eq!(actual.first().map(|result| result.score), Some(expected));
    assert_eq!(expected, 5_000);
}

#[test]
fn search_dfs_bonus_noevent_matches_bruteforce_with_suffix_max_break() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 520,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 1,
            unit_mask: 1,
            game_id: 521,
            power: 120,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 5,
            limited_bonus: 0,
            power_max: 120,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 2,
            unit_mask: 1,
            game_id: 522,
            power: 90,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 40,
            limited_bonus: 0,
            power_max: 90,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 3,
            unit_mask: 1,
            game_id: 523,
            power: 110,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 110,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 4,
            unit_mask: 1,
            game_id: 524,
            power: 95,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 35,
            limited_bonus: 0,
            power_max: 95,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 525,
            power: 105,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 15,
            limited_bonus: 0,
            power_max: 105,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Score);
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let best = dfs_search_exact(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
}

#[test]
fn search_bonus_targets_matches_single_pass_bruteforce_buckets() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 600,
            power: 100,
            skill: SkillSlot::default(),
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 1,
            unit_mask: 1,
            game_id: 601,
            power: 110,
            skill: SkillSlot::default(),
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 110,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 2,
            unit_mask: 1,
            game_id: 602,
            power: 120,
            skill: SkillSlot::default(),
            base_bonus: 30,
            limited_bonus: 0,
            power_max: 120,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 3,
            unit_mask: 1,
            game_id: 603,
            power: 130,
            skill: SkillSlot::default(),
            base_bonus: 40,
            limited_bonus: 0,
            power_max: 130,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 4,
            unit_mask: 1,
            game_id: 604,
            power: 140,
            skill: SkillSlot::default(),
            base_bonus: 50,
            limited_bonus: 0,
            power_max: 140,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 605,
            power: 150,
            skill: SkillSlot::default(),
            base_bonus: 60,
            limited_bonus: 0,
            power_max: 150,
            skill_max: 0,
        },
        TestCard {
            char_id: 6,
            attr: 1,
            unit_mask: 1,
            game_id: 606,
            power: 160,
            skill: SkillSlot::default(),
            base_bonus: 70,
            limited_bonus: 0,
            power_max: 160,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    search_ctx.event_type = Some(crate::types::EventType::Marathon);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };
    let targets = [150, 250];

    let default_actual = search_exact(&pool, &search_ctx, &params);
    let (default_expected, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_eq!(default_actual, default_expected);

    let actual = search_bonus_targets(&pool, &search_ctx, &params, &targets).0;
    let brute_params = SearchParams {
        top_k: 100,
        timeout_ms: 0,
    };
    let (all, _) = brute_force_search(&pool, &search_ctx, &brute_params);
    let expected = targets
        .iter()
        .rev()
        .flat_map(|target| {
            all.iter()
                .filter(move |result| (result.score >> 32) == (*target as u64 * 2))
                .take(params.top_k)
                .copied()
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn bonus_bucket_live_threshold_uses_low_score_key() {
    // Regression from the all-scene matrix (case 24): the 155% bucket fills
    // before its best-live-score deck is visited.  The historical pruning code
    // compared the live-score upper bound with the entire encoded
    // (bonus_x2 << 32 | live_score) threshold and incorrectly closed the bucket.
    let cards = randomized_exact_cards(0xE7AC_0000 + 24, 12, 7);
    let pool = build_pool(&cards);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let mut bonus = ready_ctx(&pool, ScoreTarget::Bonus);
    bonus.event_type = Some(EventType::Marathon);

    let all_params = SearchParams {
        top_k: 1000,
        timeout_ms: 0,
    };
    let (all_bonus, _) = brute_force_search(&pool, &bonus, &all_params);
    let target = all_bonus
        .iter()
        .map(|result| result.score >> 32)
        .find(|encoded| encoded % 2 == 0)
        .map(|encoded| (encoded / 2) as i32)
        .expect("at least one integer bonus tier");

    let (got, stats) = search_bonus_targets(&pool, &bonus, &params, &[target]);
    assert!(!stats.deadline_hit);
    let expected = all_bonus
        .iter()
        .filter(|result| (result.score >> 32) == target as u64 * 2)
        .take(1)
        .copied()
        .collect::<Vec<_>>();
    assert_property_scores(
        &pool,
        &bonus,
        &got,
        &expected,
        "bonus live-threshold regression",
    );
}
