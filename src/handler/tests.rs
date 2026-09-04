//! handler 管线测试。

use crate::pool::EventBonusExact;
use crate::types::{DefaultImage, FINAL_CHAPTER_EVENT_ID};

use super::build::*;
use super::card_config::apply_card_config;
use super::filter::*;
use super::gather::{CardIntermediate, sort_and_gather};
use super::power::PreparedPowerContext;
use super::skill::{SkillResult, SkillState, build_skill, is_bfes_skill_pair};
use super::types;
use super::validate::validate_build_params;
use super::*;
use crate::pool::{RefSkill, SkillSlot};
use crate::types::{LiveType, ScoreTarget, SkillReferenceStrategy};

fn sample_game<'a>(
    cards: &'a [MasterCard],
    params: &'a [types::CardParameter],
    rarities: &'a [types::CardRarity],
    episodes: &'a [types::CardEpisode],
    lessons: &'a [types::MasterLesson],
    skills: &'a [types::Skill],
    effects: &'a [types::SkillEffect],
    area_items: &'a [types::AreaItemLevel],
    units: &'a [types::GameCharacterUnit],
) -> GameData<'a> {
    GameData {
        cards,
        card_parameters: params,
        card_rarities: rarities,
        card_episodes: episodes,
        master_lessons: lessons,
        skills,
        skill_effects: effects,
        area_item_levels: area_items,
        game_character_units: units,
        character_ranks: &[],
        card_mysekai_canvas_bonuses: &[],
        mysekai_gates: &[],
        mysekai_gate_levels: &[],
        events: &[],
        event_cards: &[],
        event_deck_bonuses: &[],
        event_card_bonus_limits: &[],
        event_honor_bonuses: &[],
        world_bloom_different_attribute_bonuses: &[],
        world_blooms: &[],
        wb_support_deck_bonuses_wl1: &[],
        wb_support_deck_bonuses_wl2: &[],
        wb_support_deck_bonuses_wl3: &[],
        world_bloom_support_deck_unit_event_limited_bonuses: &[],
        event_mysekai_fixture_performance_bonus_limits: &[],
        event_skill_score_up_limits: &[],
        music_metas: &[],
        music_difficulties: &[],
        event_rarity_bonus_rates: &[],
        honors: &[],
        bonds_honors: &[],
    }
}

fn sample_user_card(card_id: i32) -> UserCard {
    UserCard {
        card_id,
        level: 1,
        skill_level: 1,
        master_rank: 0,
        special_training_status: "none".to_string(),
        default_image: "original".to_string(),
        episodes_read: Vec::new(),
        is_virtual: false,
        has_canvas_bonus_override: None,
    }
}

#[test]
fn bonus_target_requires_non_final_event_context() {
    let game = sample_game(&[], &[], &[], &[], &[], &[], &[], &[], &[]);
    let user = UserProfile::default();

    let no_event = BuildParams {
        target: ScoreTarget::Bonus,
        ..BuildParams::default()
    };
    assert!(matches!(
        build_card_pool(&user, &game, &no_event),
        Err(BuildError::InvalidConfig(reason)) if reason.contains("活动")
    ));

    let final_chapter = BuildParams {
        target: ScoreTarget::Bonus,
        event_id: Some(crate::types::FINAL_CHAPTER_EVENT_ID),
        event_type: Some("world_bloom".to_string()),
        ..BuildParams::default()
    };
    assert!(matches!(
        build_card_pool(&user, &game, &final_chapter),
        Err(BuildError::InvalidConfig(reason)) if reason.contains("终章")
    ));
}

#[test]
fn programmatic_build_params_enforce_compatibility_bounds() {
    let game = sample_game(&[], &[], &[], &[], &[], &[], &[], &[], &[]);
    let user = UserProfile::default();

    for (params, expected) in [
        (
            BuildParams {
                limit: 0,
                ..BuildParams::default()
            },
            "limit",
        ),
        (
            BuildParams {
                timeout_ms: 0,
                ..BuildParams::default()
            },
            "timeout",
        ),
        (
            BuildParams {
                timeout_ms: 300_001,
                ..BuildParams::default()
            },
            "timeout",
        ),
        (
            BuildParams {
                target_bonus_list: vec![100; 33],
                ..BuildParams::default()
            },
            "target_bonus_list",
        ),
        (
            BuildParams {
                custom_bonus_character_ids: vec![0],
                ..BuildParams::default()
            },
            "character",
        ),
        (
            BuildParams {
                custom_bonus_character_ids: vec![1; 27],
                ..BuildParams::default()
            },
            "character",
        ),
        (
            BuildParams {
                custom_bonus_character_ids: vec![1, 1],
                ..BuildParams::default()
            },
            "重复",
        ),
        (
            BuildParams {
                custom_bonus_character_support_units: vec![
                    crate::types::CustomSupportUnit {
                        character_id: 21,
                        unit: crate::types::Unit::Idol,
                    },
                    crate::types::CustomSupportUnit {
                        character_id: 21,
                        unit: crate::types::Unit::Street,
                    },
                ],
                ..BuildParams::default()
            },
            "重复",
        ),
    ] {
        assert!(matches!(
            build_card_pool(&user, &game, &params),
            Err(BuildError::InvalidConfig(reason)) if reason.contains(expected)
        ));
    }
}

#[test]
fn exact_card_config_values_validate_their_public_ranges() {
    for config in [
        types::CardRarityConfig {
            level: Some(0),
            ..Default::default()
        },
        types::CardRarityConfig {
            skill_level: Some(0),
            ..Default::default()
        },
        types::CardRarityConfig {
            master_rank: Some(6),
            ..Default::default()
        },
        types::CardRarityConfig {
            episode_read_count: Some(3),
            ..Default::default()
        },
    ] {
        let mut params = BuildParams::default();
        params.card_configs.rarity_4_config = config;
        assert!(matches!(
            validate_build_params(&params),
            Err(BuildError::InvalidConfig(_))
        ));
    }
}

#[test]
fn boost_is_fire_count_piecewise_multiplier() {
    assert_eq!(normalize_boost_rate_pct(Some(0)), 100);
    assert_eq!(normalize_boost_rate_pct(Some(1)), 500);
    assert_eq!(normalize_boost_rate_pct(Some(5)), 2500);
    assert_eq!(normalize_boost_rate_pct(Some(10)), 3500);
    assert_eq!(normalize_boost_rate_pct(Some(11)), 100);
    assert_eq!(normalize_boost_rate_pct(None), 100);
}

