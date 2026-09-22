//! Completion-sensitive dominance regressions, independent of search bounds.
use super::*;

fn support_conflict(duplicate_variant: bool, reserve_entry: bool) {
    let mut cards = vec![
        skill_card(300, 1, 1100, 40),
        skill_card(301, 1, 1099, 40),
        skill_card(302, 2, 1000, 40),
        skill_card(303, 3, 1000, 40),
        skill_card(304, 4, 1000, 40),
        skill_card(305, 5, 1000, 40),
    ];
    if duplicate_variant {
        cards.push(skill_card(300, 1, 1050, 40));
    }
    let pool = build_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.live_type = LiveType::Multi;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.is_world_bloom = true;
    ctx.fixed_card_ids = vec![302, 303, 304, 305];
    ctx.skill_scores[1] = [0.2; 6];
    ctx.support_deck = if reserve_entry {
        SupportDeck {
            count: 2,
            cards: vec![
                (302, 40.0),
                (303, 30.0),
                (300, 20.0),
                (900, 1.0),
                (901, 1.0),
            ],
        }
    } else {
        SupportDeck {
            count: 1,
            cards: vec![(300, 20.0), (900, 1.0), (901, 1.0)],
        }
    };
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    assert!(expected[0].cards.iter().any(|&c| pool.game_id(c) == 301));
    let (actual, stats) = search_instrumented(&pool, &ctx, &params);
    assert!(!stats.deadline_hit);
    assert_property_results(
        &pool,
        &actual,
        &expected,
        "support opportunity cost dominance",
    );
}

#[test]
fn dominance_support_reserve_can_enter_after_other_main_cards_are_excluded() {
    support_conflict(false, true);
}

#[test]
fn dominance_support_penalty_applies_to_every_cultivation_variant() {
    support_conflict(true, false);
}

#[test]
fn dominance_reference_skill_must_compare_base_and_preserved_training_state() {
    let mut weak = skill_card(400, 1, 1100, 40);
    weak.skill = SkillSlot {
        skill_type: 3,
        value: 1,
    };
    let mut strong = skill_card(401, 1, 1099, 100);
    strong.skill = SkillSlot {
        skill_type: 3,
        value: 1,
    };
    let pool = build_pool(&[
        weak,
        strong,
        skill_card(402, 2, 1000, 40),
        skill_card(403, 3, 1000, 40),
        skill_card(404, 4, 1000, 40),
        skill_card(405, 5, 1000, 40),
    ]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
    ctx.live_type = LiveType::Multi;
    ctx.skill_scores[1] = [0.2; 6];
    // Both cards are kept in their primary state, so skill_min == skill_max is
    // a valid exact metadata bound. Their identical reference tables do not
    // make their different primary scores interchangeable.
    ctx.keep_after_training_state = true;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    assert!(expected[0].cards.iter().any(|&c| pool.game_id(c) == 401));
    let (actual, _) = search_instrumented(&pool, &ctx, &params);
    assert_property_results(&pool, &actual, &expected, "reference base dominance");
}
