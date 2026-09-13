use crate::pool::{CardIdx, CardPool};

/// 搜索结果中的一组卡与其排序值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeckResult {
    /// 队伍成员，按站位顺序；索引只在产出它的 [`CardPool`] 内有效。
    pub cards: [CardIdx; 5],
    /// 排序值，语义随搜索目标而定，仅用于同一次搜索内比较。
    pub score: u64,
}

impl DeckResult {
    /// 构造一个搜索结果。
    #[inline(always)]
    pub const fn new(cards: [CardIdx; 5], score: u64) -> Self {
        Self { cards, score }
    }

    /// 返回不含站位顺序的规范化卡集合。
    #[inline(always)]
    pub fn card_set_key(&self) -> [CardIdx; 5] {
        let mut cards = self.cards;
        cards.sort_unstable();
        cards
    }

    /// 判断两个结果是否由同一组卡构成。
    #[inline(always)]
    pub fn same_card_set(&self, other: &Self) -> bool {
        self.card_set_key() == other.card_set_key()
    }

    /// 返回不含站位顺序的游戏卡 ID 集合。
    #[inline(always)]
    pub fn game_card_set_key(&self, pool: &CardPool) -> [u16; 5] {
        let mut cards = self.cards.map(|card| pool.game_id(card));
        cards.sort_unstable();
        cards
    }

    /// 判断两个结果是否由同一组游戏卡构成。
    #[inline(always)]
    pub fn same_game_card_set(&self, other: &Self, pool: &CardPool) -> bool {
        self.game_card_set_key(pool) == other.game_card_set_key(pool)
    }
}

/// 搜索结果的展示用指标汇总。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckResultSummary {
    /// 队伍成员，队长在前。
    pub ordered_cards: [CardIdx; 5],
    /// 每张卡的活动加成百分比，与 `ordered_cards` 同序。
    pub card_event_bonus_rates: [f64; 5],
    /// 每张卡的技能加成百分比，与 `ordered_cards` 同序。
    pub card_skill_score_up: [f64; 5],
    /// 每张卡的综合力，与 `ordered_cards` 同序。
    pub card_power_total: [i32; 5],
    /// 队伍综合力合计。
    pub total_power: i32,
    /// 本局 live 分数。
    pub live_score: i32,
    /// 活动 PT；无活动上下文时为 `None`。
    pub event_point: Option<i32>,
    /// 协力加成百分比。
    pub multi_live_score_up: f64,
    /// 活动加成合计百分比；无活动上下文时为 `None`。
    pub event_bonus_total: Option<f64>,
}

/// 搜索参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchParams {
    /// 返回的队伍数量上限。为 `0` 时搜索直接返回空集。
    pub top_k: usize,
    /// 搜索时间预算，毫秒；为 `0` 表示不限时。
    ///
    /// 超时后返回的是当时已收集到的结果，不是错误——因此超时的搜索不一定完整。
    pub timeout_ms: u64,
}