#[test]
fn hard_unit_filter_keeps_virtual_singer_support_unit_only_when_matching() {
    let cards = [MasterCard {
        id: 1,
        character_id: 21,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "rarity_4".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: Some("light_sound".to_string()),
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let params = [types::CardParameter {
        card_id: 1,
        level: 1,
        param1: 100,
        param2: 100,
        param3: 100,
    }];
    let units = [types::GameCharacterUnit {
        game_character_id: 21,
        unit: "piapro".to_string(),
    }];
    let game = sample_game(&cards, &params, &[], &[], &[], &[], &[], &[], &units);
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        ..UserProfile::default()
    };

    let ln_params = BuildParams {
        unit_filter: Some("light_sound".to_string()),
        ..BuildParams::default()
    };
    let (pool, _) = build_card_pool(&user, &game, &ln_params).unwrap();
    assert_eq!(pool.count(), 1);

    let mmj_params = BuildParams {
        unit_filter: Some("idol".to_string()),
        ..BuildParams::default()
    };
    assert_eq!(
        build_card_pool(&user, &game, &mmj_params).unwrap_err(),
        BuildError::EmptyPool
    );
}

#[test]
fn handler_build_power_uses_f32_item_accumulation() {
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let params = [types::CardParameter {
        card_id: 1,
        level: 1,
        param1: 101,
        param2: 101,
        param3: 101,
    }];
    let area_items = [
        types::AreaItemLevel {
            area_item_id: 1,
            level: 1,
            unit: None,
            attr: None,
            character_id: None,
            power_rate: 1.0,
            power_all_match_rate: 1.0,
        },
        types::AreaItemLevel {
            area_item_id: 2,
            level: 1,
            unit: None,
            attr: None,
            character_id: None,
            power_rate: 1.0,
            power_all_match_rate: 1.0,
        },
    ];
    let game_units = [types::GameCharacterUnit {
        game_character_id: 1,
        unit: "idol".to_string(),
    }];
    let game = sample_game(
        &cards,
        &params,
        &[],
        &[],
        &[],
        &[],
        &[],
        &area_items,
        &game_units,
    );
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        user_area_items: vec![
            types::UserAreaItem {
                area_item_id: 1,
                level: 1,
            },
            types::UserAreaItem {
                area_item_id: 2,
                level: 1,
            },
        ],
        ..UserProfile::default()
    };

    let idx = index::PoolIndexes::build(&game);
    let power_ctx = PreparedPowerContext::new(&user, &game, &idx, None);
    let result = power::build_power(
        &sample_user_card(1),
        &cards[0],
        &power_ctx,
        &idx,
        idx.unit_mask(cards[0].id),
        idx.attr(cards[0].id).unwrap(),
    );
    let scalar = power::build_power_scalar_reference(
        &sample_user_card(1),
        &cards[0],
        &power_ctx,
        &idx,
        idx.unit_mask(cards[0].id),
        idx.attr(cards[0].id).unwrap(),
    );
    assert_eq!(result, scalar);
    assert_eq!(result.detail(1, 0).area_item_bonus, 6);
    assert_eq!(result.detail(1, 0).total, 309);
    assert_eq!(result.detail(0, 0), crate::types::PowerDetail::default());
    assert!(std::mem::size_of::<power::PowerResult>() <= 128);
}

#[test]
fn handler_build_card_pool_only_clamps_fixture_bonus_for_matching_event() {
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let params = [types::CardParameter {
        card_id: 1,
        level: 1,
        param1: 100,
        param2: 100,
        param3: 100,
    }];
    let skills = [types::Skill {
        id: 10,
        level: 1,
        is_after_training: false,
    }];
    let effects = [types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up".to_string(),
        value: 100,
        additional_value: None,
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let units = [types::GameCharacterUnit {
        game_character_id: 1,
        unit: "idol".to_string(),
    }];
    let events = [types::Event {
        id: FINAL_CHAPTER_EVENT_ID,
        event_type: "marathon".to_string(),
    }];
    let fixture_limits = [types::EventFixtureBonusLimit {
        event_id: FINAL_CHAPTER_EVENT_ID,
        bonus_rate_limit: 20,
    }];
    let game = GameData {
        cards: &cards,
        card_parameters: &params,
        card_rarities: &[],
        card_episodes: &[],
        master_lessons: &[],
        skills: &skills,
        skill_effects: &effects,
        area_item_levels: &[],
        game_character_units: &units,
        character_ranks: &[],
        card_mysekai_canvas_bonuses: &[],
        mysekai_gates: &[],
        mysekai_gate_levels: &[],
        events: &events,
        event_cards: &[],
        event_deck_bonuses: &[],
        event_card_bonus_limits: &[],
        event_honor_bonuses: &[],
        world_bloom_different_attribute_bonuses: &[],
        world_blooms: &[],
        wb_support_deck_bonuses_wl1: &[],
        wb_support_deck_bonuses_wl2: &[],
        wb_support_deck_bonuses_wl3: &[],
        world_bloom_support_deck_unit_event_limited_bonuses: &[],
        event_mysekai_fixture_performance_bonus_limits: &fixture_limits,
        event_skill_score_up_limits: &[],
        music_metas: &[],
        music_difficulties: &[],
        event_rarity_bonus_rates: &[],
        honors: &[],
        bonds_honors: &[],
    };
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        user_mysekai_fixture_bonuses: vec![types::UserFixtureBonus {
            character_id: 1,
            event_id: None,
            total_bonus_rate: 30,
        }],
        ..UserProfile::default()
    };

    let (pool, _) = build_card_pool(&user, &game, &BuildParams::default()).unwrap();
    let idx = pool.card_idx(0).unwrap();
    assert_eq!(pool.power_max(idx), 309);

    let params = BuildParams {
        event_id: Some(FINAL_CHAPTER_EVENT_ID),
        fixed_characters: vec![1],
        ..BuildParams::default()
    };
    let (pool, _) = build_card_pool(&user, &game, &params).unwrap();
    let idx = pool.card_idx(0).unwrap();
    assert_eq!(pool.power_max(idx), 306);
}

#[test]
fn handler_final_chapter_allows_auto_leader_without_fixed_character() {
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let params = [types::CardParameter {
        card_id: 1,
        level: 1,
        param1: 100,
        param2: 100,
        param3: 100,
    }];
    let skills = [types::Skill {
        id: 10,
        level: 1,
        is_after_training: false,
    }];
    let effects = [types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up".to_string(),
        value: 100,
        additional_value: None,
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let units = [types::GameCharacterUnit {
        game_character_id: 1,
        unit: "idol".to_string(),
    }];
    let events = [types::Event {
        id: FINAL_CHAPTER_EVENT_ID,
        event_type: "marathon".to_string(),
    }];
    let game = sample_game(
        &cards,
        &params,
        &[],
        &[],
        &[],
        &skills,
        &effects,
        &[],
        &units,
    );
    let game = GameData {
        events: &events,
        ..game
    };
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        ..UserProfile::default()
    };
    let params = BuildParams {
        event_id: Some(FINAL_CHAPTER_EVENT_ID),
        ..BuildParams::default()
    };

    let (_, ctx) = build_card_pool(&user, &game, &params)
        .expect("终章无固定队长应允许进入自动 leader 搜索路径");
    assert!(ctx.is_final_chapter);
    assert!(!ctx.has_fixed_leader());
}

