use crate::pool::CardIdx;

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
}

/// 搜索参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchParams {
    pub top_k: usize,
    pub timeout_ms: u64,
}
