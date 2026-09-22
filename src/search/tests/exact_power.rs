//! exact power contracts.
use super::*;

#[test]
fn search_leaf_evaluate_applies_power_cap() {
    let cards = five_unique_cards().map(|mut card| {
        card.power = 1000;
        card.power_max = 1000;
        card
    });
    let pool = build_pool(&cards);
    let deck = collect_first_five(&pool);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.power_total_cap = Some(3_500);

    assert_eq!(leaf_evaluate(&pool, &search_ctx, &deck), 3_500);
}

#[test]
fn search_minimize_power_matches_bruteforce_worst() {
    // 最弱组卡：minimize=true 应返回 power 最小的 5 角色互异 deck，
    // 与暴力枚举一致。卡池跨 7 角色、power 各异，确保有真正的「最弱」组合。
    let mut cards = Vec::new();
    let powers = [120u32, 90, 250, 60, 400, 30, 180];
    for (i, &p) in powers.iter().enumerate() {
        cards.push(TestCard {
            char_id: i as u8,
            attr: (i % 4) as u8,
            unit_mask: 1,
            game_id: 200 + i as u16,
            power: p,
            skill: SkillSlot {
                skill_type: 0,
                value: 10,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: p,
            skill_max: 10,
        });
    }
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.minimize = true;

    let results = search_exact(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(results.len(), 1, "minimize 应返回 1 个结果");

    let expected = brute_force_worst_power(&pool, &search_ctx);
    assert_eq!(
        results[0].score, expected,
        "minimize 搜索结果应等于暴力最弱 power"
    );

    // 最弱解应为 5 张最小 power 卡：30+60+90+120+180 = 480。
    assert_eq!(results[0].score, 480);

    // 反向验证：同池 maximize 应严格更大（取最强 5 张）。
    let mut max_ctx = ready_ctx(&pool, ScoreTarget::Power);
    max_ctx.minimize = false;
    let max_results = search_exact(
        &pool,
        &max_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert!(
        max_results[0].score > results[0].score,
        "maximize({}) 应大于 minimize({})",
        max_results[0].score,
        results[0].score
    );
}