#[test]
fn handler_build_skill_covers_normal_unit_count_diff_and_ref() {
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let game_units = [types::GameCharacterUnit {
        game_character_id: 1,
        unit: "idol".to_string(),
    }];
    let skills = [types::Skill {
        id: 10,
        level: 1,
        is_after_training: false,
    }];
    let empty_game = sample_game(&cards, &[], &[], &[], &[], &skills, &[], &[], &game_units);

    let normal_effects = [types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up".to_string(),
        value: 120,
        additional_value: None,
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let game = GameData {
        skill_effects: &normal_effects,
        ..empty_game
    };
    let idx = index::PoolIndexes::build(&game);
    let normal = build_skill(
        &sample_user_card(1),
        &cards[0],
        &game,
        &idx,
        0,
        Some(140),
        SkillState::BeforeTraining,
    );
    assert_eq!(
        normal.slot,
        SkillSlot {
            skill_type: 0,
            value: 120
        }
    );

    let unit_count_effects = [
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_unit_count".to_string(),
            value: 10,
            additional_value: None,
            unit_member_count: Some(1),
            unit: Some("idol".to_string()),
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_unit_count".to_string(),
            value: 50,
            additional_value: None,
            unit_member_count: Some(5),
            unit: Some("idol".to_string()),
            activate_character_rank: None,
        },
    ];
    let game = GameData {
        skill_effects: &unit_count_effects,
        ..empty_game
    };
    let idx = index::PoolIndexes::build(&game);
    let unit_count = build_skill(
        &sample_user_card(1),
        &cards[0],
        &game,
        &idx,
        0,
        None,
        SkillState::BeforeTraining,
    );
    assert_eq!(unit_count.slot.skill_type, 1);
    assert_eq!(
        unit_count
            .unit_count
            .as_ref()
            .map(|entry| entry.score_up[0]),
        Some(10)
    );
    assert_eq!(
        unit_count
            .unit_count
            .as_ref()
            .map(|entry| entry.score_up[4]),
        Some(50)
    );

    let diff_effects = [types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up_diff".to_string(),
        value: 20,
        additional_value: Some(5),
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let game = GameData {
        skill_effects: &diff_effects,
        ..empty_game
    };
    let idx = index::PoolIndexes::build(&game);
    let diff = build_skill(
        &sample_user_card(1),
        &cards[0],
        &game,
        &idx,
        0,
        None,
        SkillState::BeforeTraining,
    );
    assert_eq!(
        diff.diff,
        Some(crate::pool::DiffSkill {
            base: 20,
            increment: 5
        })
    );

    let ref_effects = [
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 100,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_reference".to_string(),
            value: 20,
            additional_value: Some(60),
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
    ];
    let game = GameData {
        skill_effects: &ref_effects,
        ..empty_game
    };
    let idx = index::PoolIndexes::build(&game);
    let ref_skill = build_skill(
        &sample_user_card(1),
        &cards[0],
        &game,
        &idx,
        0,
        Some(140),
        SkillState::BeforeTraining,
    );
    assert_eq!(
        ref_skill.ref_skill,
        Some(crate::pool::RefSkill { rate: 20, max: 40 })
    );
    assert_eq!(ref_skill.skill_max, 140);
}

#[test]
fn handler_build_card_pool_splits_bfes_reference_skill_cards() {
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: Some(11),
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let skills = [
        types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        },
        types::Skill {
            id: 11,
            level: 1,
            is_after_training: true,
        },
    ];
    let effects = [
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 80,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_reference".to_string(),
            value: 50,
            additional_value: Some(70),
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 11,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 120,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
    ];
    let units = [types::GameCharacterUnit {
        game_character_id: 1,
        unit: "idol".to_string(),
    }];
    let game = sample_game(&cards, &[], &[], &[], &[], &skills, &effects, &[], &units);
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        ..UserProfile::default()
    };
    let params = BuildParams {
        target: ScoreTarget::Skill,
        ..BuildParams::default()
    };

    let (pool, _, full) =
        build_card_pool_with_details(&user, &game, &params).expect("dual-skill pool should build");
    assert_eq!(pool.count(), 2);
    assert!(full.iter().all(|card| card.skill_state_controls_image));
    assert_eq!(
        pool.skill_max(pool.card_idx(1).expect("before skill entry")),
        120
    );
}

#[test]
fn specialized_unit_count_skill_pair_keeps_both_image_states() {
    let mut before = SkillResult::default();
    before.full.skill_id = 24;
    before.unit_count = Some(crate::pool::UnitCountSkill {
        unit: 1,
        score_up: [30, 60, 90, 120, 150],
    });
    let mut after = SkillResult::default();
    after.full.skill_id = 22;

    assert!(is_bfes_skill_pair(&before, &after));
}

#[test]
fn handler_apply_card_config_supports_override_and_disable() {
    let master = MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    };
    let mut user_card = sample_user_card(1);
    let mut configs = CardConfigSet::default();
    configs.rarity_4_config.level_max = true;
    configs.single_card_configs.push(types::SingleCardConfig {
        card_id: 1,
        config: types::CardRarityConfig {
            disable: true,
            ..types::CardRarityConfig::default()
        },
    });
    assert!(!apply_card_config(
        &mut user_card,
        &master,
        &configs,
        &[],
        &[],
    ));

    let mut user_card = sample_user_card(1);
    let mut configs = CardConfigSet::default();
    configs.rarity_4_config.level_max = true;
    configs.rarity_4_config.skill_max = true;
    configs.rarity_4_config.master_max = true;
    assert!(apply_card_config(
        &mut user_card,
        &master,
        &configs,
        &[],
        &[],
    ));
    assert_eq!(user_card.level, 60);
    assert_eq!(user_card.skill_level, 4);
    assert_eq!(user_card.master_rank, 5);
}

#[test]
fn handler_level_max_marks_trainable_cards_after_training() {
    let master = MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: Some(11),
        special_training_power1_bonus_fixed: 100,
        special_training_power2_bonus_fixed: 100,
        special_training_power3_bonus_fixed: 100,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    };
    let mut user_card = sample_user_card(1);
    user_card.level = 1;
    user_card.special_training_status = "not_doing".to_string();
    user_card.default_image = "original".to_string();
    let mut configs = CardConfigSet::default();
    configs.rarity_4_config.level_max = true;

    assert!(apply_card_config(
        &mut user_card,
        &master,
        &configs,
        &[types::CardRarity {
            card_rarity_type: 4,
            max_level: 60,
            normal_max_level: 50,
            max_skill_level: 4,
        }],
        &[],
    ));

    assert_eq!(user_card.level, 60);
    assert_eq!(user_card.special_training_status, "done");
    assert_eq!(user_card.default_image, "special_training");
}

