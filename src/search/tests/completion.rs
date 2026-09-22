//! Expiry is a solver event, not an estimate inferred from wall-clock duration.
use super::*;
use crate::search::evaluate::leaf_evaluate_checked;

#[test]
fn final_search_reports_actual_deadline_stop() {
    let pool = build_pool(&longtail_cards(22, 6));
    let mut ctx = final_chapter_ctx(&pool);
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.diff_attr_bonus = [0, 0, 0, 8, 30, 115];
    ctx.support_deck = longtail_support(&pool, 0);
    let (decks, stats) = search_instrumented(
        &pool,
        &ctx,
        &SearchParams {
            top_k: 100,
            timeout_ms: 1,
        },
    );
    assert!(
        stats.deadline_hit,
        "the large Final job must not certify its early deadline exit"
    );
    for deck in decks {
        assert!(leaf_evaluate_checked(&pool, &ctx, &deck.cards).is_some());
    }
}

#[test]
fn power_scenarios_honor_a_finite_deadline() {
    let pool = build_pool(&longtail_cards(26, 8));
    let ctx = ready_ctx(&pool, ScoreTarget::Power);
    let (decks, stats) = search_instrumented(
        &pool,
        &ctx,
        &SearchParams {
            top_k: 100,
            timeout_ms: 1,
        },
    );
    assert!(
        stats.deadline_hit,
        "a multi-scenario DP must report unfinished scenarios"
    );
    for deck in decks {
        assert!(leaf_evaluate_checked(&pool, &ctx, &deck.cards).is_some());
    }
}

#[test]
fn final_auto_enumerates_character_zero_as_a_leader() {
    let pool = build_pool(&five_unique_cards());
    let mut ctx = final_chapter_ctx(&pool);
    ctx.leader_honor_bonus_x10[0] = 40_000;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let expected = ExactOracle::new(&pool, &ctx).search(&params).0;
    assert_eq!(expected.len(), 1);
    let (actual, stats) = crate::search::tuning::with_tuning(
        crate::search::tuning::SearchTuning {
            final_seeds: false,
            ..Default::default()
        },
        || search_instrumented(&pool, &ctx, &params),
    );
    assert_eq!(stats.diagnostics.seed_leaves, 0);
    assert_eq!(pool.char_id(actual[0].cards[0]), 0);
    assert!(!stats.deadline_hit);
    assert_property_results(&pool, &actual, &expected, "Final character-zero leader");
    assert_property_scores(
        &pool,
        &ctx,
        &actual,
        &expected,
        "Final character-zero objective",
    );
    assert!(
        stats.visited_nodes > 0,
        "Final proof and seed work must be observable"
    );
}
