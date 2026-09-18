//! prepared search contracts.
use super::*;

#[test]
fn prepared_multi_event_search_matches_standard_pipeline() {
    let mut cards = five_unique_cards().to_vec();
    cards.extend([
        TestCard {
            char_id: 5,
            attr: 1,
            unit_mask: 1,
            game_id: 150,
            power: 650,
            skill: SkillSlot {
                skill_type: 0,
                value: 65,
            },
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 650,
            skill_max: 65,
        },
        TestCard {
            char_id: 6,
            attr: 2,
            unit_mask: 1,
            game_id: 151,
            power: 625,
            skill: SkillSlot {
                skill_type: 0,
                value: 72,
            },
            base_bonus: 35,
            limited_bonus: 0,
            power_max: 625,
            skill_max: 72,
        },
        TestCard {
            char_id: 0,
            attr: 3,
            unit_mask: 1,
            game_id: 152,
            power: 620,
            skill: SkillSlot {
                skill_type: 0,
                value: 60,
            },
            base_bonus: 45,
            limited_bonus: 0,
            power_max: 620,
            skill_max: 60,
        },
    ]);
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.skill_scores[1] = [10.0; DECK_SIZE + 1];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let expected = search_exact(&pool, &search_ctx, &params);
    let prepared = PreparedSearch::build(&pool, &search_ctx, params.top_k).unwrap();
    let (actual, _) = prepared
        .search_instrumented(&pool, &search_ctx, &params)
        .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn prepared_plan_rejects_a_changed_query_or_pool_instance() {
    let cards = five_unique_cards();
    let pool = build_pool(&cards);
    let ctx = ready_ctx(&pool, ScoreTarget::Score);
    let prepared = PreparedSearch::build(&pool, &ctx, 8).unwrap();
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let mut changed = ctx.clone();
    changed.base_score *= 2.0;
    assert!(
        prepared
            .search_instrumented(&pool, &changed, &params)
            .is_none(),
        "query-dependent bounds cannot serve a changed query"
    );
    let mut other_cards = cards;
    other_cards[0].power *= 2;
    other_cards[0].power_max *= 2;
    let other_pool = build_pool(&other_cards);
    assert!(
        prepared
            .search_instrumented(&other_pool, &ctx, &params)
            .is_none(),
        "cached indices cannot refer to a different pool"
    );
    // Moving the original immutable pool does not invalidate its plan.
    let moved = Box::new(pool);
    let (actual, _) = prepared.search_instrumented(&moved, &ctx, &params).unwrap();
    assert_eq!(actual, search_exact(&moved, &ctx, &params));
}
