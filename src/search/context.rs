use crate::types::{
    DECK_SIZE, EventType, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy,
};

/// 预排序支援卡组。
///
/// World Bloom 的支援卡组按加成降序排好，搜索只取前 `count` 张计入。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SupportDeck {
    /// `(游戏卡 ID, 加成百分比)`，按加成降序。长度可以大于 `count`：
    /// 多出的条目是替补，主队伍占用某张支援卡时用来补位。
    pub cards: Vec<(u16, f64)>,
    /// 实际计入加成的张数。
    pub count: u8,
}

/// 终章某个队长角色假设佩戴的主称号。
///
/// 每副卡组只佩戴一枚主称号，因此每个队长角色只计入一行活动称号加成；
/// 称号 ID 与它的加成放在同一个值里，二者不会各自漂移。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaderHonor {
    /// 假设佩戴为主称号的已持有称号 ID。
    pub honor_id: i32,
    /// 该称号给这个队长角色的活动加成（百分比 × 10）。
    pub bonus_x10: u16,
}

/// 单次搜索期间不变的常量上下文。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchContext {
    /// 搜索最大化的目标。
    pub target: ScoreTarget,
    /// 必须入队的游戏卡 ID，按槽位顺序占据队首。
    pub fixed_card_ids: Vec<u16>,
    /// 必须入队的角色 ID，占据 `fixed_card_ids` 之后的槽位。
    pub fixed_character_ids: Vec<u8>,
    /// 必须占据队长位的角色。挑战 live 五张同角色，该约束无意义，建池时置 `None`。
    pub forced_leader_character_id: Option<u8>,
    /// 歌曲活动倍率，扩大 100 倍。
    pub music_rate_pct: u32,
    /// 活动 boost 倍率，扩大 100 倍。
    pub boost_rate_pct: u32,
    /// 歌曲基础分系数。
    pub base_score: f64,
    /// 歌曲 auto 模式基础分系数。
    pub base_score_auto: f64,
    /// 歌曲 fever 分系数。
    pub fever_score: f64,
    /// 技能分系数，按 `[0=solo, 1=multi, 2=auto][技能槽]` 索引。
    pub skill_scores: [[f64; 6]; 3],
    /// 计入总分的额外分数。
    pub other_score: i32,
    /// 初始生命值，影响生命回复类技能的收益。
    pub life: i32,
    /// World Bloom 异色加成，按队伍内不同属性数索引。
    pub diff_attr_bonus: [u16; 6],
    /// World Bloom 支援卡组。
    pub support_deck: SupportDeck,
    /// 终章逐队长角色的支援卡组，按角色 ID 索引；仅终章使用。
    pub support_decks_by_character: Vec<SupportDeck>,
    /// 当前是否为 World Bloom 活动。
    pub is_world_bloom: bool,
    /// 当前是否适用终章规则（队长限定加成、独立的支援卡组与综合力上限）。
    pub is_final_chapter: bool,
    /// challenge 模式下不要求角色唯一（pool 已过滤为同角色卡）
    pub enforce_char_uniqueness: bool,
    /// 反向搜索：求最弱（最小化 power）而非最强。仅 Power 目标生效，其它目标忽略。
    pub minimize: bool,
    /// live 模式。
    pub live_type: LiveType,
    /// 活动类型；无活动上下文时为 `None`。
    pub event_type: Option<EventType>,
    /// 吸分技能取值策略。
    pub skill_reference_strategy: SkillReferenceStrategy,
    /// 允许把技能最高的卡放到队长位。终章与指定队长时不生效，
    /// 判定走 [`SearchContext::effective_best_skill_as_leader`]。
    pub best_skill_as_leader: bool,
    /// 技能发动顺序假设。
    pub live_skill_order: LiveSkillOrder,
    /// `live_skill_order` 为 [`LiveSkillOrder::Specific`] 时的槽位排列。
    pub specific_skill_order: Option<[usize; DECK_SIZE]>,
    /// 协力队友的技能加成假设，单位为百分比。
    pub multi_teammate_score_up: Option<i32>,
    /// 协力队友的综合力假设。
    pub multi_teammate_power: Option<i32>,
    /// 协力加成的下界，用于把队友贡献钳在可信区间内。
    pub multi_live_score_up_lower_bound: Option<f64>,
    /// 单卡之外的加成来源上界（异色加成与支援卡组之和），供剪枝使用。
    pub extra_bonus_ub: u32,
    /// 热启动贪心排序里综合力的权重。
    pub w_power: f64,
    /// 热启动贪心排序里活动加成的权重。
    pub w_bonus: f64,
    /// 池内技能值前五之和，即整副队伍技能加成的上界。
    pub skill_ub_global: u32,
    /// 享受 limited bonus 的最大张数；终章为 4，其余通常为 [`DECK_SIZE`]。
    pub card_bonus_count_limit: usize,
    /// 称号带来的综合力加成，作为固定项计入每副队伍。
    pub honor_bonus: u32,
    /// 综合力上限；超过后按上限计算。
    pub power_total_cap: Option<u32>,
    /// 每张卡作为队长时的称号加成（百分比 × 10），按稠密卡索引。
    /// 建池时由 [`SearchContext::leader_honors`] 按卡的角色展开。
    pub leader_honor_bonus_x10: Vec<u16>,
    /// 终章各队长角色假设佩戴的主称号，按角色 ID 索引；非终章为空。
    ///
    /// 按角色而非按卡存储，[`SearchContext::remap`] 压缩卡索引时保持不变。
    pub leader_honors: Vec<Option<LeaderHonor>>,
    /// 每张卡作为队长时的当期限定加成（百分比 × 10），按稠密卡索引。
    pub leader_limit_bonus_x10: Vec<u16>,
    /// 终章 member 支配裁剪后仍保留的卡，按稠密卡索引。
    pub final_chapter_member_keep: Vec<bool>,
}

