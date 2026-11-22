use super::*;
use crate::power::MultiUnitBonusMode;

#[test]
fn optional_shuffle_table_and_finale_chapters_share_the_master_loader() {
    let required = [
        "cards.json",
        "gameCharacterUnits.json",
        "events.json",
        "skills.json",
        "cardRarities.json",
        "cardEpisodes.json",
        "masterLessons.json",
        "areaItemLevels.json",
        "characterRanks.json",
        "cardMysekaiCanvasBonuses.json",
        "eventCards.json",
        "eventDeckBonuses.json",
        "worldBloomDifferentAttributeBonuses.json",
        "eventRarityBonusRates.json",
    ];
    let mut sources = MasterdataSources::from_strings(
        required.into_iter().map(|name| (name.into(), "[]".into())),
        "[]".into(),
    );
    let plain = OwnedGameData::from_sources(&sources).unwrap();
    assert!(plain.event_shuffle_unit_bonuses.is_empty());
    sources.tables.insert(
        "worldBlooms.json".into(),
        serde_json::json!([
            {"eventId":931,"worldBloomChapterType":"finale","chapterNo":1}
        ])
        .to_string(),
    );
    sources.tables.insert(
        "eventShuffleUnitBonuses.json".into(),
        serde_json::json!([
            {"eventId":931,"unitCount":3,"bonusRate":71},
            {"eventId":931,"unitCount":5,"bonusRate":19}
        ])
        .to_string(),
    );
    let loaded = OwnedGameData::from_sources(&sources).unwrap();
    let game = loaded.as_ref();
    assert!(game.is_world_bloom_finale(931));
    assert!(!game.is_world_bloom_finale(932));
    assert_eq!(game.event_shuffle_unit_bonuses[0].bonus_rate, 71);
    assert_eq!(game.event_shuffle_unit_bonuses[1].unit_count, 5);
    let encoded = serde_json::to_value(&loaded).unwrap();
    let roundtrip: OwnedGameData = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        roundtrip.event_shuffle_unit_bonuses,
        loaded.event_shuffle_unit_bonuses
    );
    sources
        .tables
        .insert("eventShuffleUnitBonuses.json".into(), "null".into());
    assert!(
        OwnedGameData::from_sources(&sources)
            .unwrap_err()
            .contains("eventShuffleUnitBonuses.json")
    );
    sources
        .tables
        .insert("eventShuffleUnitBonuses.json".into(), "[]".into());
    sources.tables.insert(
        "worldBlooms.json".into(),
        serde_json::json!([
            {"eventId":931,"worldBloomChapterType":null,"chapterNo":1}
        ])
        .to_string(),
    );
    assert!(
        !OwnedGameData::from_sources(&sources)
            .unwrap()
            .as_ref()
            .is_world_bloom_finale(931)
    );
}

#[test]
fn area_rows_keep_three_dimensions_nullability_and_master_order() {
    let input = serde_json::json!([
        {"areaItemId":56,"level":1,"targetUnit":"multi_unit","targetGameCharacterId":0,
        "power1BonusRate":1.0,"power2BonusRate":2.0,"power3BonusRate":3.0,
        "power1AllMatchBonusRate":5.0,"power2AllMatchBonusRate":null,"power3AllMatchBonusRate":7.0},
        {"areaItemId":56,"level":1,"targetUnit":"any","targetGameCharacterId":0,
        "power1BonusRate":0.1,"power2BonusRate":0.2,"power3BonusRate":0.3,
        "power1AllMatchBonusRate":1.0,"power2AllMatchBonusRate":2.0,"power3AllMatchBonusRate":3.0}
    ]);
    let rows: Vec<RawAreaItemLevel> = serde_json::from_value(input).unwrap();
    let rows = flatten_area_item_levels(rows);
    assert_eq!(rows[0].unit.as_deref(), Some("multi_unit"));
    assert_eq!(rows[0].power_rate, [1.0, 2.0, 3.0]);
    assert_eq!(rows[0].character_id, None);
    assert_eq!(rows[0].power_all_match_rate, None);
    assert_eq!(rows[1].power_all_match_rate, Some([1.0, 2.0, 3.0]));
}

