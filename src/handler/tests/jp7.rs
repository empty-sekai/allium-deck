use super::*;
use crate::power::MultiUnitBonusMode;

fn item(id: i32, unit: Option<&str>, rate: [f64; 3]) -> types::AreaItemLevel {
    types::AreaItemLevel {
        area_item_id: id,
        level: 1,
        unit: unit.map(str::to_string),
        attr: None,
        character_id: Some(0),
        power_rate: rate,
        power_all_match_rate: None,
    }
}

#[test]
fn owned_level_above_master_max_keeps_all_effect_rows() {
    let rows = [
        item(56, None, [1.0, 2.0, 3.0]),
        item(56, Some("multi_unit"), [4.0; 3]),
    ];
    let fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    let game = GameData {
        area_item_levels: &rows,
        ..bonus_tier_game(&fixture)
    };
    let idx = index::PoolIndexes::build(&game);
    assert_eq!(idx.area_items(56, 20).len(), 2);
    assert_eq!(idx.area_items(56, 0).len(), 0);
    assert_eq!(idx.area_items(99, 20).len(), 0);
    let mut user = pool_constraint_user(&fixture);
    user.user_area_items = vec![types::UserAreaItem {
        area_item_id: 56,
        level: 20,
    }];
    let params = BuildParams {
        target: ScoreTarget::Power,
        multi_unit_bonus_mode: MultiUnitBonusMode::ForceOn,
        ..Default::default()
    };
    let (pool, _, full) = build_card_pool_with_details(&user, &game, &params).unwrap();
    assert!(pool.has_multi_power());
    assert_eq!(full[0].multi_power.unwrap()[0].area_item_bonus, 18);
}

#[test]
fn modes_are_identical_without_owned_multi_rows() {
    let rows = [
        item(1, None, [1.0; 3]),
        item(56, Some("multi_unit"), [50.0; 3]),
    ];
    let fixture = pool_constraint_fixture(&[
        (1, 4, 100),
        (2, 4, 101),
        (3, 4, 102),
        (4, 4, 103),
        (5, 4, 104),
    ]);
    let game = GameData {
        area_item_levels: &rows,
        ..bonus_tier_game(&fixture)
    };
    let mut user = pool_constraint_user(&fixture);
    user.user_area_items = vec![types::UserAreaItem {
        area_item_id: 1,
        level: 1,
    }];
    let mut expected = None;
    for mode in [
        MultiUnitBonusMode::ByDeck,
        MultiUnitBonusMode::ForceOn,
        MultiUnitBonusMode::ForceOff,
    ] {
        let params = BuildParams {
            target: ScoreTarget::Power,
            multi_unit_bonus_mode: mode,
            ..Default::default()
        };
        let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
        assert!(!pool.has_multi_power());
        let cards: Vec<_> = pool.indices().collect();
        let score = crate::search::resolve_power_for_cards(&pool, &cards);
        assert_eq!(score, *expected.get_or_insert(score));
        assert!(!ctx.is_wl3_finale);
    }
}

#[test]
fn gates_select_level_before_rate_and_keep_missing_level_ties() {
    let mut fixture = pool_constraint_fixture(&[(21, 4, 100)]);
    fixture
        .units
        .iter_mut()
        .find(|unit| unit.game_character_id == 21)
        .unwrap()
        .unit = "piapro".into();
    let gates = [
        types::MysekaiGate {
            id: 1,
            unit: "idol".into(),
        },
        types::MysekaiGate {
            id: 2,
            unit: "street".into(),
        },
        types::MysekaiGate {
            id: 6,
            unit: "none".into(),
        },
    ];
    let levels = [
        types::MysekaiGateLevel {
            mysekai_gate_id: 1,
            level: 1,
            power_bonus_rate: 9.0,
        },
        types::MysekaiGateLevel {
            mysekai_gate_id: 2,
            level: 2,
            power_bonus_rate: 1.0,
        },
    ];
    for (entries, expected) in [
        (vec![(1, 1), (2, 2)], 3),
        (vec![(1, 1), (6, 3)], 0),
        (vec![(6, 2), (2, 2)], 0),
        (vec![(2, 2), (6, 2)], 3),
        (vec![(99, 3), (2, 2)], 0),
    ] {
        let game = GameData {
            mysekai_gates: &gates,
            mysekai_gate_levels: &levels,
            ..bonus_tier_game(&fixture)
        };
        let mut user = pool_constraint_user(&fixture);
        user.user_mysekai_gate_bonuses = entries
            .into_iter()
            .map(|(id, level)| types::UserGateBonus {
                mysekai_gate_id: Some(id),
                mysekai_gate_level: Some(level),
                unit: String::new(),
                bonus_rate: 99.0,
            })
            .collect();
        let (_, _, full) =
            build_card_pool_with_details(&user, &game, &BuildParams::default()).unwrap();
        assert_eq!(full[0].power[5][0].gate_bonus, expected);
    }
}

#[test]
fn raw_master_skill_caps_and_finale_fallbacks_are_distinct() {
    let fixture = pool_constraint_fixture(&[(1, 4, 100)]);
    let user = pool_constraint_user(&fixture);
    for cap in [140, 240] {
        let mut effects = fixture.effects.clone();
        effects[0].value = 300;
        let limits = [types::EventSkillScoreUpLimit {
            event_id: 42,
            score_up_limit: cap,
        }];
        let game = GameData {
            skill_effects: &effects,
            event_skill_score_up_limits: &limits,
            ..bonus_tier_game(&fixture)
        };
        let params = BuildParams {
            event_id: Some(42),
            event_type: Some("marathon".into()),
            ..Default::default()
        };
        let (pool, _) = build_card_pool(&user, &game, &params).unwrap();
        assert_eq!(pool.skill_max(pool.card_idx(0).unwrap()), cap as u8);
    }
    for (turn, limited, fixture_cap) in [(2, 4, 20), (3, 5, 60)] {
        let game = bonus_tier_game(&fixture);
        let params = BuildParams {
            world_bloom_finale_turn: Some(turn),
            world_bloom_character_id: Some(1),
            ..Default::default()
        };
        let event = event_bonus::build_event_context(&game, &params)
            .unwrap()
            .unwrap();
        assert_eq!(event.card_bonus_count_limit, limited);
        assert_eq!(
            resolve_fixture_bonus_limit(&game, Some(&event)),
            Some(fixture_cap)
        );
        let limits = [types::EventFixtureBonusLimit {
            event_id: event.event_id,
            bonus_rate_limit: 17,
        }];
        let override_game = GameData {
            event_mysekai_fixture_performance_bonus_limits: &limits,
            ..game
        };
        assert_eq!(
            resolve_fixture_bonus_limit(&override_game, Some(&event)),
            Some(17)
        );
    }
}
