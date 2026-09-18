//! exact dominance contracts.
use super::*;

#[test]
fn search_dominance_preserves_best_score() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 400,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 30,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 30,
        },
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 401,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 20,
            },
            base_bonus: 5,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 20,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 402,
            power: 250,
            skill: SkillSlot {
                skill_type: 0,
                value: 25,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 250,
            skill_max: 25,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 403,
            power: 240,
            skill: SkillSlot {
                skill_type: 0,
                value: 24,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 240,
            skill_max: 24,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 404,
            power: 230,
            skill: SkillSlot {
                skill_type: 0,
                value: 23,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 230,
            skill_max: 23,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 405,
            power: 220,
            skill: SkillSlot {
                skill_type: 0,
                value: 22,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 220,
            skill_max: 22,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.leader_honor_bonus_x10 = vec![0; pool.count()];
    search_ctx.leader_limit_bonus_x10 = vec![0; pool.count()];
    search_ctx.skill_is_after_training = vec![false; pool.count()];
    search_ctx.trained_to_special_image = vec![false; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let before = dfs_search(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    let dominance = eliminate_dominated(&pool, &search_ctx);
    let compacted_suffix = SuffixBound::build(&dominance.pool, &dominance.ctx);
    let after = dfs_search(&dominance.pool, &dominance.ctx, &compacted_suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(before, after);
}

#[test]
fn search_top_k_recovers_dominated_alternatives() {
    // char0 的 B(295) 被 A(300) 支配裁掉，但 {B,1,2,3,4} 是全局第 2 名：
    // Top-K 替代展开应把它找回来，与暴力枚举一致（issue #2）。
    let cards = [
        dominance_pair_card(700, 0, 300),
        dominance_pair_card(701, 0, 295),
        dominance_pair_card(702, 1, 400),
        dominance_pair_card(703, 2, 410),
        dominance_pair_card(704, 3, 420),
        dominance_pair_card(705, 4, 430),
        dominance_pair_card(706, 5, 200),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    // 前提：B 确实被支配裁掉。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before - 1);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[1]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 701),
        "rank 1 should contain the dominated card 701",
    );
}

#[test]
fn search_top_k_recovers_multi_slot_dominated_alternatives() {
    // 两个角色各有一张被支配卡，第 4 名 {B,D,...} 需要同时回换两个槽位。
    let cards = [
        dominance_pair_card(800, 0, 300),
        dominance_pair_card(801, 0, 296),
        dominance_pair_card(802, 1, 400),
        dominance_pair_card(803, 1, 395),
        dominance_pair_card(804, 2, 500),
        dominance_pair_card(805, 3, 510),
        dominance_pair_card(806, 4, 520),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };

    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before - 2);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert_eq!(results.len(), 4);
    let last_game_ids = results[3].cards.map(|card| pool.game_id(card)).to_vec();
    assert!(
        last_game_ids.contains(&801) && last_game_ids.contains(&803),
        "rank 3 should substitute both dominated cards, got {last_game_ids:?}",
    );
}

#[test]
fn search_power_top_k_dedups_cultivation_variants() {
    // 同一 game_id 的两个养成变体不得挤占每角色候选/DP 状态名额：
    // 否则 701 进不了候选，含它的真实次优卡组从 Top-K 消失（issue #24）。
    let cards = [
        dominance_pair_card(700, 1, 300),
        dominance_pair_card(700, 1, 300),
        dominance_pair_card(701, 1, 295),
        dominance_pair_card(702, 2, 400),
        dominance_pair_card(703, 3, 410),
        dominance_pair_card(704, 4, 420),
        dominance_pair_card(705, 5, 430),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[1]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 701),
        "rank 1 must contain 701, not a duplicate variant of 700",
    );
}

#[test]
fn exact_top_k_ties_preserve_canonical_card_sets() {
    let cards = (0..8u16)
        .map(|idx| {
            let mut card = skill_card(30_000 + idx, idx as u8 + 1, 1_000, 50);
            card.attr = (idx % 5) as u8;
            card.base_bonus = 10;
            card
        })
        .collect::<Vec<_>>();
    let pool = build_pool(&cards);
    let params = SearchParams {
        top_k: 6,
        timeout_ms: 0,
    };

    for target in [ScoreTarget::Score, ScoreTarget::Power, ScoreTarget::Skill] {
        let mut search_ctx = ready_ctx(&pool, target);
        search_ctx.skill_scores[0] = [0.2; 6];
        let got = search(&pool, &search_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &search_ctx, &params);
        assert_property_results(&pool, &got, &expected, &format!("tie no-event {target:?}"));
    }

    // Force the Multi event SIMD candidate path.  Equal upper bounds at the
    // current kth score must remain live because card-set tie ordering still
    // decides which exact Top-K sets are returned.
    let mut event_ctx = ready_ctx(&pool, ScoreTarget::Score);
    event_ctx.live_type = LiveType::Multi;
    event_ctx.event_type = Some(EventType::Marathon);
    event_ctx.skill_scores[1] = [0.2; 6];
    let got = search(&pool, &event_ctx, &params);
    let (expected, _) = brute_force_search(&pool, &event_ctx, &params);
    assert_property_results(&pool, &got, &expected, "tie multi-event");
}
