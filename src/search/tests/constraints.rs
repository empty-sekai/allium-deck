//! constraints contracts.
use super::*;

#[test]
fn search_fixed_card_constraint_is_respected() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 0,
        unit_mask: 1,
        game_id: 150,
        power: 1,
        skill: SkillSlot::default(),
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 1,
        skill_max: 0,
    });
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_card_ids = vec![150];
    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    assert_eq!(pool.game_id(results[0].cards[0]), 150);
}

#[test]
fn search_fixed_character_constraint_is_respected() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_character_ids = vec![1, 3];
    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    assert_eq!(pool.char_id(results[0].cards[0]), 1);
    assert_eq!(pool.char_id(results[0].cards[1]), 3);
}

#[test]
fn search_fixed_card_and_character_can_combine() {
    // 放开 fixed_cards ⊕ fixed_characters 互斥后：两者同时非空应被接受，
    // 引擎按「卡在前、角色在后」前缀填槽——槽0=固定卡(队长)，槽1=固定角色。
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_card_ids = vec![102]; // game_id 102 = char 2（见 five_unique_cards）
    search_ctx.fixed_character_ids = vec![4];
    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    // 槽0 = 固定卡 102（队长）
    assert_eq!(pool.game_id(results[0].cards[0]), 102);
    // 槽1 = 固定角色 4
    assert_eq!(pool.char_id(results[0].cards[1]), 4);
}

#[test]
fn forced_leader_character_takes_the_leader_slot() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    // 默认「最高技能作队长」会把 char 4 摆到队长位；指定队长必须压过它。
    search_ctx.best_skill_as_leader = true;
    search_ctx.forced_leader_character_id = Some(1);

    assert!(!search_ctx.effective_best_skill_as_leader());

    let deck = collect_first_five(&pool);
    let summary = summarize_deck(&pool, &search_ctx, &deck).expect("summary");
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 1);
}

#[test]
fn forced_leader_character_is_ignored_when_unset() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.best_skill_as_leader = true;

    let deck = collect_first_five(&pool);
    let summary = summarize_deck(&pool, &search_ctx, &deck).expect("summary");
    // 技能最高的是 char 4。
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 4);
}

#[test]
fn forced_leader_character_keeps_that_character_in_the_deck() {
    let pool = build_pool(&six_cards_with_weak_first());
    let unconstrained = search_exact(
        &pool,
        &ready_ctx(&pool, ScoreTarget::Power),
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(unconstrained.len(), 1);
    assert!(
        !unconstrained[0]
            .cards
            .iter()
            .any(|card| pool.char_id(*card) == 0),
        "最弱角色本不该出现在无约束最优解里",
    );

    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.forced_leader_character_id = Some(0);
    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );
    assert!(!results.is_empty());
    for result in &results {
        assert!(
            result.cards.iter().any(|card| pool.char_id(*card) == 0),
            "指定队长后每条结果都必须包含该角色",
        );
    }
}

#[test]
fn forced_leader_character_wins_over_a_fixed_card_in_another_slot() {
    // 固定一张别的角色的卡 + 指定队长：队长位归指定角色，固定卡退到其他槽位。
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.fixed_card_ids = vec![104]; // game_id 104 = char 4
    search_ctx.forced_leader_character_id = Some(1);

    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(results.len(), 1);
    let summary = summarize_deck(&pool, &search_ctx, &results[0].cards).expect("summary");
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 1);
    assert!(
        summary
            .ordered_cards
            .iter()
            .any(|card| pool.game_id(*card) == 104),
        "固定卡仍须留在队内",
    );
}

