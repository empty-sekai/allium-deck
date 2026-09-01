//! 建池查表索引（P3 性能优化）。
//!
//! `build_card_pool` 对每张用户卡都要在 masterdata 大表里按 id/rarity 线性 `find/filter`，
//! 其中 `card_parameters`（卡×等级，数万行）和 `area_item_levels` 是主要热点，
//! 导致大账号建池 ~90ms。本模块在建池开始时对这些表建一次按键索引，
//! 把每卡查表从 O(表行数) 降到 O(该卡相关行数)。
//!
//! 索引借用大表，并将等级 power 与技能 effect 编译成紧凑数值表；生命周期与 `GameData` 一致。
//! 改动是纯性能重构，查询结果与原线性扫描等价，因此组卡输出必须逐字节不变。

use std::collections::HashMap;

use crate::simd::PowerAreaItem;

use super::types::{
    CardEpisode, CardMysekaiCanvasBonus, CardParameter, GameData, MasterCard, MasterLesson, Skill,
    attr_to_pool_index, parse_attr_code, parse_unit_code, unit_to_pool_index,
};

pub(crate) struct PreparedCardIndex {
    pub(crate) master: MasterCard,
    base_power_by_level: Vec<[i32; 3]>,
    pub(crate) unit_mask: u8,
    pub(crate) attr: Option<u8>,
    pub(crate) primary_unit: Option<crate::types::Unit>,
    pub(crate) support_unit: Option<crate::types::Unit>,
    pub(crate) support_unit_unrestricted: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum PreparedSkillEffectKind {
    ScoreUp,
    LifeRecovery,
    CharacterRank,
    UnitCount,
    Diff,
    Reference,
    Other,
}

#[derive(Clone, Copy)]
pub(crate) struct PreparedSkillEffect {
    pub(crate) kind: PreparedSkillEffectKind,
    pub(crate) value: i32,
    pub(crate) additional_value: Option<i32>,
    pub(crate) unit_member_count: Option<i32>,
    pub(crate) unit: Option<crate::types::Unit>,
    pub(crate) activate_character_rank: Option<i32>,
}

/// 建池期只读查表索引。
pub(crate) struct PoolIndexes {
    card_by_id: HashMap<i32, PreparedCardIndex>,
    episodes_by_card: HashMap<i32, Vec<CardEpisode>>,
    lessons_by_rarity: HashMap<i32, Vec<MasterLesson>>,
    canvas_by_rarity: HashMap<i32, CardMysekaiCanvasBonus>,
    area_by_item_level: HashMap<(i32, i32), Vec<PowerAreaItem>>,
    skill_by_id_level: HashMap<(i32, i32), Skill>,
    effects_by_skill_level: HashMap<(i32, i32), Vec<PreparedSkillEffect>>,
    honor_bonus_by_id_level: HashMap<(i32, i32), u32>,
    character_bonus_by_rank: Vec<(i32, f64)>,
    max_card_id: usize,
}

impl PoolIndexes {
    /// 对 masterdata 各表建一次索引。
    pub(crate) fn build(game: &GameData<'_>) -> Self {
        let mut primary_unit_by_character = HashMap::new();
        for entry in game.game_character_units {
            primary_unit_by_character
                .entry(entry.game_character_id)
                .or_insert_with(|| parse_unit_code(&entry.unit));
        }
        let mut card_by_id = HashMap::with_capacity(game.cards.len());
        for card in game.cards {
            let mut unit_mask = 0u8;
            let primary_unit = primary_unit_by_character
                .get(&card.character_id)
                .copied()
                .flatten();
            let support_unit = card.support_unit.as_deref().and_then(parse_unit_code);
            let support_unit_unrestricted = card.support_unit.as_deref().is_none_or(|value| {
                value.trim().is_empty() || value.trim().eq_ignore_ascii_case("none")
            });
            if let Some(primary) = primary_unit {
                if let Some(unit) = unit_to_pool_index(primary) {
                    unit_mask |= 1u8 << unit;
                }
                if matches!(primary, crate::types::Unit::Piapro)
                    && let Some(secondary) = support_unit
                        .filter(|unit| !matches!(unit, crate::types::Unit::Piapro))
                        .and_then(unit_to_pool_index)
                {
                    unit_mask |= 1u8 << secondary;
                }
            }
            let mut master = card.clone();
            if (master.max_level.is_none() || master.max_skill_level.is_none())
                && let Some(rarity) = game
                    .card_rarities
                    .iter()
                    .find(|entry| entry.card_rarity_type == master.card_rarity_type)
            {
                master.max_level.get_or_insert(rarity.max_level);
                master.max_skill_level.get_or_insert(rarity.max_skill_level);
            }
            if master.max_master_rank.is_none() {
                master.max_master_rank = Some(
                    game.master_lessons
                        .iter()
                        .filter(|entry| entry.card_rarity_type == master.card_rarity_type)
                        .map(|entry| entry.master_rank)
                        .max()
                        .unwrap_or(0),
                );
            }
            card_by_id.entry(card.id).or_insert(PreparedCardIndex {
                master,
                base_power_by_level: Vec::new(),
                unit_mask,
                attr: parse_attr_code(&card.attr).and_then(attr_to_pool_index),
                primary_unit,
                support_unit,
                support_unit_unrestricted,
            });
        }

        let mut params_by_card: HashMap<i32, Vec<&CardParameter>> = HashMap::new();
        for entry in game.card_parameters {
            params_by_card.entry(entry.card_id).or_default().push(entry);
        }
        for card in card_by_id.values_mut() {
            let Some(params) = params_by_card.get(&card.master.id) else {
                continue;
            };
            let max_level = params
                .iter()
                .map(|entry| entry.level.max(0) as usize)
                .max()
                .unwrap_or(0);
            let mut by_level = vec![[0; 3]; max_level + 1];
            let mut current = [0; 3];
            for (level, slot) in by_level.iter_mut().enumerate() {
                for entry in params.iter().filter(|entry| entry.level == level as i32) {
                    current = [entry.param1, entry.param2, entry.param3];
                }
                *slot = current;
            }
            card.base_power_by_level = by_level;
        }

        let mut episodes_by_card: HashMap<i32, Vec<CardEpisode>> = HashMap::new();
        for entry in game.card_episodes {
            episodes_by_card
                .entry(entry.card_id)
                .or_default()
                .push(entry.clone());
        }

        let mut lessons_by_rarity: HashMap<i32, Vec<MasterLesson>> = HashMap::new();
        for entry in game.master_lessons {
            lessons_by_rarity
                .entry(entry.card_rarity_type)
                .or_default()
                .push(entry.clone());
        }

        // 原逻辑用 `.find`（取第一个匹配），故索引保留首个出现的项。
        let mut canvas_by_rarity = HashMap::new();
        for entry in game.card_mysekai_canvas_bonuses {
            canvas_by_rarity
                .entry(entry.card_rarity_type)
                .or_insert_with(|| entry.clone());
        }

        let mut area_by_item_level: HashMap<(i32, i32), Vec<PowerAreaItem>> = HashMap::new();
        for entry in game.area_item_levels {
            area_by_item_level
                .entry((entry.area_item_id, entry.level))
                .or_default()
                .push(PowerAreaItem {
                    unit: entry
                        .unit
                        .as_deref()
                        .and_then(parse_unit_code)
                        .and_then(unit_to_pool_index)
                        .unwrap_or(PowerAreaItem::ANY),
                    attr: entry
                        .attr
                        .as_deref()
                        .and_then(parse_attr_code)
                        .and_then(attr_to_pool_index)
                        .unwrap_or(PowerAreaItem::ANY),
                    character_id: entry.character_id.unwrap_or(PowerAreaItem::ANY_CHARACTER),
                    power_rate: entry.power_rate,
                    power_all_match_rate: entry.power_all_match_rate,
                });
        }

        // 原逻辑按 (skill_id, skill_level) `.find`，保留首个。
        let mut skill_by_id_level = HashMap::new();
        for entry in game.skills {
            skill_by_id_level
                .entry((entry.id, entry.level))
                .or_insert_with(|| entry.clone());
        }

        let mut effects_by_skill_level: HashMap<(i32, i32), Vec<PreparedSkillEffect>> =
            HashMap::new();
        for entry in game.skill_effects {
            let kind = match entry.effect_type.trim() {
                value
                    if value.eq_ignore_ascii_case("score_up")
                        || value.eq_ignore_ascii_case("score_up_condition_life")
                        || value.eq_ignore_ascii_case("score_up_keep") =>
                {
                    PreparedSkillEffectKind::ScoreUp
                }
                value if value.eq_ignore_ascii_case("life_recovery") => {
                    PreparedSkillEffectKind::LifeRecovery
                }
                value if value.eq_ignore_ascii_case("score_up_character_rank") => {
                    PreparedSkillEffectKind::CharacterRank
                }
                value if value.eq_ignore_ascii_case("score_up_unit_count") => {
                    PreparedSkillEffectKind::UnitCount
                }
                value if value.eq_ignore_ascii_case("score_up_diff") => {
                    PreparedSkillEffectKind::Diff
                }
                value if value.eq_ignore_ascii_case("score_up_reference") => {
                    PreparedSkillEffectKind::Reference
                }
                _ => PreparedSkillEffectKind::Other,
            };
            effects_by_skill_level
                .entry((entry.skill_id, entry.skill_level))
                .or_default()
                .push(PreparedSkillEffect {
                    kind,
                    value: entry.value,
                    additional_value: entry.additional_value,
                    unit_member_count: entry.unit_member_count,
                    unit: entry.unit.as_deref().and_then(parse_unit_code),
                    activate_character_rank: entry.activate_character_rank,
                });
        }

        let mut honor_bonus_by_id_level = HashMap::new();
        for honor in game.honors {
            for level in &honor.levels {
                honor_bonus_by_id_level
                    .entry((honor.id, level.level))
                    .or_insert(level.bonus.max(0) as u32);
            }
        }

        // Character-rank rows and the maximum card id are immutable masterdata.
        // Keep their compact lookup form in the shared snapshot instead of
        // rescanning the master tables for every account build.
        let mut character_bonus_by_rank = game
            .character_ranks
            .iter()
            .map(|entry| (entry.character_rank, entry.power_bonus_rate))
            .collect::<Vec<_>>();
        character_bonus_by_rank.sort_by_key(|entry| entry.0);
        let mut unique_character_bonuses = Vec::with_capacity(character_bonus_by_rank.len());
        for entry in character_bonus_by_rank {
            if unique_character_bonuses
                .last()
                .is_some_and(|previous: &(i32, f64)| previous.0 == entry.0)
            {
                if let Some(previous) = unique_character_bonuses.last_mut() {
                    *previous = entry;
                }
            } else {
                unique_character_bonuses.push(entry);
            }
        }
        let max_card_id = game
            .cards
            .iter()
            .map(|card| card.id)
            .filter(|id| *id >= 0)
            .max()
            .unwrap_or(0) as usize;

        Self {
            card_by_id,
            episodes_by_card,
            lessons_by_rarity,
            canvas_by_rarity,
            area_by_item_level,
            skill_by_id_level,
            effects_by_skill_level,
            honor_bonus_by_id_level,
            character_bonus_by_rank: unique_character_bonuses,
            max_card_id,
        }
    }

