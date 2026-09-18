//! Source-level leader bonus precision must survive pool construction.
use super::*;

fn final_params() -> BuildParams {
    BuildParams {
        target: ScoreTarget::Score,
        live_type: LiveType::Multi,
        event_id: Some(crate::types::FINAL_CHAPTER_EVENT_ID),
        event_type: Some("world_bloom".to_string()),
        fixed_cards: vec![1],
        ..Default::default()
    }
}

#[test]
fn fractional_final_leader_bonus_survives_build_and_display() {
    let fixture = pool_constraint_fixture(&[
        (1, 4, 100),
        (2, 4, 100),
        (3, 4, 100),
        (4, 4, 100),
        (5, 4, 100),
    ]);
    let params = final_params();
    let user = pool_constraint_user(&fixture);
    for leader_x10 in [1, 5, 9, 10, 15, 19, 20] {
        let events = [types::EventCard {
            event_id: params.event_id.unwrap(),
            card_id: 1,
            bonus_rate_x10: 0,
            leader_bonus_rate_x10: leader_x10,
        }];
        let game = GameData {
            event_cards: &events,
            ..bonus_tier_game(&fixture)
        };
        let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
        let decks = pool_constraint_search(&pool, &ctx, &params);
        assert_eq!(decks.len(), 1);
        let summary = crate::search::summarize_deck(&pool, &ctx, &decks[0].cards).unwrap();
        assert_eq!(
            summary.event_bonus_total,
            Some(f64::from(leader_x10) / 10.0),
            "source leader bonus={leader_x10}/10"
        );
    }
}

#[test]
fn fractional_final_leader_and_member_form_an_exact_tier() {
    let fixture = pool_constraint_fixture(&[
        (1, 4, 100),
        (2, 4, 100),
        (3, 4, 100),
        (4, 4, 100),
        (5, 4, 100),
    ]);
    let params = final_params();
    let events = [
        types::EventCard {
            event_id: params.event_id.unwrap(),
            card_id: 1,
            bonus_rate_x10: 0,
            leader_bonus_rate_x10: 15,
        },
        types::EventCard {
            event_id: params.event_id.unwrap(),
            card_id: 2,
            bonus_rate_x10: 5,
            leader_bonus_rate_x10: 10,
        },
    ];
    let game = GameData {
        event_cards: &events,
        ..bonus_tier_game(&fixture)
    };
    let user = pool_constraint_user(&fixture);
    let (pool, mut ctx) = build_card_pool(&user, &game, &params).unwrap();
    // The public Final API still accepts only Score; this additionally checks
    // the exact tier kernel without weakening that validation boundary.
    ctx.target = ScoreTarget::Bonus;
    let decks = crate::search::search_bonus_targets(
        &pool,
        &ctx,
        &crate::search::SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
        &[2],
    )
    .0;
    assert_eq!(
        decks.len(),
        1,
        "1.5% leader + 0.5% member must reach exact 2%"
    );
    assert_eq!(
        crate::search::summarize_deck(&pool, &ctx, &decks[0].cards)
            .unwrap()
            .event_bonus_total,
        Some(2.0)
    );
}

#[test]
fn leader_bonus_width_overflow_returns_an_explicit_error() {
    let fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    let params = final_params();
    let events = [types::EventCard {
        event_id: params.event_id.unwrap(),
        card_id: 1,
        bonus_rate_x10: 0,
        leader_bonus_rate_x10: 1_000_000,
    }];
    let game = GameData {
        event_cards: &events,
        ..bonus_tier_game(&fixture)
    };
    let user = pool_constraint_user(&fixture);
    assert!(matches!(
        build_card_pool(&user, &game, &params),
        Err(BuildError::CapacityExceeded { .. })
    ));
}
