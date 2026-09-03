use crate::types::{
    DECK_SIZE, EventType, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy,
};

/// 预排序支援卡组。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SupportDeck {
    pub cards: Vec<(u16, f64)>,
    pub count: u8,
}

/// 单次搜索期间不变的常量上下文。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchContext {
    pub target: ScoreTarget,
    pub fixed_card_ids: Vec<u16>,
    pub fixed_character_ids: Vec<u8>,
    pub forced_leader_character_id: Option<u8>,
    pub music_rate_pct: u32,
    pub boost_rate_pct: u32,
    pub base_score: f64,
    pub base_score_auto: f64,
    pub fever_score: f64,
    pub skill_scores: [[f64; 6]; 3],
    pub other_score: i32,
    pub life: i32,
    pub diff_attr_bonus: [u16; 6],
    pub support_deck: SupportDeck,
    pub support_decks_by_character: Vec<SupportDeck>,
    pub is_world_bloom: bool,
    pub is_final_chapter: bool,
    /// challenge 模式下不要求角色唯一（pool 已过滤为同角色卡）
    pub enforce_char_uniqueness: bool,
    /// 反向搜索：求最弱（最小化 power）而非最强。仅 Power 目标生效，其它目标忽略。
    pub minimize: bool,
    pub live_type: LiveType,
    pub event_type: Option<EventType>,
    pub keep_after_training_state: bool,
    pub skill_reference_strategy: SkillReferenceStrategy,
    pub best_skill_as_leader: bool,
    pub live_skill_order: LiveSkillOrder,
    pub specific_skill_order: Option<[usize; DECK_SIZE]>,
    pub multi_teammate_score_up: Option<i32>,
    pub multi_teammate_power: Option<i32>,
    pub multi_live_score_up_lower_bound: Option<f64>,
    pub extra_bonus_ub: u32,
    pub w_power: f64,
    pub w_bonus: f64,
    pub skill_ub_global: u32,
    pub card_bonus_count_limit: usize,
    pub honor_bonus: u32,
    pub power_total_cap: Option<u32>,
    pub leader_honor_bonus: Vec<u16>,
    pub leader_limit_bonus: Vec<u16>,
    pub final_chapter_member_keep: Vec<bool>,
    pub skill_is_after_training: Vec<bool>,
    pub trained_to_special_image: Vec<bool>,
}

impl SearchContext {
    /// 返回按 `keep` 位图压缩后的搜索上下文。
    pub fn remap(&self, keep: &[bool]) -> Self {
        assert_eq!(
            self.skill_is_after_training.len(),
            keep.len(),
            "skill_is_after_training length must match pool count",
        );
        assert_eq!(
            self.leader_honor_bonus.len(),
            keep.len(),
            "leader_honor_bonus length must match pool count",
        );
        assert_eq!(
            self.leader_limit_bonus.len(),
            keep.len(),
            "leader_limit_bonus length must match pool count",
        );
        assert_eq!(
            self.trained_to_special_image.len(),
            keep.len(),
            "trained_to_special_image length must match pool count",
        );

        let mut remapped = self.clone();
        remapped.skill_is_after_training = remap_vec(&self.skill_is_after_training, keep);
        remapped.leader_honor_bonus = remap_vec(&self.leader_honor_bonus, keep);
        remapped.leader_limit_bonus = remap_vec(&self.leader_limit_bonus, keep);
        remapped.final_chapter_member_keep = remap_vec(&self.final_chapter_member_keep, keep);
        remapped.trained_to_special_image = remap_vec(&self.trained_to_special_image, keep);
        remapped
    }

    /// 返回当前 deck leader 对应的支援卡组。
    #[inline(always)]
    pub fn support_deck_for_leader(&self, leader_character_id: u8) -> &SupportDeck {
        if self.is_final_chapter
            && let Some(deck) = self
                .support_decks_by_character
                .get(leader_character_id as usize)
                .filter(|deck| deck.count > 0)
        {
            return deck;
        }
        &self.support_deck
    }

    /// 返回搜索期生效的 live 类型。
    #[inline(always)]
    pub fn effective_live_type(&self) -> LiveType {
        if matches!(self.live_type, LiveType::Multi)
            && self
                .event_type
                .is_some_and(|event_type| matches!(event_type, EventType::CheerfulCarnival))
        {
            LiveType::Cheerful
        } else {
            self.live_type
        }
    }

    /// 返回搜索期生效的 leader 选择策略。
    #[inline(always)]
    pub fn effective_best_skill_as_leader(&self) -> bool {
        self.best_skill_as_leader
            && !self.is_final_chapter
            && self.forced_leader_character_id.is_none()
            && self.fixed_character_ids.is_empty()
    }

