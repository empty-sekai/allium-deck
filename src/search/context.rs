use crate::types::{
    EventType, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy, DECK_SIZE,
};

/// 预排序支援卡组。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SupportDeck {
    pub cards: Vec<(u16, u16)>,
    pub count: u8,
}

/// 单次搜索期间不变的常量上下文。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchContext {
    pub target: ScoreTarget,
    pub bonus_targets: Vec<u32>,
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
    pub is_world_bloom: bool,
    pub is_final_chapter: bool,
    pub live_type: LiveType,
    pub event_type: Option<EventType>,
    pub keep_after_training_state: bool,
    pub skill_reference_strategy: SkillReferenceStrategy,
    pub best_skill_as_leader: bool,
    pub live_skill_order: LiveSkillOrder,
    pub specific_skill_order: Option<[usize; DECK_SIZE]>,
    pub multi_teammate_score_up: Option<i32>,
    pub multi_teammate_power: Option<i32>,
    pub extra_bonus_ub: u32,
    pub w_power: f64,
    pub w_bonus: f64,
    pub skill_ub_global: u32,
    pub card_bonus_count_limit: usize,
    pub honor_bonus: u32,
    pub leader_honor_bonus: Vec<u16>,
    pub leader_limit_bonus: Vec<u16>,
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
        remapped.trained_to_special_image = remap_vec(&self.trained_to_special_image, keep);
        remapped
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
        self.best_skill_as_leader && !self.is_final_chapter
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
