//! Compact representations must reject overflow, never change the problem.
use super::*;

fn checked_build(
    fixture: &BonusTierFixture,
    game: &GameData<'_>,
    params: &BuildParams,
) -> Result<(crate::pool::CardPool, crate::search::SearchContext), BuildError> {
    let user = pool_constraint_user(fixture);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build_card_pool(&user, game, params)
    }));
    assert!(
        caught.is_ok(),
        "unsupported metadata must produce a typed error, not a panic"
    );
    caught.unwrap()
}

#[test]
fn capacity_skill_overflow_is_an_error_not_saturation() {
    for kind in ["normal", "unit", "diff", "reference", "reference-rate"] {
        let mut fixture = pool_constraint_fixture(&[(1, 4, 100)]);
        let base = fixture.effects[0].clone();
        match kind {
            "normal" => fixture.effects[0].value = 300,
            "unit" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_unit_count".to_string(),
                value: 300,
                unit: Some("idol".to_string()),
                unit_member_count: Some(5),
                ..base
            }),
            "diff" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_diff".to_string(),
                value: 200,
                additional_value: Some(30),
                ..base
            }),
            "reference" => {
                fixture.effects[0].value = 200;
                fixture.effects.push(types::SkillEffect {
                    effect_type: "score_up_reference".to_string(),
                    value: 100,
                    additional_value: Some(100),
                    ..base
                });
            }
            "reference-rate" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_reference".to_string(),
                value: 300,
                additional_value: Some(100),
                ..base
            }),
            _ => unreachable!(),
        }
        let game = bonus_tier_game(&fixture);
        let params = BuildParams {
            target: ScoreTarget::Skill,
            ..Default::default()
        };
        assert!(
            checked_build(&fixture, &game, &params).is_err(),
            "kind={kind}"
        );
    }
}

#[test]
fn capacity_power_overflow_is_an_error_not_bit_truncation() {
    let fixture = pool_constraint_fixture(&[(1, 4, 100_000)]);
    let game = bonus_tier_game(&fixture);
    assert!(checked_build(&fixture, &game, &BuildParams::default()).is_err());
}

#[test]
fn capacity_public_card_identity_is_not_clamped() {
    let mut fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    fixture.master_cards[0].id = 70_000;
    fixture.card_params[0].card_id = 70_000;
    let game = bonus_tier_game(&fixture);
    assert!(checked_build(&fixture, &game, &BuildParams::default()).is_err());
}

#[test]
fn capacity_bonus_overflow_returns_an_error_before_packing() {
    let fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    let event_cards = [types::EventCard {
        event_id: 42,
        card_id: 1,
        bonus_rate_x10: 5_000,
        leader_bonus_rate_x10: 0,
    }];
    let game = GameData {
        event_cards: &event_cards,
        ..bonus_tier_game(&fixture)
    };
    let params = BuildParams {
        event_id: Some(42),
        event_type: Some("marathon".to_string()),
        ..Default::default()
    };
    assert!(checked_build(&fixture, &game, &params).is_err());
}

#[test]
fn capacity_limited_bonus_side_table_returns_an_error() {
    let spec = (1..=16)
        .map(|character| (character, 4, 100))
        .collect::<Vec<_>>();
    let fixture = pool_constraint_fixture(&spec);
    let event_cards = (1..=16)
        .map(|card_id| types::EventCard {
            event_id: 42,
            card_id,
            bonus_rate_x10: card_id * 10,
            leader_bonus_rate_x10: 0,
        })
        .collect::<Vec<_>>();
    let game = GameData {
        event_cards: &event_cards,
        ..bonus_tier_game(&fixture)
    };
    let params = BuildParams {
        event_id: Some(42),
        event_type: Some("marathon".to_string()),
        ..Default::default()
    };
    assert!(checked_build(&fixture, &game, &params).is_err());
}

fn reference_fixture(distinct: bool) -> BonusTierFixture {
    let spec = (0..256)
        .map(|index| (index % 26 + 1, 4, 100))
        .collect::<Vec<_>>();
    let mut fixture = pool_constraint_fixture(&spec);
    let base = fixture.effects[0].clone();
    fixture.effects.clear();
    fixture.skills.clear();
    for (index, card) in fixture.master_cards.iter_mut().enumerate() {
        let skill_id = 10 + index as i32;
        card.skill_id = skill_id;
        fixture.skills.push(types::Skill {
            id: skill_id,
            level: 1,
            is_after_training: false,
        });
        fixture.effects.push(types::SkillEffect {
            skill_id,
            ..base.clone()
        });
        fixture.effects.push(types::SkillEffect {
            skill_id,
            effect_type: "score_up_reference".to_string(),
            value: if distinct { index as i32 % 255 + 1 } else { 50 },
            additional_value: Some(if distinct { index as i32 / 255 + 1 } else { 50 }),
            ..base.clone()
        });
    }
    fixture
}