    /// 卡组是否满足指定队长约束（队里必须有该角色的卡）。
    #[inline(always)]
    pub fn deck_matches_forced_leader(
        &self,
        pool: &crate::pool::CardPool,
        deck: &[crate::pool::CardIdx; DECK_SIZE],
    ) -> bool {
        let Some(leader_character_id) = self.forced_leader_character_id else {
            return true;
        };
        deck.iter()
            .any(|&card| pool.char_id(card) == leader_character_id)
    }

    /// 返回队长在 `deck` 中的槽位：指定队长时为该角色所在槽位，否则 `None`。
    #[inline(always)]
    pub fn forced_leader_slot(
        &self,
        pool: &crate::pool::CardPool,
        deck: &[crate::pool::CardIdx; DECK_SIZE],
    ) -> Option<usize> {
        let leader_character_id = self.forced_leader_character_id?;
        deck.iter()
            .position(|&card| pool.char_id(card) == leader_character_id)
    }

    /// 判断当前搜索是否走 Mysekai 路径。
    #[inline(always)]
    pub fn is_mysekai(&self) -> bool {
        matches!(self.target, ScoreTarget::Mysekai)
            || matches!(self.effective_live_type(), LiveType::Mysekai)
    }

    /// 判断当前搜索是否存在活动上下文。
    #[inline(always)]
    pub fn has_event(&self) -> bool {
        self.event_type.is_some()
    }

    /// 当前是否存在固定 leader 约束。
    #[inline(always)]
    pub fn has_fixed_leader(&self) -> bool {
        self.forced_leader_character_id.is_some() || !self.fixed_character_ids.is_empty()
    }

    /// 返回终章生效的固定队长角色。
    #[inline(always)]
    pub fn final_chapter_leader_character(&self) -> Option<u8> {
        self.forced_leader_character_id
            .or_else(|| self.fixed_character_at(0))
    }

    /// 读取指定槽位固定卡 ID。
    #[inline(always)]
    pub fn fixed_card_at(&self, slot: usize) -> Option<u16> {
        self.fixed_card_ids.get(slot).copied()
    }

    /// 读取指定槽位固定角色 ID。
    #[inline(always)]
    pub fn fixed_character_at(&self, slot: usize) -> Option<u8> {
        let index = slot.checked_sub(self.fixed_card_ids.len())?;
        self.fixed_character_ids.get(index).copied()
    }

    /// 判断指定槽位是否存在固定约束。
    #[inline(always)]
    pub fn is_fixed_slot(&self, slot: usize) -> bool {
        self.fixed_card_at(slot).is_some() || self.fixed_character_at(slot).is_some()
    }

    /// 判断是否是精确固定卡。
    #[inline(always)]
    pub fn is_fixed_game_id(&self, game_id: u16) -> bool {
        self.fixed_card_ids.contains(&game_id)
    }

    /// 统一应用综合力上限。
    #[inline(always)]
    pub fn clamp_power_total(&self, power_total: u32) -> u32 {
        self.power_total_cap
            .map_or(power_total, |cap| power_total.min(cap))
    }

    /// 读取指定卡位的终章称号加成。
    #[inline(always)]
    pub fn leader_honor_bonus_at(&self, dense_idx: usize) -> u32 {
        self.leader_honor_bonus.get(dense_idx).copied().unwrap_or(0) as u32
    }

    /// 读取指定卡位的终章当期队长加成。
    #[inline(always)]
    pub fn leader_limit_bonus_at(&self, dense_idx: usize) -> u32 {
        self.leader_limit_bonus.get(dense_idx).copied().unwrap_or(0) as u32
    }

    /// 判断终章 member 候选是否保留。
    #[inline(always)]
    pub fn final_chapter_member_keep_at(&self, dense_idx: usize) -> bool {
        self.final_chapter_member_keep
            .get(dense_idx)
            .copied()
            .unwrap_or(true)
    }

    /// 判断技能是否为花后技能。
    #[inline(always)]
    pub fn skill_is_after_training_at(&self, dense_idx: usize) -> bool {
        self.skill_is_after_training
            .get(dense_idx)
            .copied()
            .unwrap_or(false)
    }

    /// 判断当前卡默认立绘是否已是特训图。
    #[inline(always)]
    pub fn trained_to_special_image_at(&self, dense_idx: usize) -> bool {
        self.trained_to_special_image
            .get(dense_idx)
            .copied()
            .unwrap_or(false)
    }
}

fn remap_vec<T: Copy>(values: &[T], keep: &[bool]) -> Vec<T> {
    values
        .iter()
        .zip(keep.iter())
        .filter_map(|(value, keep)| keep.then_some(*value))
        .collect()
}