#[test]
fn handler_exact_card_config_overrides_max_flags_and_uses_episode_ids() {
    let master = MasterCard {
        id: 7,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "rarity_4".to_string(),
        asset_bundle_name: "card_000007".to_string(),
        skill_id: 10,
        special_training_skill_id: Some(11),
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    };
    let rarities = [types::CardRarity {
        card_rarity_type: 4,
        max_level: 60,
        normal_max_level: 50,
        max_skill_level: 4,
    }];
    let episodes = [
        types::CardEpisode {
            card_id: 7,
            episode_no: 702,
            power1_bonus_fixed: 0,
            power2_bonus_fixed: 0,
            power3_bonus_fixed: 0,
        },
        types::CardEpisode {
            card_id: 7,
            episode_no: 701,
            power1_bonus_fixed: 0,
            power2_bonus_fixed: 0,
            power3_bonus_fixed: 0,
        },
    ];
    let mut configs = CardConfigSet {
        rarity_4_config: types::CardRarityConfig {
            level_max: true,
            level: Some(51),
            skill_max: true,
            skill_level: Some(2),
            master_max: true,
            master_rank: Some(3),
            episode_read: true,
            episode_read_count: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut user_card = sample_user_card(7);

    assert!(apply_card_config(
        &mut user_card,
        &master,
        &configs,
        &rarities,
        &episodes,
    ));
    assert_eq!(user_card.level, 51);
    assert_eq!(user_card.skill_level, 2);
    assert_eq!(user_card.master_rank, 3);
    assert_eq!(user_card.episodes_read, vec![701]);
    assert_eq!(user_card.special_training_status, "done");
    assert_eq!(user_card.default_image, "special_training");

    configs.rarity_4_config.level = Some(50);
    let mut user_card = sample_user_card(7);
    assert!(apply_card_config(
        &mut user_card,
        &master,
        &configs,
        &rarities,
        &episodes,
    ));
    assert_eq!(user_card.special_training_status, "not_doing");
    assert_eq!(user_card.default_image, "original");
}

#[test]
fn handler_cultivated_user_cards_matches_pool_cultivation() {
    // 渲染层养成卡况必须与建池同源：满级开关后 level 抬到 max，disable 的卡被剔除。
    let cards = [
        MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
        MasterCard {
            id: 2,
            character_id: 2,
            attr: "cute".to_string(),
            card_rarity_type: 1,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 11,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(20),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
    ];
    let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
    let user = UserProfile {
        user_cards: vec![sample_user_card(1), sample_user_card(2)],
        ..UserProfile::default()
    };

    // rarity_4 满级，rarity_1 禁用 → 卡1 level=60、卡2 被剔除。
    let mut params = BuildParams::default();
    params.card_configs.rarity_4_config.level_max = true;
    params.card_configs.rarity_1_config.disable = true;

    let cultivated = cultivated_user_cards(&user, &game, &params);
    assert_eq!(cultivated.len(), 1, "disabled 稀有度应被剔除");
    assert_eq!(cultivated[0].card_id, 1);
    assert_eq!(cultivated[0].level, 60, "满级开关应把 level 抬到 max_level");
}

#[test]
fn handler_cultivated_user_cards_canvas_sets_override() {
    // 画布开关应在养成卡况里置 has_canvas_bonus_override，渲染据此显示画布。
    let cards = [MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    }];
    let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
    let user = UserProfile {
        user_cards: vec![sample_user_card(1)],
        ..UserProfile::default()
    };
    let mut params = BuildParams::default();
    params.card_configs.rarity_4_config.canvas = true;

    let cultivated = cultivated_user_cards(&user, &game, &params);
    assert_eq!(cultivated.len(), 1);
    assert_eq!(cultivated[0].has_canvas_bonus_override, Some(true));
}

#[test]
fn handler_virtual_fixed_card_training_state_follows_master() {
    // 虚拟固定卡（用户未持有）的训练态应按 master.special_training_skill_id 判定：
    // 可特训卡 → done/special_training（否则渲染成花前、且漏掉特训固定 power 加成）；
    // 不可特训卡 → none/original。
    let cards = [
        MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: Some(11),
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
        MasterCard {
            id: 2,
            character_id: 2,
            attr: "cute".to_string(),
            card_rarity_type: 1,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 20,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(20),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
    ];
    let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
    // 用户一张都没有；两张都作为固定卡注入虚拟卡。
    let user = UserProfile::default();
    let params = BuildParams {
        fixed_cards: vec![1, 2],
        ..BuildParams::default()
    };

    let normalized = normalize_user_cards(&user, &params, &game);
    let trainable = normalized
        .iter()
        .find(|card| card.card_id == 1)
        .expect("可特训固定卡应存在");
    assert_eq!(trainable.special_training_status, "done");
    assert_eq!(trainable.default_image, "special_training");
    let untrainable = normalized
        .iter()
        .find(|card| card.card_id == 2)
        .expect("不可特训固定卡应存在");
    assert_eq!(untrainable.special_training_status, "none");
    assert_eq!(untrainable.default_image, "original");
}

#[test]
fn handler_sort_and_gather_reindexes_dense_order() {
    let card = |game_card_id: i32, power_max: i32| CardIntermediate {
        game_card_id,
        card_rarity_type: 4,
        character_id: game_card_id as u8,
        attr: 0,
        unit_mask_raw: 1,
        default_image: crate::types::DefaultImage::Original,
        after_training: false,
        skill_state_controls_image: false,
        master_rank: 0,
        skill_level: 1,
        has_char_bonus: false,
        has_attr_bonus: false,
        power: power::PowerResult {
            power_min: power_max - 10,
            power_max,
            ..power::PowerResult::default()
        },
        skill: skill::SkillResult {
            slot: SkillSlot::default(),
            unit_count: None,
            diff: None,
            ref_skill: None,
            skill_min: 1,
            skill_max: 2,
            full: crate::types::SkillInfo::default(),
        },
        event_bonus: EventBonusExact::from_whole(1, 1),
        leader_honor_bonus: 0,
        leader_limit_bonus: 0,
        ep_sort_key: power_max as i64,
    };
    let (pool, _, _) = sort_and_gather(
        vec![card(1, 100), card(3, 300), card(2, 200)],
        ScoreTarget::Power,
        false,
        LiveType::Solo,
        &[],
        &[],
        false,
    );
    assert_eq!(pool.count(), 3);
    assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), 3);
    assert_eq!(pool.game_id(pool.card_idx(1).unwrap()), 2);
    assert_eq!(pool.game_id(pool.card_idx(2).unwrap()), 1);
}

#[test]
fn handler_sort_and_gather_moves_fixed_card_states_before_members() {
    let card = |game_card_id: i32,
                character_id: u8,
                power_max: i32,
                skill_max: u8,
                default_image| {
        CardIntermediate {
            game_card_id,
            card_rarity_type: 4,
            character_id,
            attr: 0,
            unit_mask_raw: 1,
            default_image,
            after_training: matches!(default_image, crate::types::DefaultImage::SpecialTraining),
            skill_state_controls_image: false,
            master_rank: 0,
            skill_level: 1,
            has_char_bonus: false,
            has_attr_bonus: false,
            power: power::PowerResult {
                power_min: power_max - 10,
                power_max,
                ..power::PowerResult::default()
            },
            skill: skill::SkillResult {
                slot: SkillSlot::default(),
                unit_count: None,
                diff: None,
                ref_skill: if game_card_id == 949
                    && matches!(default_image, crate::types::DefaultImage::Original)
                {
                    Some(RefSkill { rate: 50, max: 70 })
                } else {
                    None
                },
                skill_min: skill_max,
                skill_max,
                full: crate::types::SkillInfo {
                    skill_id: if game_card_id == 949 {
                        if matches!(default_image, crate::types::DefaultImage::SpecialTraining) {
                            2
                        } else {
                            1
                        }
                    } else {
                        0
                    },
                    is_after_training: matches!(
                        default_image,
                        crate::types::DefaultImage::SpecialTraining
                    ),
                    has_ref: game_card_id == 949
                        && matches!(default_image, crate::types::DefaultImage::Original),
                    ..crate::types::SkillInfo::default()
                },
            },
            event_bonus: EventBonusExact::from_whole(1, 1),
            leader_honor_bonus: 0,
            leader_limit_bonus: 0,
            ep_sort_key: power_max as i64,
        }
    };
    let (pool, full, _) = sort_and_gather(
        vec![
            card(121, 26, 90_000, 110, crate::types::DefaultImage::Original),
            card(949, 17, 70_000, 150, crate::types::DefaultImage::Original),
            card(
                949,
                17,
                70_000,
                148,
                crate::types::DefaultImage::SpecialTraining,
            ),
            card(404, 21, 80_000, 120, crate::types::DefaultImage::Original),
        ],
        ScoreTarget::Score,
        true,
        LiveType::Multi,
        &[949],
        &[],
        true,
    );

    assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), 949);
    assert_eq!(pool.game_id(pool.card_idx(1).unwrap()), 949);
    assert_eq!(full[0].game_card_id, 949);
    assert_eq!(full[1].game_card_id, 949);
    assert!(matches!(
        full[0].default_image,
        crate::types::DefaultImage::SpecialTraining
    ));
    assert!(full[0].after_training);
    assert!(!full[1].after_training);
}

