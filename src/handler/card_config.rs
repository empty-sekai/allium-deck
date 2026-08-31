use super::types::{
    config_for_rarity, CardConfigSet, CardEpisode, CardRarity, CardRarityConfig, MasterCard,
    UserCard,
};

fn resolve_single_config(configs: &CardConfigSet, card_id: i32) -> Option<&CardRarityConfig> {
    configs
        .single_card_configs
        .iter()
        .find(|entry| entry.card_id == card_id)
        .map(|entry| &entry.config)
}

/// 应用 preset / 单卡覆盖配置。
pub(crate) fn apply_card_config(
    user_card: &mut UserCard,
    master: &MasterCard,
    configs: &CardConfigSet,
    rarities: &[CardRarity],
    episodes: &[CardEpisode],
) -> bool {
    let config = resolve_single_config(configs, master.id)
        .unwrap_or_else(|| config_for_rarity(configs, master.card_rarity_type));
    if config.disable {
        return false;
    }

    if config.level_max {
        if let Some(max_level) = master.max_level {
            user_card.level = max_level.max(user_card.level);
        }
        if super::build::card_can_special_train(master) {
            user_card.special_training_status = "done".to_string();
            user_card.default_image = "special_training".to_string();
        }
    }
    if let Some(level) = config.level {
        let max_level = master.max_level.unwrap_or(level);
        user_card.level = level.clamp(1, max_level);
        let normal_max_level = rarities
            .iter()
            .find(|rarity| rarity.card_rarity_type == master.card_rarity_type)
            .map(|rarity| rarity.normal_max_level)
            .unwrap_or(max_level);
        if super::build::card_can_special_train(master) && user_card.level > normal_max_level {
            user_card.special_training_status = "done".to_string();
            user_card.default_image = "special_training".to_string();
        } else {
            user_card.special_training_status = "not_doing".to_string();
            user_card.default_image = "original".to_string();
        }
    }
    if config.skill_max
        && let Some(max_skill_level) = master.max_skill_level {
            user_card.skill_level = max_skill_level.max(user_card.skill_level);
        }
    if let Some(skill_level) = config.skill_level {
        user_card.skill_level = skill_level.clamp(1, master.max_skill_level.unwrap_or(skill_level));
    }
    if config.master_max
        && let Some(max_master_rank) = master.max_master_rank {
            user_card.master_rank = max_master_rank.max(user_card.master_rank);
        }
    if let Some(master_rank) = config.master_rank {
        user_card.master_rank = master_rank.clamp(0, master.max_master_rank.unwrap_or(master_rank));
    }
    if config.episode_read || config.episode_read_count.is_some() {
        let count = config.episode_read_count.unwrap_or(2) as usize;
        let mut episode_ids = episodes
            .iter()
            .filter(|episode| episode.card_id == master.id)
            .map(|episode| episode.episode_no)
            .collect::<Vec<_>>();
        episode_ids.sort_unstable();
        episode_ids.truncate(count);
        user_card.episodes_read = episode_ids;
    }
    if config.canvas {
        user_card.has_canvas_bonus_override = Some(true);
    }

    true
}
