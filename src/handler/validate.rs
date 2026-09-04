//! 构建参数（`BuildParams`）合法性校验。

use std::collections::BTreeSet;

use super::BuildError;
use super::types::{self, parse_attr_code};

pub(crate) fn validate_build_params(params: &types::BuildParams) -> Result<(), BuildError> {
    let configs = [
        &params.card_configs.rarity_1_config,
        &params.card_configs.rarity_2_config,
        &params.card_configs.rarity_3_config,
        &params.card_configs.rarity_4_config,
        &params.card_configs.rarity_birthday_config,
    ]
    .into_iter()
    .chain(
        params
            .card_configs
            .single_card_configs
            .iter()
            .map(|entry| &entry.config),
    )
    .chain(params.single_card_configs.iter().map(|entry| &entry.config));
    for config in configs {
        if config.level.is_some_and(|value| value <= 0) {
            return Err(BuildError::InvalidConfig(
                "level must be positive".to_string(),
            ));
        }
        if config.skill_level.is_some_and(|value| value <= 0) {
            return Err(BuildError::InvalidConfig(
                "skillLevel must be positive".to_string(),
            ));
        }
        if config
            .master_rank
            .is_some_and(|value| !(0..=5).contains(&value))
        {
            return Err(BuildError::InvalidConfig(
                "masterRank must be in 0..=5".to_string(),
            ));
        }
        if config
            .episode_read_count
            .is_some_and(|value| !(0..=2).contains(&value))
        {
            return Err(BuildError::InvalidConfig(
                "episodeReadCount must be in 0..=2".to_string(),
            ));
        }
    }
    if params
        .forced_leader_character_id
        .is_some_and(|id| !(1..=26).contains(&id))
    {
        return Err(BuildError::InvalidConfig(
            "forcedLeaderCharacterId must be in 1..=26".to_string(),
        ));
    }
    if !(1..=types::MAX_BUILD_LIMIT).contains(&params.limit) {
        return Err(BuildError::InvalidConfig(format!(
            "limit 必须在 1..={} 范围内",
            types::MAX_BUILD_LIMIT
        )));
    }
    if !(1..=types::MAX_BUILD_TIMEOUT_MS).contains(&params.timeout_ms) {
        return Err(BuildError::InvalidConfig(format!(
            "timeout_ms 必须在 1..={} 范围内",
            types::MAX_BUILD_TIMEOUT_MS
        )));
    }
    if params
        .member
        .is_some_and(|member| member != crate::types::DECK_SIZE)
    {
        return Err(BuildError::InvalidConfig(format!(
            "member 仅支持 {}",
            crate::types::DECK_SIZE
        )));
    }
    if let Some(character_id) = params.challenge_live_character_id
        && !(1..=26).contains(&character_id)
    {
        return Err(BuildError::InvalidConfig(
            "challenge_live_character_id 需在 1..=26".to_string(),
        ));
    }
    if params.target_bonus_list.len() > types::MAX_TARGET_BONUS_BUCKETS {
        return Err(BuildError::InvalidConfig(format!(
            "target_bonus_list 最多支持 {} 个档位",
            types::MAX_TARGET_BONUS_BUCKETS
        )));
    }
    let mut bonus_targets = BTreeSet::new();
    for &bonus in &params.target_bonus_list {
        if !(0..=types::MAX_TARGET_BONUS).contains(&bonus) {
            return Err(BuildError::InvalidConfig(format!(
                "target bonus 必须在 0..={} 范围内",
                types::MAX_TARGET_BONUS
            )));
        }
        if !bonus_targets.insert(bonus) {
            return Err(BuildError::InvalidConfig(
                "target_bonus_list 不得包含重复档位".to_string(),
            ));
        }
    }
    if params.custom_bonus_character_ids.len() > 26 {
        return Err(BuildError::InvalidConfig(
            "custom bonus character 最多支持 26 项".to_string(),
        ));
    }
    let mut custom_characters = BTreeSet::new();
    if params
        .custom_bonus_character_ids
        .iter()
        .any(|id| !(1..=26).contains(id) || !custom_characters.insert(*id))
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus character id 非法或重复".to_string(),
        ));
    }
    if params
        .custom_bonus_attr
        .as_deref()
        .is_some_and(|attr| parse_attr_code(attr).is_none())
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus attr 非法".to_string(),
        ));
    }
    if params.custom_bonus_character_support_units.len() > 26 {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit 最多支持 26 项".to_string(),
        ));
    }
    let mut support_characters = BTreeSet::new();
    if params
        .custom_bonus_character_support_units
        .iter()
        .any(|entry| {
            !(1..=26).contains(&entry.character_id)
                || !support_characters.insert(entry.character_id)
                || !matches!(
                    entry.unit,
                    crate::types::Unit::LightSound
                        | crate::types::Unit::Idol
                        | crate::types::Unit::Street
                        | crate::types::Unit::Themepark
                        | crate::types::Unit::SchoolRefusal
                        | crate::types::Unit::Piapro
                )
        })
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit 非法或重复".to_string(),
        ));
    }
    if params
        .custom_bonus_character_support_units
        .iter()
        .any(|entry| !custom_characters.contains(&entry.character_id))
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit character 必须包含在 custom bonus character 中".to_string(),
        ));
    }
    if params.multi_live_score_up_lower_bound.is_some()
        && !matches!(params.live_type, crate::types::LiveType::Multi)
    {
        return Err(BuildError::InvalidConfig(
            "multi_live_score_up_lower_bound 仅支持 multi live".to_string(),
        ));
    }
    if !params.target_bonus_list.is_empty()
        && !matches!(params.target, crate::types::ScoreTarget::Bonus)
    {
        return Err(BuildError::InvalidConfig(
            "target_bonus_list 仅支持 bonus target".to_string(),
        ));
    }
    if matches!(
        params.live_skill_order,
        crate::types::LiveSkillOrder::Specific
    ) && params.specific_skill_order.is_none()
    {
        return Err(BuildError::InvalidConfig(
            "specific_skill_order 是 specific 策略的必填项".to_string(),
        ));
    }
    Ok(())
}