#[test]
fn handler_ordinary_trained_card_without_after_skill_uses_trained_art() {
    let mut user_card = sample_user_card(1);
    user_card.special_training_status = "done".to_string();
    user_card.default_image = "original".to_string();
    let master = MasterCard {
        id: 1,
        character_id: 1,
        attr: "cool".to_string(),
        card_rarity_type: 4,
        rarity: "rarity_4".to_string(),
        asset_bundle_name: "chara_000001".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 100,
        special_training_power2_bonus_fixed: 100,
        special_training_power3_bonus_fixed: 100,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    };

    assert_eq!(
        skill_states_for_card(
            DefaultImage::Original,
            is_after_training(&user_card.special_training_status),
            &master,
            &BuildParams::default(),
        ),
        ([SkillState::AfterTraining, SkillState::AfterTraining], 1)
    );
}

#[test]
fn handler_non_bfes_before_after_skill_states_collapse_to_best_skill() {
    let before = SkillResult {
        skill_min: 120,
        skill_max: 120,
        full: crate::types::SkillInfo {
            skill_id: 10,
            is_after_training: false,
            base_score_up: 120.0,
            ..crate::types::SkillInfo::default()
        },
        ..SkillResult::default()
    };
    let after = SkillResult {
        full: crate::types::SkillInfo {
            skill_id: 11,
            is_after_training: true,
            ..before.full
        },
        ..before.clone()
    };

    let mut collapsed = [
        Some((SkillState::AfterTraining, after)),
        Some((SkillState::BeforeTraining, before)),
    ];
    assert!(!collapse_non_bfes_skill_states(&mut collapsed, 2));
    let collapsed = collapsed.into_iter().flatten().collect::<Vec<_>>();

    assert_eq!(collapsed.len(), 1);
    assert!(matches!(collapsed[0].0, SkillState::BeforeTraining));
}

#[test]
fn handler_build_card_pool_end_to_end_minimal() {
    let cards = [
        MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
        MasterCard {
            id: 2,
            character_id: 2,
            attr: "cute".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 11,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
        MasterCard {
            id: 3,
            character_id: 3,
            attr: "happy".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 12,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        },
    ];
    let params_table = [
        types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        },
        types::CardParameter {
            card_id: 2,
            level: 1,
            param1: 110,
            param2: 110,
            param3: 110,
        },
        types::CardParameter {
            card_id: 3,
            level: 1,
            param1: 120,
            param2: 120,
            param3: 120,
        },
    ];
    let rarities = [types::CardRarity {
        card_rarity_type: 4,
        max_level: 60,
        normal_max_level: 50,
        max_skill_level: 4,
    }];
    let skills = [
        types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        },
        types::Skill {
            id: 11,
            level: 1,
            is_after_training: false,
        },
        types::Skill {
            id: 12,
            level: 1,
            is_after_training: false,
        },
    ];
    let effects = [
        types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 100,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 11,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 110,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
        types::SkillEffect {
            skill_id: 12,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 120,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        },
    ];
    let units = [
        types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        },
        types::GameCharacterUnit {
            game_character_id: 2,
            unit: "street".to_string(),
        },
        types::GameCharacterUnit {
            game_character_id: 3,
            unit: "themepark".to_string(),
        },
    ];
    let music = [types::MusicMeta {
        music_id: 99,
        difficulty: "master".to_string(),
        event_rate_solo: 100,
        event_rate_multi: 110,
        event_rate_auto: 90,
        base_score: 1.0,
        base_score_auto: 1.0,
        fever_score: 0.0,
        solo_skill_scores: [0.0; 6],
        multi_skill_scores: [0.0; 6],
        auto_skill_scores: [0.0; 6],
        music_time: 100.0,
        tap_count: 500,
    }];
    let game = GameData {
        cards: &cards,
        card_parameters: &params_table,
        card_rarities: &rarities,
        card_episodes: &[],
        master_lessons: &[],
        skills: &skills,
        skill_effects: &effects,
        area_item_levels: &[],
        game_character_units: &units,
        character_ranks: &[],
        card_mysekai_canvas_bonuses: &[],
        mysekai_gates: &[],
        mysekai_gate_levels: &[],
        events: &[],
        event_cards: &[],
        event_deck_bonuses: &[],
        event_card_bonus_limits: &[],
        event_honor_bonuses: &[],
        world_bloom_different_attribute_bonuses: &[],
        world_blooms: &[],
        wb_support_deck_bonuses_wl1: &[],
        wb_support_deck_bonuses_wl2: &[],
        wb_support_deck_bonuses_wl3: &[],
        world_bloom_support_deck_unit_event_limited_bonuses: &[],
        event_mysekai_fixture_performance_bonus_limits: &[],
        event_skill_score_up_limits: &[],
        music_metas: &music,
        music_difficulties: &[],
        event_rarity_bonus_rates: &[],
        honors: &[],
        bonds_honors: &[],
    };
    let user = UserProfile {
        user_cards: vec![
            sample_user_card(1),
            sample_user_card(2),
            sample_user_card(3),
        ],
        ..UserProfile::default()
    };
    let params = BuildParams {
        music_id: Some(99),
        live_type: LiveType::Solo,
        target: ScoreTarget::Score,
        skill_reference_strategy: SkillReferenceStrategy::Average,
        ..BuildParams::default()
    };

    let (pool, ctx, details) = build_card_pool_with_details(&user, &game, &params).unwrap();
    let shared_indexes = PreparedGameIndexes::new(&game);
    let prepared = PreparedGameData::with_indexes(game, &shared_indexes);
    let (prepared_pool, prepared_ctx, prepared_details) =
        build_card_pool_with_details_prepared(&user, &prepared, &params).unwrap();
    let prepared_build = PreparedPoolBuild::new(&user, &prepared, &params).unwrap();
    let (fully_prepared_pool, fully_prepared_ctx, fully_prepared_details) =
        build_card_pool_with_details_fully_prepared(&prepared, &prepared_build).unwrap();
    assert_eq!(prepared_pool.count(), pool.count());
    assert_eq!(prepared_ctx, ctx);
    assert_eq!(prepared_details, details);
    assert_eq!(fully_prepared_pool.count(), pool.count());
    assert_eq!(fully_prepared_ctx, ctx);
    assert_eq!(fully_prepared_details, details);
    assert_eq!(pool.count(), 3);
    assert_eq!(details.len(), pool.count());
    assert!(
        details
            .iter()
            .enumerate()
            .all(|(index, detail)| detail.game_card_id
                == pool.game_id(crate::pool::CardIdx::new(index as u16)))
    );
    assert_eq!(ctx.music_rate_pct, 100);
    assert_eq!(ctx.target, ScoreTarget::Score);
    assert_eq!(ctx.leader_honor_bonus.len(), 3);
}

fn make_card(
    game_card_id: i32,
    character_id: u8,
    power_max: i32,
    skill_max: u8,
) -> CardIntermediate {
    CardIntermediate {
        game_card_id,
        card_rarity_type: 4,
        character_id,
        attr: (character_id % 5),
        unit_mask_raw: 1u8 << (character_id % 6),
        default_image: crate::types::DefaultImage::Original,
        after_training: false,
        skill_state_controls_image: false,
        master_rank: 0,
        skill_level: 1,
        power: power::PowerResult {
            power_min: power_max - 10,
            power_max,
            ..power::PowerResult::default()
        },
        skill: skill::SkillResult {
            slot: SkillSlot::default(),
            unit_count: None,
            diff: None,
            ref_skill: None,
            skill_min: 1,
            skill_max,
            full: crate::types::SkillInfo::default(),
        },
        event_bonus: EventBonusExact::from_whole(0, 0),
        has_char_bonus: false,
        has_attr_bonus: false,
        leader_honor_bonus: 0,
        leader_limit_bonus: 0,
        ep_sort_key: power_max as i64,
    }
}