    #[inline]
    pub(crate) fn card_data(&self, card_id: i32) -> Option<&PreparedCardIndex> {
        self.card_by_id.get(&card_id)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn unit_mask(&self, card_id: i32) -> u8 {
        self.card_by_id
            .get(&card_id)
            .map(|card| card.unit_mask)
            .unwrap_or(0)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn attr(&self, card_id: i32) -> Option<u8> {
        self.card_by_id.get(&card_id).and_then(|card| card.attr)
    }

    #[inline]
    pub(crate) fn base_power(&self, card_id: i32, level: i32) -> [i32; 3] {
        let Some(card) = self.card_by_id.get(&card_id) else {
            return [0; 3];
        };
        let level = level.max(1) as usize;
        card.base_power_by_level
            .get(level)
            .or_else(|| card.base_power_by_level.last())
            .copied()
            .unwrap_or([0; 3])
    }

    #[inline]
    pub(crate) fn card_episodes(&self, card_id: i32) -> &[CardEpisode] {
        self.episodes_by_card
            .get(&card_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn master_lessons(&self, rarity: i32) -> &[MasterLesson] {
        self.lessons_by_rarity
            .get(&rarity)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn canvas_bonus(&self, rarity: i32) -> Option<&CardMysekaiCanvasBonus> {
        self.canvas_by_rarity.get(&rarity)
    }

    #[inline]
    pub(crate) fn area_items(&self, area_item_id: i32, level: i32) -> &[PowerAreaItem] {
        self.area_by_item_level
            .get(&(area_item_id, level))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn skill(&self, skill_id: i32, skill_level: i32) -> Option<&Skill> {
        self.skill_by_id_level.get(&(skill_id, skill_level))
    }

    #[inline]
    pub(crate) fn skill_effects(&self, skill_id: i32, skill_level: i32) -> &[PreparedSkillEffect] {
        self.effects_by_skill_level
            .get(&(skill_id, skill_level))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn honor_bonus(&self, honor_id: i32, level: i32) -> u32 {
        self.honor_bonus_by_id_level
            .get(&(honor_id, level))
            .copied()
            .unwrap_or(0)
    }

    #[inline]
    pub(crate) fn character_bonus_rate(&self, rank: i32) -> f64 {
        let index = self
            .character_bonus_by_rank
            .partition_point(|entry| entry.0 <= rank);
        index
            .checked_sub(1)
            .map(|index| self.character_bonus_by_rank[index].1)
            .unwrap_or(0.0)
    }

    #[inline]
    pub(crate) fn max_card_id(&self) -> usize {
        self.max_card_id
    }
}
