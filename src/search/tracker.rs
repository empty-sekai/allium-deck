//! Canonical ranking and distinct public-card-set collection for every solver.
use super::{DeckResult, SearchContext, evaluate};
use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};

/// Ascending keys are better. Reversing a numeric objective never reverses ties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ResultKey {
    objective: u64,
    resolved_power: u32,
    public_set: [u16; DECK_SIZE],
    ordered_ids: [u16; DECK_SIZE],
    variants: [CardIdx; DECK_SIZE],
}

impl ResultKey {
    fn new(pool: &CardPool, ctx: &SearchContext, result: &DeckResult) -> Self {
        let ordered_ids = result.cards.map(|card| pool.game_id(card));
        let mut public_set = ordered_ids;
        public_set.sort_unstable();
        let power = if ctx.target == ScoreTarget::Mysekai {
            ctx.clamp_power_total(
                evaluate::resolve_power_target(pool, &result.cards) + ctx.honor_bonus,
            )
        } else {
            0
        };
        Self {
            objective: if ctx.minimize && ctx.target == ScoreTarget::Power {
                result.score
            } else {
                !result.score
            },
            resolved_power: !power,
            public_set,
            ordered_ids,
            variants: result.cards,
        }
    }
}

/// Total order of concrete legal results, independent of traversal and `top_k`.
pub(super) fn deck_result_cmp(
    pool: &CardPool,
    ctx: &SearchContext,
    left: &DeckResult,
    right: &DeckResult,
) -> std::cmp::Ordering {
    ResultKey::new(pool, ctx, left).cmp(&ResultKey::new(pool, ctx, right))
}

pub(super) struct TopKTracker {
    top_k: usize,
    bounds_enabled: bool,
    /// A primary objective that K known legal public sets already reach.
    floor: u64,
    results: Vec<DeckResult>,
    keys: Vec<ResultKey>,
}

impl TopKTracker {
    pub(super) fn new(top_k: usize) -> Self {
        Self::with_floor(top_k, 0)
    }

    /// Starts from an external incumbent: K distinct legal public sets whose
    /// primary objective is at least `floor` exist outside this tracker, so
    /// a branch strictly below `floor` cannot contribute to the global Top-K.
    /// The floor only raises pruning cutoffs; it never evicts a result.
    pub(super) fn with_floor(top_k: usize, floor: u64) -> Self {
        Self {
            top_k,
            bounds_enabled: true,
            floor,
            results: Vec::with_capacity(top_k),
            keys: Vec::with_capacity(top_k),
        }
    }

    pub(super) fn set_bounds_enabled(&mut self, enabled: bool) {
        self.bounds_enabled = enabled;
    }

    /// Numeric cutoffs exclude ties only with a strict bound comparison.
    pub(super) fn cutoff(&self) -> Option<u64> {
        if !self.bounds_enabled {
            return None;
        }
        let own = if self.results.len() < self.top_k {
            None
        } else {
            self.results.last().map(|result| result.score)
        };
        match own {
            Some(score) => Some(score.max(self.floor)),
            None => (self.floor != 0).then_some(self.floor),
        }
    }

    pub(super) fn threshold(&self) -> u64 {
        self.cutoff().unwrap_or(0)
    }

    /// Inputs must already satisfy the exact leaf and placement contracts.
    /// Keys are computed once, not re-evaluated for every retained incumbent.
    pub(super) fn insert(&mut self, pool: &CardPool, ctx: &SearchContext, candidate: DeckResult) {
        if self.top_k == 0 {
            return;
        }
        let key = ResultKey::new(pool, ctx, &candidate);
        if self.results.len() >= self.top_k && self.keys.last().is_some_and(|last| key >= *last) {
            return;
        }
        if let Some(existing) = self
            .keys
            .iter()
            .position(|old| old.public_set == key.public_set)
        {
            if key >= self.keys[existing] {
                return;
            }
            self.keys.remove(existing);
            self.results.remove(existing);
        }
        let pos = self.keys.partition_point(|old| *old < key);
        self.keys.insert(pos, key);
        self.results.insert(pos, candidate);
        if self.results.len() > self.top_k {
            self.results.pop();
            self.keys.pop();
        }
    }

    pub(super) fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }
}
