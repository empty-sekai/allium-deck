use std::collections::BTreeSet;

use allium_deck::handler::{
    BuildParams, UserAreaItem, UserCard, UserCharacter, UserFixtureBonus, UserGateBonus, UserHonor,
    UserProfile,
};
use allium_deck::search::SearchParams;
use allium_deck::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};

use super::legacy_types::{LegacyInput, LegacyUserCard, LegacyUserData};

/// 将旧 input JSON 转为 `BuildParams`、`UserProfile` 和 `SearchParams`。
pub fn transform_input(
    input: &LegacyInput,
) -> Result<(BuildParams, UserProfile, SearchParams), String> {
    let legacy_user: LegacyUserData = serde_json::from_str(&input.user_data_str)
        .map_err(|err| format!("user_data_str 解析失败: {err}"))?;

    let build = BuildParams {
        region: input.region.clone(),
        event_id: input.event_id,
        music_id: input.music_id,
        music_diff: input.music_diff.clone(),
        target: parse_target(&input.target)?,
        target_bonus_list: input.target_bonus_list.clone().unwrap_or_default(),
        live_type: parse_live_type(&input.live_type)?,
        skill_reference_strategy: SkillReferenceStrategy::Average,
        live_skill_order: LiveSkillOrder::Average,
        ..BuildParams::default()
    };

    let user = UserProfile {
        user_cards: legacy_user
            .user_cards
            .iter()
            .map(transform_user_card)
            .collect(),
        user_characters: legacy_user
            .user_characters
            .iter()
            .map(|entry| UserCharacter {
                character_id: entry.character_id,
                character_rank: entry.character_rank,
            })
            .collect(),
        user_area_items: legacy_user
            .user_areas
            .iter()
            .flat_map(|area| area.area_items.iter())
            .map(|item| UserAreaItem {
                area_item_id: item.area_item_id,
                level: item.level,
            })
            .collect(),
        user_decks: Vec::new(),
        user_world_bloom_support_decks: Vec::new(),
        user_challenge_live_solo_decks: Vec::new(),
        user_mysekai_fixture_bonuses: legacy_user
            .user_mysekai_fixture_game_character_performance_bonuses
            .iter()
            .map(|entry| UserFixtureBonus {
                character_id: entry.game_character_id,
                event_id: None,
                total_bonus_rate: entry.total_bonus_rate,
            })
            .collect(),
        user_mysekai_gate_bonuses: legacy_user
            .user_mysekai_gates
            .iter()
            .map(|entry| UserGateBonus {
                unit: gate_unit(entry.mysekai_gate_id).to_string(),
                bonus_rate: (entry.mysekai_gate_level.max(0) as f64) * 0.1,
            })
            .collect(),
        user_mysekai_canvas_bonus_cards: legacy_user
            .user_mysekai_canvases
            .iter()
            .map(|entry| entry.card_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        user_honors: legacy_user
            .user_honors
            .iter()
            .map(|entry| UserHonor {
                honor_id: entry.honor_id,
                level: entry.level,
            })
            .collect(),
    };

    let search = SearchParams {
        top_k: input.limit,
        timeout_ms: input.timeout_ms,
    };

    Ok((build, user, search))
}

fn gate_unit(gate_id: i32) -> &'static str {
    match gate_id {
        1 => "light_sound",
        2 => "idol",
        3 => "street",
        4 => "theme_park",
        5 => "school_refusal",
        _ => "piapro",
    }
}

fn transform_user_card(card: &LegacyUserCard) -> UserCard {
    UserCard {
        card_id: card.card_id,
        level: card.level,
        skill_level: card.skill_level,
        master_rank: card.master_rank,
        special_training_status: card.special_training_status.clone(),
        default_image: card.default_image.clone(),
        episodes_read: card
            .episodes
            .iter()
            .filter(|episode| episode.scenario_status == "already_read")
            .map(|episode| episode.card_episode_id)
            .collect(),
        is_virtual: false,
        has_canvas_bonus_override: None,
    }
}

fn parse_target(value: &str) -> Result<ScoreTarget, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "score" => Ok(ScoreTarget::Score),
        "power" => Ok(ScoreTarget::Power),
        "skill" => Ok(ScoreTarget::Skill),
        "bonus" => Ok(ScoreTarget::Bonus),
        "mysekai" => Ok(ScoreTarget::Mysekai),
        other => Err(format!("未知 target: {other}")),
    }
}

fn parse_live_type(value: &str) -> Result<LiveType, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "solo" => Ok(LiveType::Solo),
        "auto" => Ok(LiveType::Auto),
        "multi" => Ok(LiveType::Multi),
        "challenge" => Ok(LiveType::Challenge),
        "challenge_auto" => Ok(LiveType::ChallengeAuto),
        "mysekai" => Ok(LiveType::Mysekai),
        other => Err(format!("未知 live_type: {other}")),
    }
}