#[test]
fn capacity_identical_special_skills_are_interned() {
    let fixture = reference_fixture(false);
    let game = bonus_tier_game(&fixture);
    let (pool, _) = checked_build(&fixture, &game, &BuildParams::default())
        .expect("256 identical reference entries fit one interned slot");
    assert_eq!(pool.count(), 256);
    assert_eq!(pool.special().ref_skills().len(), 1);
}

#[test]
fn capacity_distinct_special_skills_returns_an_error() {
    let fixture = reference_fixture(true);
    let game = bonus_tier_game(&fixture);
    assert!(checked_build(&fixture, &game, &BuildParams::default()).is_err());
}

#[test]
fn capacity_applies_a_real_event_cap_before_checking_skill_width() {
    for kind in ["normal", "unit", "diff", "reference"] {
        let mut fixture = pool_constraint_fixture(&[(1, 4, 100)]);
        let base = fixture.effects[0].clone();
        match kind {
            "normal" => fixture.effects[0].value = 300,
            "unit" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_unit_count".to_string(),
                value: 400,
                unit: Some("idol".to_string()),
                unit_member_count: Some(5),
                ..base
            }),
            "diff" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_diff".to_string(),
                value: 200,
                additional_value: Some(300),
                ..base
            }),
            "reference" => fixture.effects.push(types::SkillEffect {
                effect_type: "score_up_reference".to_string(),
                value: 100,
                additional_value: Some(300),
                ..base
            }),
            _ => unreachable!(),
        }
        let limits = [types::EventSkillScoreUpLimit {
            event_id: 42,
            score_up_limit: 240,
        }];
        let game = GameData {
            event_skill_score_up_limits: &limits,
            ..bonus_tier_game(&fixture)
        };
        let params = BuildParams {
            event_id: Some(42),
            event_type: Some("marathon".to_string()),
            ..Default::default()
        };
        let (pool, _) = checked_build(&fixture, &game, &params).unwrap();
        assert_eq!(pool.count(), 1);
        let card = pool.card_idx(0).unwrap();
        assert_eq!(pool.skill_max(card), 140, "kind={kind}");
        if let Some(reference) = pool.special().ref_skills().first() {
            assert_eq!(
                u16::from(pool.skill_min(card)) + u16::from(reference.max),
                140
            );
        }
    }
}

#[test]
fn capacity_maximum_public_id_is_supported() {
    let mut fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    fixture.master_cards[0].id = 65_535;
    fixture.card_params[0].card_id = 65_535;
    let game = bonus_tier_game(&fixture);
    let (pool, _) = checked_build(&fixture, &game, &BuildParams::default()).unwrap();
    assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), u16::MAX);
}

#[test]
fn capacity_support_only_id_is_checked_before_main_filtering() {
    let mut fixture = pool_constraint_fixture(&[
        (1, 4, 100),
        (2, 4, 100),
        (3, 4, 100),
        (4, 4, 100),
        (5, 4, 100),
        (6, 4, 100),
    ]);
    fixture.master_cards[5].id = 70_000;
    fixture.card_params[5].card_id = 70_000;
    let events = [types::Event {
        id: 214,
        event_type: "world_bloom".to_string(),
    }];
    let chapters = [types::WorldBloom {
        event_id: 214,
        game_character_id: Some(1),
        chapter_no: 1,
        world_bloom_chapter_type: Some("game_character".to_string()),
    }];
    let game = GameData {
        events: &events,
        world_blooms: &chapters,
        ..bonus_tier_game(&fixture)
    };
    let params = BuildParams {
        event_id: Some(214),
        excluded_cards: vec![70_000],
        ..Default::default()
    };
    assert!(matches!(
        checked_build(&fixture, &game, &params),
        Err(BuildError::CapacityExceeded {
            field: "public card id",
            value: 70_000,
            max: 65_535
        })
    ));
}

#[test]
fn capacity_negative_character_is_not_silently_changed_to_zero() {
    let mut fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    fixture.master_cards[0].character_id = -1;
    let game = bonus_tier_game(&fixture);
    assert!(matches!(
        checked_build(&fixture, &game, &BuildParams::default()),
        Err(BuildError::InvalidConfig(_))
    ));
}