impl SearchContext {
    /// 返回按 `keep` 位图压缩后的搜索上下文。
    pub fn remap(&self, keep: &[bool]) -> Self {
        assert_eq!(
            self.leader_honor_bonus_x10.len(),
            keep.len(),
            "leader_honor_bonus_x10 length must match pool count",
        );
        assert_eq!(
            self.leader_limit_bonus_x10.len(),
            keep.len(),
            "leader_limit_bonus_x10 length must match pool count",
        );

        let mut remapped = self.clone();
        remapped.leader_honor_bonus_x10 = remap_vec(&self.leader_honor_bonus_x10, keep);
        remapped.leader_limit_bonus_x10 = remap_vec(&self.leader_limit_bonus_x10, keep);
        remapped.final_chapter_member_keep = remap_vec(&self.final_chapter_member_keep, keep);
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

    /// Public slot constraints shared by frontiers, seeds and reconstruction.
    /// Final bonuses use the concrete leader slot; ordinary forced leaders are
    /// still selected when the evaluator materializes their skill order.
    #[inline(always)]
    pub(crate) fn card_matches_slot(
        &self,
        pool: &crate::pool::CardPool,
        slot: usize,
        card: crate::pool::CardIdx,
    ) -> bool {
        self.fixed_card_at(slot)
            .is_none_or(|id| pool.game_id(card) == id)
            && self
                .fixed_character_at(slot)
                .is_none_or(|id| pool.char_id(card) == id)
            && (!(self.is_final_chapter && slot == 0)
                || self
                    .forced_leader_character_id
                    .is_none_or(|id| pool.char_id(card) == id))
    }

    /// Checks only public slot roles, never optimizer member-dominance state.
    #[inline]
    pub(crate) fn deck_matches_slots(
        &self,
        pool: &crate::pool::CardPool,
        deck: &[crate::pool::CardIdx; DECK_SIZE],
    ) -> bool {
        let constrained_prefix = (self.fixed_card_ids.len() + self.fixed_character_ids.len())
            .max(usize::from(
                self.is_final_chapter && self.forced_leader_character_id.is_some(),
            ))
            .min(DECK_SIZE);
        deck[..constrained_prefix]
            .iter()
            .enumerate()
            .all(|(slot, &card)| self.card_matches_slot(pool, slot, card))
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

    /// 读取指定卡位的终章称号加成（百分比 × 10）。
    #[inline(always)]
    pub fn leader_honor_bonus_x10_at(&self, dense_idx: usize) -> u32 {
        self.leader_honor_bonus_x10
            .get(dense_idx)
            .copied()
            .unwrap_or(0) as u32
    }

    /// 返回终章中该角色当队长时假设佩戴的主称号；非终章或无可用称号时为 `None`。
    #[inline]
    pub fn leader_honor_for_character(&self, character_id: u8) -> Option<LeaderHonor> {
        if !self.is_final_chapter {
            return None;
        }
        self.leader_honors
            .get(usize::from(character_id))
            .copied()
            .flatten()
    }

    /// 读取指定卡位的终章当期队长加成（百分比 × 10）。
    #[inline(always)]
    pub fn leader_limit_bonus_x10_at(&self, dense_idx: usize) -> u32 {
        self.leader_limit_bonus_x10
            .get(dense_idx)
            .copied()
            .unwrap_or(0) as u32
    }

    /// Integer-percent upper relaxation of the exact leader-only bonus.
    /// Both components remain in tenths until their final upward rounding.
    #[inline(always)]
    pub(crate) fn leader_bonus_upper_at(&self, dense_idx: usize) -> u32 {
        (self.leader_honor_bonus_x10_at(dense_idx) + self.leader_limit_bonus_x10_at(dense_idx))
            .div_ceil(10)
    }

    /// 判断终章 member 候选是否保留。
    #[inline(always)]
    pub fn final_chapter_member_keep_at(&self, dense_idx: usize) -> bool {
        self.final_chapter_member_keep
            .get(dense_idx)
            .copied()
            .unwrap_or(true)
    }
}

fn remap_vec<T: Copy>(values: &[T], keep: &[bool]) -> Vec<T> {
    values
        .iter()
        .zip(keep.iter())
        .filter_map(|(value, keep)| keep.then_some(*value))
        .collect()
}