#[test]
fn forced_leader_search_matches_bruteforce_for_every_target() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };
    for target in [
        ScoreTarget::Score,
        ScoreTarget::Power,
        ScoreTarget::Skill,
        ScoreTarget::Bonus,
    ] {
        for leader in [0u8, 3, 6] {
            let mut search_ctx = ready_ctx(&pool, target);
            search_ctx.live_type = LiveType::Multi;
            search_ctx.event_type = Some(EventType::Marathon);
            search_ctx.forced_leader_character_id = Some(leader);

            let results = search_exact(&pool, &search_ctx, &params);
            let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
            assert_results_match_bruteforce(&pool, &results, &brute);
            assert!(
                !results.is_empty(),
                "target={target:?} leader={leader} 应有结果",
            );
            for result in &results {
                assert_eq!(
                    leader_character_of(&pool, &search_ctx, &result.cards),
                    leader,
                    "target={target:?} leader={leader} 队长位不对",
                );
            }
        }
    }
}

#[test]
fn forced_leader_holds_for_every_top_k_deck() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.forced_leader_character_id = Some(2);

    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 8,
            timeout_ms: 0,
        },
    );
    assert!(results.len() > 1);
    for result in &results {
        assert_eq!(leader_character_of(&pool, &search_ctx, &result.cards), 2);
        assert!(result.cards.iter().any(|card| pool.char_id(*card) == 2));
    }
}

#[test]
fn forced_leader_survives_minimize_and_fixed_characters() {
    let pool = build_pool(&eight_cards_for_leader_tests());

    // 反向搜索（最弱综合力）。
    let mut minimize_ctx = ready_ctx(&pool, ScoreTarget::Power);
    minimize_ctx.minimize = true;
    minimize_ctx.forced_leader_character_id = Some(7);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };
    let results = search_exact(&pool, &minimize_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &minimize_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    for result in &results {
        assert_eq!(leader_character_of(&pool, &minimize_ctx, &result.cards), 7);
    }

    // 与固定角色共存：两个约束都要满足，队长位归指定角色。
    let mut combined_ctx = ready_ctx(&pool, ScoreTarget::Score);
    combined_ctx.live_type = LiveType::Multi;
    combined_ctx.event_type = Some(EventType::Marathon);
    combined_ctx.fixed_character_ids = vec![1, 5];
    combined_ctx.forced_leader_character_id = Some(5);
    let results = search_exact(&pool, &combined_ctx, &params);
    assert!(!results.is_empty());
    for result in &results {
        assert!(result.cards.iter().any(|card| pool.char_id(*card) == 1));
        assert_eq!(leader_character_of(&pool, &combined_ctx, &result.cards), 5);
    }
}

#[test]
fn forced_leader_for_an_absent_character_returns_nothing() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.forced_leader_character_id = Some(20); // 池里没有的角色

    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );
    assert!(results.is_empty(), "队长角色不在池里时不得产出卡组");
}

#[test]
fn forced_leader_does_not_change_results_when_it_matches_the_natural_leader() {
    // 指定的队长恰好就是默认（最高技能）队长时，结果集必须与不指定完全一致。
    let pool = build_pool(&eight_cards_for_leader_tests());
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };
    let mut free_ctx = ready_ctx(&pool, ScoreTarget::Score);
    free_ctx.live_type = LiveType::Multi;
    free_ctx.event_type = Some(EventType::Marathon);
    let free = search_exact(&pool, &free_ctx, &params);
    assert!(!free.is_empty());
    let natural_leader = leader_character_of(&pool, &free_ctx, &free[0].cards);

    let mut forced_ctx = free_ctx.clone();
    forced_ctx.forced_leader_character_id = Some(natural_leader);
    let forced = search_exact(&pool, &forced_ctx, &params);

    assert_eq!(forced[0].score, free[0].score);
    assert_eq!(
        forced[0].game_card_set_key(&pool),
        free[0].game_card_set_key(&pool),
    );
}

