use crate::pool::{CardIdx, CardPool};

/// 搜索结果中的一组卡与其排序值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeckResult {
    pub cards: [CardIdx; 5],
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
    pub ordered_cards: [CardIdx; 5],
    pub card_event_bonus_rates: [f64; 5],
    pub card_skill_score_up: [f64; 5],
    pub card_power_total: [i32; 5],
    pub total_power: i32,
    pub live_score: i32,
    pub event_point: Option<i32>,
    pub multi_live_score_up: f64,
    pub event_bonus_total: Option<f64>,
}

/// 搜索参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchParams {
    pub top_k: usize,
    pub timeout_ms: u64,
}