#[test]
fn handler_target_trim_power_keeps_top_per_character() {
    // 530 卡：26 角色各 ~20 张，power_max 从高到低排列
    let mut cards = Vec::new();
    for ch in 0..26u8 {
        for i in 0..21i32 {
            cards.push(make_card(
                (ch as i32) * 100 + i,
                ch,
                30000 - i * 100, // 第一张最高
                20,
            ));
        }
    }
    // 530 卡 < 512 容量
    assert!(cards.len() > 512);

    let params = BuildParams {
        target: ScoreTarget::Power,
        ..BuildParams::default()
    };
    target_per_character_trim(&mut cards, &params);

    // 每角色最多 10 张
    let mut chars_seen = [0u8; 27];
    for card in &cards {
        let ch = card.character_id as usize;
        chars_seen[ch] += 1;
    }
    for (ch, &count) in chars_seen.iter().enumerate() {
        assert!(
            count <= GENERAL_PER_CHAR_KEEP as u8,
            "角色 {ch} 有 {count} 张卡，超过上限"
        );
    }
    // 总计 ≤ 260，远小于 512
    assert!(cards.len() <= 260, "裁剪后仍有 {} 张", cards.len());

    // 每角色的最高 power 卡应被保留
    for ch in 0..26u8 {
        let best_id = (ch as i32) * 100; // 该角色第一张（最高 power）
        assert!(
            cards.iter().any(|c| c.game_card_id == best_id),
            "角色 {ch} 最高 power 卡 {best_id} 未被保留"
        );
    }
}

#[test]
fn handler_target_trim_skill_keeps_top_per_character() {
    let mut cards = Vec::new();
    for ch in 0..26u8 {
        for i in 0..21i32 {
            cards.push(make_card(
                (ch as i32) * 100 + i,
                ch,
                30000,
                100 - i as u8, // 第一张最高 skill
            ));
        }
    }
    assert!(cards.len() > 512);

    let params = BuildParams {
        target: ScoreTarget::Skill,
        ..BuildParams::default()
    };
    target_per_character_trim(&mut cards, &params);

    let mut chars_seen = [0u8; 27];
    for card in &cards {
        let ch = card.character_id as usize;
        chars_seen[ch] += 1;
    }
    for (ch, &count) in chars_seen.iter().enumerate() {
        assert!(
            count <= GENERAL_PER_CHAR_KEEP as u8,
            "角色 {ch} 有 {count} 张卡，超过上限"
        );
    }
    assert!(cards.len() <= 260, "裁剪后仍有 {} 张", cards.len());

    for ch in 0..26u8 {
        let best_id = (ch as i32) * 100;
        assert!(
            cards.iter().any(|c| c.game_card_id == best_id),
            "角色 {ch} 最高 skill 卡 {best_id} 未被保留"
        );
    }
}

#[test]
fn handler_target_trim_preserves_fixed_cards_and_characters() {
    let mut cards = Vec::new();
    for ch in 0..26u8 {
        for i in 0..21i32 {
            cards.push(make_card((ch as i32) * 100 + i, ch, 30000 - i * 100, 20));
        }
    }

    // fixed_card: 角色 5 的第 20 张卡（power 较低）
    let fixed_card_id: i32 = 5 * 100 + 20;
    let params = BuildParams {
        target: ScoreTarget::Power,
        fixed_cards: vec![fixed_card_id],
        ..BuildParams::default()
    };
    target_per_character_trim(&mut cards, &params);

    assert!(
        cards.iter().any(|c| c.game_card_id == fixed_card_id),
        "fixed_card 未被保留"
    );

    // 再测 fixed_characters
    let mut cards2 = Vec::new();
    for ch in 0..26u8 {
        for i in 0..21i32 {
            cards2.push(make_card((ch as i32) * 100 + i, ch, 30000 - i * 100, 20));
        }
    }
    let params2 = BuildParams {
        target: ScoreTarget::Power,
        fixed_characters: vec![3],
        ..BuildParams::default()
    };
    target_per_character_trim(&mut cards2, &params2);

    // 角色 3 的所有卡应被保留（21 张 > 10）
    let role3_count = cards2.iter().filter(|c| c.character_id == 3).count();
    assert_eq!(role3_count, 21, "fixed_character=3 的卡未全部保留");
}

#[test]
fn forced_leader_character_becomes_a_fixed_character_slot() {
    let cards = vec![
        make_card(100, 1, 30000, 20),
        make_card(200, 2, 29000, 20),
        make_card(300, 3, 28000, 20),
        make_card(400, 4, 27000, 20),
        make_card(500, 5, 26000, 20),
    ];

    // 固定 100（角色 1）+ 指定角色 3 当队长：角色 3 补一个固定角色槽。
    let params = BuildParams {
        fixed_cards: vec![100],
        forced_leader_character_id: Some(3),
        ..BuildParams::default()
    };
    let (fixed_card_ids, fixed_character_ids) =
        validate_fixed_constraints(&params, &cards).expect("固定约束应合法");
    assert_eq!(fixed_card_ids, vec![100u16]);
    assert_eq!(fixed_character_ids, vec![3u8]);

    // 队长角色已被固定卡覆盖时不重复占槽。
    let params = BuildParams {
        fixed_cards: vec![100],
        forced_leader_character_id: Some(1),
        ..BuildParams::default()
    };
    let (fixed_card_ids, fixed_character_ids) =
        validate_fixed_constraints(&params, &cards).expect("固定约束应合法");
    assert_eq!(fixed_card_ids, vec![100u16]);
    assert!(fixed_character_ids.is_empty());

    // 队长角色已在 fixed_characters 里时同样不重复占槽。
    let params = BuildParams {
        fixed_characters: vec![3],
        forced_leader_character_id: Some(3),
        ..BuildParams::default()
    };
    let (_, fixed_character_ids) =
        validate_fixed_constraints(&params, &cards).expect("固定约束应合法");
    assert_eq!(fixed_character_ids, vec![3u8]);
}

#[test]
fn forced_leader_character_rejects_a_full_fixed_deck_and_an_absent_character() {
    let cards = vec![
        make_card(100, 1, 30000, 20),
        make_card(200, 2, 29000, 20),
        make_card(300, 3, 28000, 20),
        make_card(400, 4, 27000, 20),
        make_card(500, 5, 26000, 20),
    ];

    // 五个槽位已被固定卡占满，再指定一个新角色当队长无处安放。
    let params = BuildParams {
        fixed_cards: vec![100, 200, 300, 400, 500],
        forced_leader_character_id: Some(6),
        ..BuildParams::default()
    };
    assert!(validate_fixed_constraints(&params, &cards).is_err());

    // 卡池里没有该角色的卡。
    let params = BuildParams {
        forced_leader_character_id: Some(6),
        ..BuildParams::default()
    };
    assert!(validate_fixed_constraints(&params, &cards).is_err());
}

#[test]
fn forced_leader_character_is_dropped_for_challenge_live() {
    let cards = vec![make_card(100, 1, 30000, 20), make_card(101, 1, 29000, 20)];
    let params = BuildParams {
        live_type: LiveType::Challenge,
        forced_leader_character_id: Some(3),
        ..BuildParams::default()
    };
    let (_, fixed_character_ids) =
        validate_fixed_constraints(&params, &cards).expect("挑战 live 应忽略队长约束");
    assert!(fixed_character_ids.is_empty());
}