#[test]
fn search_top_k_dominated_alternatives_respect_fixed_slots() {
    // 固定卡槽位不参与回换：固定 game_id 800 时，被支配卡 801 不得顶掉它。
    let cards = [
        dominance_pair_card(800, 0, 300),
        dominance_pair_card(801, 0, 296),
        dominance_pair_card(802, 1, 400),
        dominance_pair_card(803, 2, 500),
        dominance_pair_card(804, 3, 510),
        dominance_pair_card(805, 4, 520),
        dominance_pair_card(806, 5, 210),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.fixed_card_ids = vec![800];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search_exact(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    for result in &results {
        assert_eq!(pool.game_id(result.cards[0]), 800);
        assert!(result.cards.iter().all(|card| pool.game_id(*card) != 801));
    }
}

#[test]
fn simple_target_search_preserves_large_fixed_character_prefixes() {
    for fixed_candidates in [65u16, 508] {
        let mut cards = Vec::new();
        for candidate in 0..fixed_candidates {
            cards.push(skill_card(
                1000 + candidate,
                1,
                100 + u32::from(candidate),
                (candidate % 150 + 1) as u8,
            ));
        }
        for char_id in 2..=5u8 {
            cards.push(skill_card(
                2000 + u16::from(char_id),
                char_id,
                1000 + u32::from(char_id),
                160 + char_id,
            ));
        }
        let pool = build_pool(&cards);
        assert_eq!(pool.count(), usize::from(fixed_candidates) + 4);
        for target in [ScoreTarget::Power, ScoreTarget::Skill] {
            let mut search_ctx = ready_ctx(&pool, target);
            search_ctx.fixed_character_ids = vec![1];
            let params = SearchParams {
                top_k: 3,
                timeout_ms: 0,
            };
            let actual = search_exact(&pool, &search_ctx, &params);
            let (expected, _) = brute_force_search(&pool, &search_ctx, &params);
            assert_eq!(actual.len(), params.top_k);
            assert_results_match_bruteforce(&pool, &actual, &expected);
            for result in actual {
                assert_eq!(pool.char_id(result.cards[0]), 1);
                let mut characters = result.cards.map(|card| pool.char_id(card));
                characters.sort_unstable();
                assert_eq!(characters, [1, 2, 3, 4, 5]);
            }
        }
    }
}

#[test]
fn fully_fixed_lineup_keeps_summary_slots_and_metrics() {
    for live_type in [LiveType::Solo, LiveType::Challenge] {
        let mut cards = five_unique_cards();
        if live_type == LiveType::Challenge {
            for card in &mut cards {
                card.char_id = 1;
            }
        }
        let pool = build_pool(&cards);
        let mut context = ready_ctx(&pool, ScoreTarget::Power);
        context.live_type = live_type;
        context.enforce_char_uniqueness = live_type != LiveType::Challenge;
        context.best_skill_as_leader = false;
        context.fixed_card_ids = vec![104, 103, 102, 101, 100];
        let result = search_exact(
            &pool,
            &context,
            &SearchParams {
                top_k: 1,
                timeout_ms: 0,
            },
        );
        assert_eq!(result.len(), 1);
        let summary = summarize_deck(&pool, &context, &result[0].cards).unwrap();
        assert_eq!(
            summary.ordered_cards.map(|c| pool.game_id(c)),
            [104, 103, 102, 101, 100]
        );
        assert_eq!(summary.card_skill_score_up, [50.0, 40.0, 30.0, 20.0, 10.0]);
        assert_eq!(summary.card_power_total, [500, 400, 300, 200, 100]);
    }
}

#[test]
fn fully_fixed_lineup_moves_forced_leader_to_front_in_order() {
    let cards = five_unique_cards();
    let pool = build_pool(&cards);
    let mut context = ready_ctx(&pool, ScoreTarget::Power);
    context.best_skill_as_leader = false;
    context.fixed_card_ids = vec![104, 103, 102, 101, 100];
    context.forced_leader_character_id = Some(pool.char_id(CardIdx::new(1)));
    let result = search_exact(
        &pool,
        &context,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(result.len(), 1);
    let summary = summarize_deck(&pool, &context, &result[0].cards).unwrap();
    assert_eq!(
        summary.ordered_cards.map(|c| pool.game_id(c)),
        [101, 104, 103, 102, 100]
    );
}