#[test]
fn evaluation_public_spellings_and_invalid_values() {
    for key in ["multi_unit_bonus_evaluation", "multiUnitBonusEvaluation"] {
        for (value, mode) in [
            ("by_deck", MultiUnitBonusMode::ByDeck),
            ("force_on", MultiUnitBonusMode::ForceOn),
            ("force_off", MultiUnitBonusMode::ForceOff),
        ] {
            let input = serde_json::json!({key:value}).to_string();
            assert_eq!(
                parse_build_params_json(&input)
                    .unwrap()
                    .multi_unit_bonus_mode,
                mode
            );
        }
        for value in [
            serde_json::json!("sometimes"),
            serde_json::json!(true),
            serde_json::json!(7),
        ] {
            assert!(parse_build_params_json(&serde_json::json!({key:value}).to_string()).is_err());
        }
    }
    assert_eq!(
        parse_build_params_json("{}").unwrap().multi_unit_bonus_mode,
        MultiUnitBonusMode::ByDeck
    );
}

#[test]
fn multi_teammate_cap_follows_configs_event_score_ceiling() {
    let (mut game, user, _) = super::tests::minimal_game_and_user();
    game.cards.iter_mut().for_each(|card| {
        card.attr = "cool".to_string();
    });
    for param in &mut game.card_parameters {
        param.param1 = 50_000;
        param.param2 = 50_000;
        param.param3 = 50_000;
    }
    game.events.push(crate::handler::Event {
        id: 1,
        event_type: "marathon".to_string(),
    });
    game.music_metas.push(crate::handler::MusicMeta {
        music_id: 1,
        difficulty: "expert".to_string(),
        event_rate_solo: 100,
        event_rate_multi: 100,
        event_rate_auto: 100,
        base_score: 1.0,
        base_score_auto: 1.0,
        fever_score: 0.0,
        solo_skill_scores: [0.0; 6],
        multi_skill_scores: [0.0; 6],
        auto_skill_scores: [0.0; 6],
        music_time: 100.0,
        tap_count: 100,
    });
    let params = crate::handler::BuildParams {
        live_type: crate::types::LiveType::Multi,
        target: crate::types::ScoreTarget::Score,
        event_id: Some(1),
        music_id: Some(1),
        music_diff: Some("expert".to_string()),
        limit: 1,
        ..Default::default()
    };

    for (ceiling, expected_cap) in [(None, 16), (Some("5500000"), 16), (Some("4500000"), 13)] {
        game.configs = ceiling
            .map(|value| {
                vec![
                    crate::handler::Config {
                        config_key: "date_change_hour".to_string(),
                        value: "4".to_string(),
                    },
                    crate::handler::Config {
                        config_key: "event_score_ceiling".to_string(),
                        value: value.to_string(),
                    },
                ]
            })
            .unwrap_or_default();
        let game_ref = game.as_ref();
        let (pool, ctx) = crate::handler::build_card_pool(&user, &game_ref, &params).unwrap();
        let outcome = recommend(&user, &game_ref, &params).unwrap();
        let deck = outcome.results[0];
        let dense: [crate::pool::CardIdx; 5] = deck
            .cards
            .map(|id| pool.indices().find(|&c| pool.game_id(c) == id).unwrap());
        let summary = crate::search::summarize_deck(&pool, &ctx, &dense).unwrap();
        assert!(4 * summary.live_score / 340_000 > 16);
        let expected_ep = 110 + summary.live_score / 17_000 + expected_cap;
        assert_eq!(summary.event_point, Some(expected_ep));
        assert_eq!((deck.score >> 32) as i32, expected_ep);
    }

    game.configs = vec![crate::handler::Config {
        config_key: "event_score_ceiling".to_string(),
        value: "not-a-number".to_string(),
    }];
    let err = recommend(&user, &game.as_ref(), &params).unwrap_err();
    assert!(
        err.to_string()
            .contains("Invalid configs.event_score_ceiling value: not-a-number"),
        "{err}"
    );
}

