//! exact world bloom contracts.
use super::*;

#[test]
fn search_world_bloom_support_penalty_blocks_first_pass_domination() {
    // 非终章 WL（issue #23）：A(900) 在支援表内，编入队伍损失 4.0 支援加成；
    // 第一轮支配若支援盲会裁掉 B(901)，而真实 Top-1 是 B 卡组。
    let cards = [
        skill_card(900, 0, 300, 10),
        skill_card(901, 0, 295, 10),
        skill_card(902, 1, 400, 10),
        skill_card(903, 2, 410, 10),
        skill_card(904, 3, 420, 10),
        skill_card(905, 4, 296, 10),
        skill_card(906, 5, 294, 10),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.is_world_bloom = true;
    search_ctx.event_type = Some(EventType::WorldBloom);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.live_skill_order = LiveSkillOrder::Average;
    search_ctx.best_skill_as_leader = false;
    search_ctx.support_deck.cards = vec![(900, 5.0), (998, 1.0)];
    search_ctx.support_deck.count = 1;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    // 前提：支援惩罚阻止 A 支配 B，第一轮不得裁任何卡。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[0]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 901),
        "top-1 must contain B (901): the support-listed dominator forfeits its bonus in deck",
    );
}