/// 造 n 张 4 星卡（角色 1..n），供指定队长的端到端建池测试使用。
fn leader_master_cards(count: i32) -> Vec<MasterCard> {
    (1..=count)
        .map(|id| MasterCard {
            id,
            character_id: id,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: String::new(),
            asset_bundle_name: format!("chara_{id:06}"),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        })
        .collect()
}

#[test]
fn handler_forced_leader_reaches_the_search_context_for_a_normal_event() {
    // 回归点：forced_leader_character_id 曾只在 WL 终章写进 SearchContext，
    // 其余活动一律丢弃，该参数因此对普通活动完全无效。
    let cards = leader_master_cards(6);
    let card_params = (1..=6)
        .map(|card_id| types::CardParameter {
            card_id,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        })
        .collect::<Vec<_>>();
    let skills = [types::Skill {
        id: 10,
        level: 1,
        is_after_training: false,
    }];
    let effects = [types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up".to_string(),
        value: 100,
        additional_value: None,
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let units = (1..=6)
        .map(|id| types::GameCharacterUnit {
            game_character_id: id,
            unit: "idol".to_string(),
        })
        .collect::<Vec<_>>();
    let events = [types::Event {
        id: 42,
        event_type: "marathon".to_string(),
    }];
    let game = sample_game(
        &cards,
        &card_params,
        &[],
        &[],
        &[],
        &skills,
        &effects,
        &[],
        &units,
    );
    let game = GameData {
        events: &events,
        ..game
    };
    let user = UserProfile {
        user_cards: (1..=6).map(sample_user_card).collect(),
        ..UserProfile::default()
    };

    let params = BuildParams {
        event_id: Some(42),
        live_type: LiveType::Multi,
        target: ScoreTarget::Score,
        forced_leader_character_id: Some(3),
        ..BuildParams::default()
    };
    let (_, ctx) = build_card_pool(&user, &game, &params).expect("普通活动应能建池");
    assert!(!ctx.is_final_chapter);
    assert_eq!(ctx.forced_leader_character_id, Some(3));
    assert_eq!(ctx.fixed_character_ids, vec![3u8]);
    assert!(
        !ctx.effective_best_skill_as_leader(),
        "指定队长时不得再让最高技能抢队长位",
    );

    // 不指定队长时上下文保持原样。
    let params = BuildParams {
        event_id: Some(42),
        live_type: LiveType::Multi,
        target: ScoreTarget::Score,
        ..BuildParams::default()
    };
    let (_, ctx) = build_card_pool(&user, &game, &params).expect("建池");
    assert_eq!(ctx.forced_leader_character_id, None);
    assert!(ctx.fixed_character_ids.is_empty());
    assert!(ctx.effective_best_skill_as_leader());
}

#[test]
fn handler_build_power_large_pool_does_not_error() {
    // 模拟大账号：26 角色各 25 张卡 = 650 张 > 512
    let mut master_cards = Vec::new();
    let mut card_params = Vec::new();
    let mut skills = Vec::new();
    let mut effects = Vec::new();
    let mut units = Vec::new();
    let mut user_cards = Vec::new();

    for ch in 1i32..=26i32 {
        for i in 0i32..25i32 {
            let card_id = ch * 100 + i;
            master_cards.push(MasterCard {
                id: card_id,
                character_id: ch,
                attr: "cool".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: card_id * 10,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            });
            card_params.push(types::CardParameter {
                card_id,
                level: 1,
                param1: 100 + i,
                param2: 100,
                param3: 100,
            });
            skills.push(types::Skill {
                id: card_id * 10,
                level: 1,
                is_after_training: false,
            });
            effects.push(types::SkillEffect {
                skill_id: card_id * 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 100,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            });
            units.push(types::GameCharacterUnit {
                game_character_id: ch,
                unit: "idol".to_string(),
            });
            user_cards.push(sample_user_card(card_id));
        }
    }
    let rarities = [types::CardRarity {
        card_rarity_type: 4,
        max_level: 60,
        normal_max_level: 50,
        max_skill_level: 4,
    }];
    let game = GameData {
        cards: &master_cards,
        card_parameters: &card_params,
        card_rarities: &rarities,
        card_episodes: &[],
        master_lessons: &[],
        skills: &skills,
        skill_effects: &effects,
        area_item_levels: &[],
        game_character_units: &units,
        character_ranks: &[],
        card_mysekai_canvas_bonuses: &[],
        mysekai_gates: &[],
        mysekai_gate_levels: &[],
        events: &[],
        event_cards: &[],
        event_deck_bonuses: &[],
        event_card_bonus_limits: &[],
        event_honor_bonuses: &[],
        world_bloom_different_attribute_bonuses: &[],
        world_blooms: &[],
        wb_support_deck_bonuses_wl1: &[],
        wb_support_deck_bonuses_wl2: &[],
        wb_support_deck_bonuses_wl3: &[],
        world_bloom_support_deck_unit_event_limited_bonuses: &[],
        event_mysekai_fixture_performance_bonus_limits: &[],
        event_skill_score_up_limits: &[],
        music_metas: &[],
        music_difficulties: &[],
        event_rarity_bonus_rates: &[],
        honors: &[],
        bonds_honors: &[],
    };
    let user = UserProfile {
        user_cards,
        ..UserProfile::default()
    };
    let params = BuildParams {
        target: ScoreTarget::Power,
        ..BuildParams::default()
    };

    let result = build_card_pool(&user, &game, &params);
    assert!(result.is_ok(), "大卡池 Power 构建应成功，实际: {result:?}");
    let (pool, _) = result.unwrap();
    assert!(pool.count() <= 512, "池子大小应 ≤ 512");
}

/// 精确档位组卡的合成上下文：马拉松活动 + 稀有度×突破加成表 + 可选的
/// 属性轴 deck bonus。所有卡默认互不命中队伍/属性规则。
struct BonusTierFixture {
    master_cards: Vec<MasterCard>,
    card_params: Vec<types::CardParameter>,
    skills: Vec<types::Skill>,
    effects: Vec<types::SkillEffect>,
    units: Vec<types::GameCharacterUnit>,
    events: Vec<types::Event>,
    deck_bonuses: Vec<types::EventDeckBonus>,
    rarity_rates: Vec<types::EventRarityBonusRate>,
    music_metas: Vec<MusicMeta>,
}

fn bonus_tier_fixture() -> BonusTierFixture {
    let mut master_cards = Vec::new();
    let mut card_params = Vec::new();
    // 60 张无关 4★（0破，加成 10%）：让池子规模超过 EP_PREFILTER_MIN_POOL，
    // EP 预过滤在该活动下会真实触发。
    for id in 1..=60i32 {
        let character_id = (id - 1) % 26 + 1;
        master_cards.push(MasterCard {
            id,
            character_id,
            attr: "cute".to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: format!("chara_{id:06}"),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        });
        card_params.push(types::CardParameter {
            card_id: id,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        });
    }
    // 档位 33% 的骨架卡：4★0破(10) + 4★4破(20) + 三张 2★5破(1)。
    for (id, character_id, rarity, attr) in [
        (101, 5, 4, "cute"),
        (102, 6, 4, "cute"),
        (103, 7, 2, "cute"),
        (104, 8, 2, "cute"),
        (105, 9, 2, "cute"),
    ] {
        master_cards.push(MasterCard {
            id,
            character_id,
            attr: attr.to_string(),
            card_rarity_type: rarity,
            rarity: format!("rarity_{rarity}"),
            asset_bundle_name: format!("chara_{id:06}"),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        });
        card_params.push(types::CardParameter {
            card_id: id,
            level: 1,
            param1: 120,
            param2: 100,
            param3: 100,
        });
    }
    let skills = vec![types::Skill {
        id: 10,
        level: 1,
        is_after_training: false,
    }];
    let effects = vec![types::SkillEffect {
        skill_id: 10,
        skill_level: 1,
        effect_type: "score_up".to_string(),
        value: 100,
        additional_value: None,
        unit_member_count: None,
        unit: None,
        activate_character_rank: None,
    }];
    let units = (1..=26)
        .map(|character_id| types::GameCharacterUnit {
            game_character_id: character_id,
            unit: "idol".to_string(),
        })
        .collect();
    let events = vec![types::Event {
        id: 42,
        event_type: "marathon".to_string(),
    }];
    let deck_bonuses = vec![types::EventDeckBonus {
        event_id: 42,
        character_id: None,
        unit: None,
        attr: Some("mysterious".to_string()),
        bonus_rate: 25,
    }];
    let rarity_rates = vec![
        types::EventRarityBonusRate {
            event_id: 42,
            card_rarity_type: 4,
            master_rank: 0,
            bonus_rate_x10: 100,
        },
        types::EventRarityBonusRate {
            event_id: 42,
            card_rarity_type: 4,
            master_rank: 4,
            bonus_rate_x10: 200,
        },
        types::EventRarityBonusRate {
            event_id: 42,
            card_rarity_type: 2,
            master_rank: 5,
            bonus_rate_x10: 10,
        },
    ];
    let music_metas = vec![MusicMeta {
        music_id: 7,
        difficulty: "expert".to_string(),
        event_rate_solo: 100,
        event_rate_multi: 100,
        event_rate_auto: 100,
        base_score: 100.0,
        base_score_auto: 100.0,
        fever_score: 0.0,
        solo_skill_scores: [0.0; 6],
        multi_skill_scores: [0.0; 6],
        auto_skill_scores: [0.0; 6],
        music_time: 100.0,
        tap_count: 500,
    }];
    BonusTierFixture {
        master_cards,
        card_params,
        skills,
        effects,
        units,
        events,
        deck_bonuses,
        rarity_rates,
        music_metas,
    }
}

fn bonus_tier_game<'a>(fixture: &'a BonusTierFixture) -> GameData<'a> {
    let game = sample_game(
        &fixture.master_cards,
        &fixture.card_params,
        &[],
        &[],
        &[],
        &fixture.skills,
        &fixture.effects,
        &[],
        &fixture.units,
    );
    GameData {
        events: &fixture.events,
        event_deck_bonuses: &fixture.deck_bonuses,
        event_rarity_bonus_rates: &fixture.rarity_rates,
        music_metas: &fixture.music_metas,
        ..game
    }
}