#[test]
fn configs_table_is_optional_in_master_loader() {
    let required = [
        "cards.json",
        "gameCharacterUnits.json",
        "events.json",
        "skills.json",
        "cardRarities.json",
        "cardEpisodes.json",
        "masterLessons.json",
        "areaItemLevels.json",
        "characterRanks.json",
        "cardMysekaiCanvasBonuses.json",
        "eventCards.json",
        "eventDeckBonuses.json",
        "worldBloomDifferentAttributeBonuses.json",
        "eventRarityBonusRates.json",
    ];
    let mut sources = MasterdataSources::from_strings(
        required.into_iter().map(|name| (name.into(), "[]".into())),
        "[]".into(),
    );
    let absent = OwnedGameData::from_sources(&sources).unwrap();
    assert!(absent.configs.is_empty());
    assert_eq!(
        crate::handler::resolve_event_score_ceiling(&absent.as_ref()).unwrap(),
        crate::types::DEFAULT_EVENT_SCORE_CEILING
    );

    sources.tables.insert(
        "configs.json".into(),
        serde_json::json!([
            {"configKey": "event_score_ceiling", "value": "4500000"}
        ])
        .to_string(),
    );
    let present = OwnedGameData::from_sources(&sources).unwrap();
    assert_eq!(present.configs.len(), 1);
    assert_eq!(
        crate::handler::resolve_event_score_ceiling(&present.as_ref()).unwrap(),
        4_500_000
    );
}

#[test]
fn regional_basis_point_bonus_rounding_applies_to_tw_kr_and_cn() {
    // 1) Character rank: float32(1.15) * 0.01f * 30000 = 344.99997 -> JP/EN floors to 344/dim (1032),
    //    while TW/KR/CN uses round_ties_even(1.15 * 100) = 115 bps -> (3 * 115 * 30000) / 10000 = 1035.
    // 2) Area items: rate=1.5 with param=10050/dim -> JP/EN floors 150.75 to 150/dim (450),
    //    while TW/KR/CN sums across all 3 dims before dividing by 10000 -> (150 * 30150) / 10000 = 452.
    let (mut game, mut user, _) = super::tests::minimal_game_and_user();
    for param in &mut game.card_parameters {
        if param.card_id == 501 {
            param.param1 = 30_000;
            param.param2 = 30_000;
            param.param3 = 30_000;
        } else {
            param.param1 = 10_050;
            param.param2 = 10_050;
            param.param3 = 10_050;
        }
    }
    game.character_ranks.push(crate::handler::CharacterRank {
        character_rank: 5,
        power_bonus_rate: 1.15,
    });
    game.area_item_levels.push(crate::handler::AreaItemLevel {
        area_item_id: 1,
        level: 1,
        unit: Some("idol".to_string()),
        attr: None,
        character_id: None,
        power_rate: [1.5; 3],
        power_all_match_rate: None,
    });
    game.area_item_levels.push(crate::handler::AreaItemLevel {
        area_item_id: 56,
        level: 1,
        unit: Some("multi_unit".to_string()),
        attr: None,
        character_id: None,
        power_rate: [1.5; 3],
        power_all_match_rate: None,
    });
    user.user_characters.push(crate::handler::UserCharacter {
        character_id: 1,
        character_rank: 5,
    });
    user.user_characters.push(crate::handler::UserCharacter {
        character_id: 2,
        character_rank: 5,
    });
    user.user_area_items = vec![
        crate::handler::UserAreaItem {
            area_item_id: 1,
            level: 1,
        },
        crate::handler::UserAreaItem {
            area_item_id: 56,
            level: 1,
        },
    ];

    let game_ref = game.as_ref();
    let expected_jp_char = ((((1.15_f64 as f32) * 0.01_f32) * 30_000.0_f32).floor() as i32) * 3;
    let expected_bp_char = ((115_i64 * 90_000) / 10_000) as i32;
    assert_eq!(expected_jp_char, 1032);
    assert_eq!(expected_bp_char, 1035);

    let expected_jp_area_single = ((1.5_f64 * 0.01_f64 * 10_050.0_f64).floor() as i32) * 3;
    let expected_bp_area_single = ((150_i64 * 30_150) / 10_000) as i32;
    assert_eq!(expected_jp_area_single, 450);
    assert_eq!(expected_bp_area_single, 452);

    let expected_jp_area_multi = ((3.0_f64 * 0.01_f64 * 10_050.0_f64).floor() as i32) * 3;
    let expected_bp_area_multi = ((300_i64 * 30_150) / 10_000) as i32;
    assert_eq!(expected_jp_area_multi, 903);
    assert_eq!(expected_bp_area_multi, 904);

    for (region, exp_char, exp_single, exp_multi) in [
        (
            "jp",
            expected_jp_char,
            expected_jp_area_single,
            expected_jp_area_multi,
        ),
        (
            "en",
            expected_jp_char,
            expected_jp_area_single,
            expected_jp_area_multi,
        ),
        (
            "tw",
            expected_bp_char,
            expected_bp_area_single,
            expected_bp_area_multi,
        ),
        (
            "kr",
            expected_bp_char,
            expected_bp_area_single,
            expected_bp_area_multi,
        ),
        (
            "cn",
            expected_bp_char,
            expected_bp_area_single,
            expected_bp_area_multi,
        ),
    ] {
        let params_off = crate::handler::BuildParams {
            region: region.to_string(),
            target: crate::types::ScoreTarget::Power,
            multi_unit_bonus_mode: crate::power::MultiUnitBonusMode::ForceOff,
            ..Default::default()
        };
        let (_, _, details_off) =
            crate::handler::build_card_pool_with_details(&user, &game_ref, &params_off).unwrap();
        let card1 = details_off
            .iter()
            .find(|card| card.character_id == 1)
            .unwrap();
        assert_eq!(
            card1.power[1][0].character_bonus, exp_char,
            "region={region}"
        );
        let card2_off = details_off
            .iter()
            .find(|card| card.character_id == 2)
            .unwrap();
        assert_eq!(
            card2_off.power[1][0].area_item_bonus, exp_single,
            "region={region}"
        );

        let params_on = crate::handler::BuildParams {
            region: region.to_string(),
            target: crate::types::ScoreTarget::Power,
            multi_unit_bonus_mode: crate::power::MultiUnitBonusMode::ForceOn,
            ..Default::default()
        };
        let (_, _, details_on) =
            crate::handler::build_card_pool_with_details(&user, &game_ref, &params_on).unwrap();
        let card2_on = details_on
            .iter()
            .find(|card| card.character_id == 2)
            .unwrap();
        assert_eq!(
            card2_on.multi_power.unwrap()[0].area_item_bonus,
            exp_multi,
            "region={region}"
        );
    }
}

