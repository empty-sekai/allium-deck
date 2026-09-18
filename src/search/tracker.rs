//! Shared numeric objective ranking and public-card-set deduplication.
use super::DeckResult;
use crate::pool::CardPool;

pub(super) struct SimpleTopKTracker {
    top_k: usize,
    minimize: bool,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
}

impl SimpleTopKTracker {
    pub(super) fn new(top_k: usize, minimize: bool, pool: &CardPool) -> Self {
        Self {
            top_k,
            minimize,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
        }
    }

    /// Returns a pruning cutoff only after the requested number of sets is present.
    pub(super) fn threshold(&self) -> Option<u64> {
        if self.results.len() < self.top_k {
            None
        } else {
            self.results.last().map(|result| result.score)
        }
    }

    /// candidate 是否比 incumbent 更优。minimize 时「更优」= 分数更小。
    #[inline(always)]
    fn is_better(&self, candidate: &DeckResult, incumbent: &DeckResult) -> bool {
        let cmp = deck_result_cmp(candidate, incumbent);
        if self.minimize {
            cmp.is_gt()
        } else {
            cmp.is_lt()
        }
    }

    pub(super) fn insert(&mut self, candidate: DeckResult) {
        if let Some(existing_pos) = self
            .results
            .iter()
            .position(|existing| self.same_game_card_set(existing, &candidate))
        {
            if !self.is_better(&candidate, &self.results[existing_pos]) {
                return;
            }
            self.results.remove(existing_pos);
        }
        let pos = self
            .results
            .iter()
            .position(|existing| self.is_better(&candidate, existing))
            .unwrap_or(self.results.len());
        self.results.insert(pos, candidate);
        if self.results.len() > self.top_k {
            self.results.pop();
        }
    }

    pub(super) fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }

    fn same_game_card_set(&self, left: &DeckResult, right: &DeckResult) -> bool {
        self.game_card_set_key(left) == self.game_card_set_key(right)
    }

    fn game_card_set_key(&self, result: &DeckResult) -> [u16; 5] {
        let mut cards = result.cards.map(|card| self.game_ids[card.raw()]);
        cards.sort_unstable();
        cards
    }
}

#[inline(always)]
pub(super) fn deck_result_cmp(left: &DeckResult, right: &DeckResult) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.cards.cmp(&right.cards))
}
