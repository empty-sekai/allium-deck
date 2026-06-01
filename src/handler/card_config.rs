use super::types::{config_for_rarity, CardConfigSet, CardRarityConfig, MasterCard, UserCard};

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
    }
    if config.skill_max {
        if let Some(max_skill_level) = master.max_skill_level {
            user_card.skill_level = max_skill_level.max(user_card.skill_level);
        }
    }
    if config.master_max {
        if let Some(max_master_rank) = master.max_master_rank {
            user_card.master_rank = max_master_rank.max(user_card.master_rank);
        }
    }
    if config.episode_read && user_card.episodes_read.is_empty() {
        user_card.episodes_read = vec![1, 2];
    }
    if config.canvas {
        user_card.has_canvas_bonus_override = Some(true);
    }

    true
}