#[test]
fn memory_event_all_units_preserve_master_rank_and_exclude_associated_singers() {
    let (mut game, mut user, _) = super::tests::minimal_game_and_user();
    // Characters: 1 (light_sound), 5 (idol), 9 (street), 13 (theme_park), 17 (school_refusal),
    // plus two cards for character 21 (piapro): 506 (bare VS) and 507 (VS associated with street).
    let char_by_card = [
        (501, 1, "light_sound", "cool", None),
        (502, 5, "idol", "cute", None),
        (503, 9, "street", "happy", None),
        (504, 13, "theme_park", "pure", None),
        (505, 17, "school_refusal", "mysterious", None),
        (506, 21, "piapro", "cool", None),
        (507, 22, "piapro", "cute", Some("street")),
    ];
    game.cards = char_by_card
        .iter()
        .map(
            |&(id, character_id, _, attr, support)| crate::handler::MasterCard {
                id,
                character_id,
                attr: attr.to_string(),
                card_rarity_type: 4,
                rarity: String::new(),
                asset_bundle_name: format!("card_{id:06}"),
                skill_id: 10,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: support.map(str::to_string),
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
        )
        .collect();
    game.card_parameters = char_by_card
        .iter()
        .map(|&(card_id, ..)| crate::handler::CardParameter {
            card_id,
            level: 1,
            param1: 10_000,
            param2: 10_000,
            param3: 10_000,
        })
        .collect();
    game.game_character_units = vec![
        crate::handler::GameCharacterUnit {
            game_character_id: 1,
            unit: "light_sound".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 5,
            unit: "idol".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 9,
            unit: "street".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 13,
            unit: "theme_park".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 17,
            unit: "school_refusal".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 21,
            unit: "piapro".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 22,
            unit: "piapro".to_string(),
        },
        crate::handler::GameCharacterUnit {
            game_character_id: 22,
            unit: "street".to_string(),
        },
    ];
    game.event_rarity_bonus_rates = vec![crate::handler::EventRarityBonusRate {
        event_id: 1,
        card_rarity_type: 4,
        master_rank: 5,
        bonus_rate_x10: 100,
    }];
    game.music_metas.push(crate::handler::MusicMeta {
        music_id: 1,
        difficulty: "expert".to_string(),
        event_rate_solo: 100,
        event_rate_multi: 100,
        event_rate_auto: 100,
        base_score: 1.0,
        base_score_auto: 1.0,
        fever_score: 0.0,
        solo_skill_scores: [0.0; 6],
        multi_skill_scores: [0.0; 6],
        auto_skill_scores: [0.0; 6],
        music_time: 100.0,
        tap_count: 100,
    });
    user.user_cards = char_by_card
        .iter()
        .map(|&(card_id, ..)| crate::handler::UserCard {
            card_id,
            level: 1,
            skill_level: 1,
            master_rank: 5,
            special_training_status: "none".to_string(),
            default_image: "original".to_string(),
            episodes_read: Vec::new(),
            is_virtual: false,
            has_canvas_bonus_override: None,
        })
        .collect();

    let game_ref = game.as_ref();
    let unit_of = |card_id: i32| -> &str {
        char_by_card
            .iter()
            .find(|&&(id, ..)| id == card_id)
            .unwrap()
            .2
    };

    for unit in [
        "light_sound",
        "idol",
        "street",
        "theme_park",
        "school_refusal",
        "piapro",
    ] {
        for cards in [
            [501, 502, 503, 504, 505],
            [501, 502, 503, 504, 507],
            [503, 505, 506, 507, 501],
        ] {
            let params_json = serde_json::json!({
                "region": "jp",
                "live_type": "multi",
                "music_id": 1,
                "music_diff": "expert",
                "event_me_unit": unit,
                "target": "power",
                "limit": 1,
                "fixed_cards": cards,
            })
            .to_string();
            let params = parse_build_params_json(&params_json).unwrap();
            assert_eq!(params.event_me_unit.as_deref(), Some(unit));
            let (pool, ctx) = crate::handler::build_card_pool(&user, &game_ref, &params).unwrap();
            let outcome = recommend(&user, &game_ref, &params).unwrap();
            let dense: [crate::pool::CardIdx; 5] = outcome.results[0]
                .cards
                .map(|id| pool.indices().find(|&c| pool.game_id(c) == id).unwrap());
            let summary = crate::search::summarize_deck(&pool, &ctx, &dense).unwrap();
            let mut actual_total = 0.0;
            for (pos, card_idx) in summary.ordered_cards.iter().enumerate() {
                let id = i32::from(pool.game_id(*card_idx));
                let expected = if unit_of(id) == unit { 85.0 } else { 35.0 };
                assert_eq!(
                    summary.card_event_bonus_rates[pos], expected,
                    "unit={unit} card={id}"
                );
                actual_total += expected;
            }
            assert_eq!(summary.event_bonus_total, Some(actual_total));
        }

        // MySekai simulation with event_me_unit also preserves unit bonuses.
        let mysekai_params = crate::handler::BuildParams {
            region: "jp".to_string(),
            live_type: crate::types::LiveType::Mysekai,
            target: crate::types::ScoreTarget::Score,
            event_me_unit: Some(unit.to_string()),
            fixed_cards: vec![501, 502, 503, 504, 507],
            limit: 1,
            ..Default::default()
        };
        let (pool, ctx) =
            crate::handler::build_card_pool(&user, &game_ref, &mysekai_params).unwrap();
        let outcome = recommend(&user, &game_ref, &mysekai_params).unwrap();
        let dense: [crate::pool::CardIdx; 5] = outcome.results[0]
            .cards
            .map(|id| pool.indices().find(|&c| pool.game_id(c) == id).unwrap());
        let summary = crate::search::summarize_deck(&pool, &ctx, &dense).unwrap();
        let expected_bonus: f64 = [501, 502, 503, 504, 507]
            .into_iter()
            .map(|id| if unit_of(id) == unit { 85.0 } else { 35.0 })
            .sum();
        assert_eq!(summary.event_bonus_total, Some(expected_bonus));
        let power_bonus_x10 = 10 + (summary.total_power as u64 * 10) / 450_000;
        let expected_mysekai =
            ((power_bonus_x10 as f64 * (100.0 + expected_bonus) * 500.0) / 1000.0) as u64;
        assert_eq!(outcome.results[0].score, expected_mysekai);
    }

    // Memory event bonus changes selection and event points for target=score.
    let score_params = crate::handler::BuildParams {
        region: "jp".to_string(),
        live_type: crate::types::LiveType::Multi,
        music_id: Some(1),
        music_diff: Some("expert".to_string()),
        event_me_unit: Some("street".to_string()),
        target: crate::types::ScoreTarget::Score,
        limit: 1,
        ..Default::default()
    };
    let (pool, ctx) = crate::handler::build_card_pool(&user, &game_ref, &score_params).unwrap();
    let outcome = recommend(&user, &game_ref, &score_params).unwrap();
    assert!(outcome.results[0].cards.contains(&503));
    let dense: [crate::pool::CardIdx; 5] = outcome.results[0]
        .cards
        .map(|id| pool.indices().find(|&c| pool.game_id(c) == id).unwrap());
    let summary = crate::search::summarize_deck(&pool, &ctx, &dense).unwrap();
    assert_eq!(summary.event_bonus_total, Some(225.0));
    let base = 110 + summary.live_score / 17_000 + ((4 * summary.live_score) / 340_000).min(16);
    assert_eq!(summary.event_point, Some((base as f64 * 3.25) as i32));

    // Reject conflicting selectors and invalid memory event unit.
    let conflicts = [
        serde_json::json!({"event_id": 1}),
        serde_json::json!({"event_attr": "cute"}),
        serde_json::json!({"event_unit": "idol"}),
        serde_json::json!({"world_bloom_event_turn": 1}),
        serde_json::json!({"world_bloom_finale_turn": 3}),
        serde_json::json!({"world_bloom_character_id": 9}),
        serde_json::json!({"event_type": "cheerful_carnival"}),
        serde_json::json!({"custom_bonus_attr": "cool"}),
        serde_json::json!({"custom_bonus_character_ids": [9]}),
        serde_json::json!({"live_type": "challenge"}),
        serde_json::json!({"live_type": "cheerful"}),
    ];
    for extra in conflicts {
        let mut base_obj = serde_json::json!({
            "region": "jp",
            "live_type": "multi",
            "music_id": 1,
            "music_diff": "expert",
            "event_me_unit": "street",
            "target": "power",
            "limit": 1,
        });
        for (k, v) in extra.as_object().unwrap() {
            base_obj
                .as_object_mut()
                .unwrap()
                .insert(k.clone(), v.clone());
        }
        let err = match parse_build_params_json(&base_obj.to_string()) {
            Ok(params) => recommend(&user, &game_ref, &params)
                .unwrap_err()
                .to_string(),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("event_me_unit requires"),
            "extra={extra} err={err}"
        );
    }

    assert!(
        parse_build_params_json(r#"{"event_me_unit":"invalid"}"#)
            .unwrap_err()
            .to_string()
            .contains("Invalid memory event unit")
    );
    let mut invalid_typed = score_params;
    invalid_typed.event_me_unit = Some("invalid".to_string());
    assert!(
        recommend(&user, &game_ref, &invalid_typed)
            .unwrap_err()
            .to_string()
            .contains("Invalid memory event unit")
    );
}

#[test]
fn region_parsing_inherits_default_when_blank_and_rejects_non_string() {
    assert_eq!(parse_build_params_json("{}").unwrap().region, "cn");
    assert_eq!(
        parse_build_params_json_with_default_region("{}", "jp")
            .unwrap()
            .region,
        "jp"
    );
    for raw in [
        r#"{"region":null}"#,
        r#"{"region":""}"#,
        r#"{"region":"   "}"#,
    ] {
        assert_eq!(parse_build_params_json(raw).unwrap().region, "cn");
        assert_eq!(
            parse_build_params_json_with_default_region(raw, "jp")
                .unwrap()
                .region,
            "jp"
        );
    }
    assert_eq!(
        parse_build_params_json_with_default_region(r#"{"region":" tw "}"#, "jp")
            .unwrap()
            .region,
        "tw"
    );
    for bad in [
        r#"{"region":123}"#,
        r#"{"region":true}"#,
        r#"{"region":["jp"]}"#,
        r#"{"region":{}}"#,
    ] {
        assert!(parse_build_params_json(bad).is_err(), "bad={bad}");
        assert!(
            parse_build_params_json_with_default_region(bad, "jp").is_err(),
            "bad={bad}"
        );
    }
    assert!(crate::types::is_memory_event_id(4_000_001));
    assert!(crate::types::is_memory_event_id(4_000_006));
    assert!(!crate::types::is_memory_event_id(4_000_000));
    assert!(!crate::types::is_memory_event_id(4_000_007));
}