fn user_card_with_rank(card_id: i32, master_rank: i32) -> UserCard {
    UserCard {
        master_rank,
        ..sample_user_card(card_id)
    }
}

fn bonus_tier_params(tiers: &[i32]) -> BuildParams {
    BuildParams {
        target: ScoreTarget::Bonus,
        event_id: Some(42),
        live_type: LiveType::Multi,
        music_id: Some(7),
        music_diff: Some("expert".to_string()),
        target_bonus_list: tiers.to_vec(),
        ..BuildParams::default()
    }
}

#[test]
fn bonus_tier_pool_keeps_master_rank_bonus_cards_and_hits_exact_tiers() {
    // 回归点：EP 预过滤要求低星卡「加成>0 且角色+属性双轴命中」，2★5破的
    // 1% 突破加成卡因此全部出局，池内单卡下限 10%、卡组下限 50%，25%~33%
    // 这类零头档位永远不可达。档位组卡不应触发该预过滤。
    let fixture = bonus_tier_fixture();
    let game = bonus_tier_game(&fixture);
    let mut user_cards: Vec<UserCard> = (1..=60).map(sample_user_card).collect();
    user_cards.push(user_card_with_rank(101, 0));
    user_cards.push(user_card_with_rank(102, 4));
    user_cards.push(user_card_with_rank(103, 5));
    user_cards.push(user_card_with_rank(104, 5));
    user_cards.push(user_card_with_rank(105, 5));
    let user = UserProfile {
        user_cards,
        ..UserProfile::default()
    };

    let (pool, ctx) =
        build_card_pool(&user, &game, &bonus_tier_params(&[33])).expect("档位组卡应能建池");
    let bonuses: Vec<u32> = (0..pool.count())
        .map(|index| {
            pool.event_bonus(crate::pool::CardIdx::new(index as u16))
                .total_x10() as u32
        })
        .collect();
    assert!(
        bonuses.contains(&10),
        "1% 突破加成卡必须进池，实际池内加成种类: {:?}",
        bonuses
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
    );

    let decks = crate::search::search_targets(
        &pool,
        &ctx,
        &crate::search::SearchParams {
            top_k: 3,
            timeout_ms: 10_000,
        },
        &[33],
    );
    assert!(!decks.is_empty(), "33 档应能组出卡组");
    for deck in &decks {
        let total_x10: u32 = deck
            .cards
            .iter()
            .map(|card| pool.event_bonus(*card).total_x10() as u32)
            .sum();
        assert_eq!(total_x10, 330, "命中档位的卡组加成总和应为 33%");
    }
}

#[test]
fn bonus_tier_pool_drops_cards_above_the_highest_target() {
    // 超过最高档位的卡不可能出现在任何命中卡组里（加成非负、总和单调），
    // 专属路径在综合力计算前把它们收掉；4★5破+属性轴命中 = 50% > 33%。
    let mut fixture = bonus_tier_fixture();
    fixture.master_cards.push(MasterCard {
        id: 201,
        character_id: 10,
        attr: "mysterious".to_string(),
        card_rarity_type: 4,
        rarity: "rarity_4".to_string(),
        asset_bundle_name: "chara_002010".to_string(),
        skill_id: 10,
        special_training_skill_id: None,
        special_training_power1_bonus_fixed: 0,
        special_training_power2_bonus_fixed: 0,
        special_training_power3_bonus_fixed: 0,
        support_unit: None,
        max_level: Some(60),
        max_skill_level: Some(4),
        max_master_rank: Some(5),
    });
    fixture.card_params.push(types::CardParameter {
        card_id: 201,
        level: 1,
        param1: 200,
        param2: 100,
        param3: 100,
    });
    let game = bonus_tier_game(&fixture);
    let mut user_cards: Vec<UserCard> = (1..=60).map(sample_user_card).collect();
    user_cards.push(user_card_with_rank(101, 0));
    user_cards.push(user_card_with_rank(102, 4));
    user_cards.push(user_card_with_rank(103, 5));
    user_cards.push(user_card_with_rank(104, 5));
    user_cards.push(user_card_with_rank(105, 5));
    user_cards.push(user_card_with_rank(201, 5));
    let user = UserProfile {
        user_cards,
        ..UserProfile::default()
    };

    let (pool, _) =
        build_card_pool(&user, &game, &bonus_tier_params(&[33])).expect("档位组卡应能建池");
    let over_target = (0..pool.count())
        .filter(|index| pool.game_id(crate::pool::CardIdx::new(*index as u16)) == 201)
        .count();
    assert_eq!(over_target, 0, "超过最高档位的卡不应进池");
}
