//! 建池查表索引（P3 性能优化）。
//!
//! `build_card_pool` 对每张用户卡都要在 masterdata 大表里按 id/rarity 线性 `find/filter`，
//! 其中 `card_parameters`（卡×等级，数万行）和 `area_item_levels` 是主要热点，
//! 导致大账号建池 ~90ms。本模块在建池开始时对这些表建一次按键索引，
//! 把每卡查表从 O(表行数) 降到 O(该卡相关行数)。
//!
//! 索引只借用 `GameData<'a>` 的切片，不复制数据；生命周期与 `GameData` 一致。
//! 改动是纯性能重构：所有查询结果集合与原线性扫描**完全等价**（同样的匹配条件、同样的元素），
//! 因此组卡输出必须逐字节不变。

use std::collections::HashMap;

use super::types::{
    AreaItemLevel, CardEpisode, CardMysekaiCanvasBonus, CardParameter, GameData, MasterCard,
    MasterLesson, Skill, SkillEffect,
};

/// 建池期只读查表索引。
pub(crate) struct PoolIndexes<'a> {
    card_by_id: HashMap<i32, &'a MasterCard>,
    params_by_card: HashMap<i32, Vec<&'a CardParameter>>,
    episodes_by_card: HashMap<i32, Vec<&'a CardEpisode>>,
    lessons_by_rarity: HashMap<i32, Vec<&'a MasterLesson>>,
    canvas_by_rarity: HashMap<i32, &'a CardMysekaiCanvasBonus>,
    area_by_item_level: HashMap<(i32, i32), Vec<&'a AreaItemLevel>>,
    skill_by_id_level: HashMap<(i32, i32), &'a Skill>,
    effects_by_skill_level: HashMap<(i32, i32), Vec<&'a SkillEffect>>,
}

impl<'a> PoolIndexes<'a> {
    /// 对 masterdata 各表建一次索引。
    pub(crate) fn build(game: &GameData<'a>) -> Self {
        let mut card_by_id = HashMap::with_capacity(game.cards.len());
        for card in game.cards {
            card_by_id.entry(card.id).or_insert(card);
        }

        let mut params_by_card: HashMap<i32, Vec<&CardParameter>> = HashMap::new();
        for entry in game.card_parameters {
            params_by_card.entry(entry.card_id).or_default().push(entry);
        }

        let mut episodes_by_card: HashMap<i32, Vec<&CardEpisode>> = HashMap::new();
        for entry in game.card_episodes {
            episodes_by_card
                .entry(entry.card_id)
                .or_default()
                .push(entry);
        }

        let mut lessons_by_rarity: HashMap<i32, Vec<&MasterLesson>> = HashMap::new();
        for entry in game.master_lessons {
            lessons_by_rarity
                .entry(entry.card_rarity_type)
                .or_default()
                .push(entry);
        }

        // 原逻辑用 `.find`（取第一个匹配），故索引保留首个出现的项。
        let mut canvas_by_rarity = HashMap::new();
        for entry in game.card_mysekai_canvas_bonuses {
            canvas_by_rarity
                .entry(entry.card_rarity_type)
                .or_insert(entry);
        }

        let mut area_by_item_level: HashMap<(i32, i32), Vec<&AreaItemLevel>> = HashMap::new();
        for entry in game.area_item_levels {
            area_by_item_level
                .entry((entry.area_item_id, entry.level))
                .or_default()
                .push(entry);
        }

        // 原逻辑按 (skill_id, skill_level) `.find`，保留首个。
        let mut skill_by_id_level = HashMap::new();
        for entry in game.skills {
            skill_by_id_level
                .entry((entry.id, entry.level))
                .or_insert(entry);
        }

        let mut effects_by_skill_level: HashMap<(i32, i32), Vec<&SkillEffect>> = HashMap::new();
        for entry in game.skill_effects {
            effects_by_skill_level
                .entry((entry.skill_id, entry.skill_level))
                .or_default()
                .push(entry);
        }

        Self {
            card_by_id,
            params_by_card,
            episodes_by_card,
            lessons_by_rarity,
            canvas_by_rarity,
            area_by_item_level,
            skill_by_id_level,
            effects_by_skill_level,
        }
    }

    #[inline]
    pub(crate) fn card(&self, card_id: i32) -> Option<&'a MasterCard> {
        self.card_by_id.get(&card_id).copied()
    }

    #[inline]
    pub(crate) fn card_parameters(&self, card_id: i32) -> &[&'a CardParameter] {
        self.params_by_card
            .get(&card_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn card_episodes(&self, card_id: i32) -> &[&'a CardEpisode] {
        self.episodes_by_card
            .get(&card_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn master_lessons(&self, rarity: i32) -> &[&'a MasterLesson] {
        self.lessons_by_rarity
            .get(&rarity)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn canvas_bonus(&self, rarity: i32) -> Option<&'a CardMysekaiCanvasBonus> {
        self.canvas_by_rarity.get(&rarity).copied()
    }

    #[inline]
    pub(crate) fn area_items(&self, area_item_id: i32, level: i32) -> &[&'a AreaItemLevel] {
        self.area_by_item_level
            .get(&(area_item_id, level))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[inline]
    pub(crate) fn skill(&self, skill_id: i32, skill_level: i32) -> Option<&'a Skill> {
        self.skill_by_id_level.get(&(skill_id, skill_level)).copied()
    }

    #[inline]
    pub(crate) fn skill_effects(&self, skill_id: i32, skill_level: i32) -> &[&'a SkillEffect] {
        self.effects_by_skill_level
            .get(&(skill_id, skill_level))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}
