use super::*;

#[test]
fn final_members_builds_without_pin_order_and_rejects_unsupported_requests() {
    let fixture = pool_constraint_fixture(&[
        (1, 4, 100),
        (5, 4, 100),
        (9, 4, 100),
        (13, 4, 100),
        (17, 4, 100),
        (21, 4, 100),
    ]);
    let user = pool_constraint_user(&fixture);
    let events = [types::Event {
        id: 218,
        event_type: "world_bloom".into(),
    }];
    let chapters = [types::WorldBloom {
        event_id: 218,
        game_character_id: None,
        chapter_no: 1,
        world_bloom_chapter_type: Some("finale".into()),
    }];
    let limits = [types::EventSkillScoreUpLimit {
        event_id: 218,
        score_up_limit: 140,
    }];
    let game = GameData {
        events: &events,
        world_blooms: &chapters,
        event_skill_score_up_limits: &limits,
        ..bonus_tier_game(&fixture)
    };
    let mut params = BuildParams {
        event_id: Some(218),
        fixed_cards: vec![1, 2],
        forced_leader_character_id: Some(21),
        fixed_constraint_mode: types::FixedConstraintMode::Members,
        ..Default::default()
    };
    let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
    assert!(ctx.uses_member_constraints());
    assert_eq!(ctx.fixed_prefix_len(), 0);
    params.fixed_cards.reverse();
    let (reversed, _) = build_card_pool(&user, &game, &params).unwrap();
    assert_eq!(
        pool.indices()
            .map(|card| pool.game_id(card))
            .collect::<Vec<_>>(),
        reversed
            .indices()
            .map(|card| reversed.game_id(card))
            .collect::<Vec<_>>()
    );
    params.target = ScoreTarget::Bonus;
    params.target_bonus_list = vec![100];
    assert!(matches!(build_card_pool(&user, &game, &params),
        Err(BuildError::InvalidConfig(message)) if message.contains("target_bonus_list")));
    params.target_bonus_list.clear();
    params.fixed_cards.clear();
    params.target_bonus_list = vec![100];
    let (_, only_leader) = build_card_pool(&user, &game, &params).unwrap();
    assert_eq!(
        only_leader.fixed_constraint_mode,
        types::FixedConstraintMode::Slots
    );
    params.fixed_characters = vec![21];
    let (_, pinned_leader) = build_card_pool(&user, &game, &params).unwrap();
    assert_eq!(
        pinned_leader.fixed_constraint_mode,
        types::FixedConstraintMode::Slots
    );
    params.target_bonus_list.clear();
    let ordinary = GameData {
        world_blooms: &[],
        ..game
    };
    assert!(matches!(build_card_pool(&user, &ordinary, &params),
        Err(BuildError::InvalidConfig(message)) if message.contains("requires a World Bloom final chapter")));
}
