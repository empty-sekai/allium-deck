//! Independent exhaustive oracle. No production bounds, dominance, candidate
//! trimming, placement optimizer, or search traversal are used here.
use super::context::SearchContext;
use super::evaluate::leaf_evaluate_checked;
use super::types::{DeckResult, SearchParams};
use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};

/// Counts from exhaustive ordered-deck enumeration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BruteForceStats {
    /// Complete legal slot assignments offered to the objective evaluator.
    pub candidates: u64,
    /// Assignments accepted by the objective and skill constraints.
    pub evaluated: u64,
    /// Assignments rejected by leaf-level constraints.
    pub invalid: u64,
}

/// Small-instance reference solver over the full ordered feasible set.
///
/// Public card identities are distinct within a deck; cultivation variants and
/// alternative role assignments compete for one result per public card set.
/// This is intentionally factorial and is not a production search fallback.
pub struct ExactOracle<'a> {
    pool: &'a CardPool,
    context: &'a SearchContext,
}

impl<'a> ExactOracle<'a> {
    /// Binds an immutable pool and its semantic context.
    pub fn new(pool: &'a CardPool, context: &'a SearchContext) -> Self {
        Self { pool, context }
    }

    /// Enumerates every legal ordered deck, returning the exact distinct Top-K.
    /// `timeout_ms` is deliberately ignored: an oracle never returns a partial proof.
    pub fn search(&self, params: &SearchParams) -> (Vec<DeckResult>, BruteForceStats) {
        let pool = self.pool;
        let ctx = self.context;
        if params.top_k == 0 || pool.count() < DECK_SIZE {
            return (Vec::new(), BruteForceStats::default());
        }
        let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
        let mut tracker = BruteForceTopK::new(params.top_k, minimize, pool);
        let mut stats = BruteForceStats::default();
        enumerate(
            pool,
            ctx,
            0,
            &mut [CardIdx::new(0); DECK_SIZE],
            &mut tracker,
            &mut stats,
        );
        (tracker.into_vec(), stats)
    }
}

/// Compatibility entry point for the independent full-feasible-set oracle.
pub fn brute_force_search(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, BruteForceStats) {
    ExactOracle::new(pool, ctx).search(params)
}

fn enumerate(
    pool: &CardPool,
    ctx: &SearchContext,
    depth: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    tracker: &mut BruteForceTopK,
    stats: &mut BruteForceStats,
) {
    if depth == DECK_SIZE {
        stats.candidates += 1;
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            stats.evaluated += 1;
            tracker.insert(DeckResult::new(*deck, score));
        } else {
            stats.invalid += 1;
        }
        return;
    }
    // Every slot starts from zero. In particular, no ascending dense-index
    // constraint may silently remove an observable leader or skill-order role.
    for card in pool.indices() {
        let game = pool.game_id(card);
        let character = pool.char_id(card);
        if deck[..depth]
            .iter()
            .any(|&other| pool.game_id(other) == game)
        {
            continue;
        }
        if ctx.fixed_card_at(depth).is_some_and(|id| id != game)
            || ctx
                .fixed_character_at(depth)
                .is_some_and(|id| id != character)
        {
            continue;
        }
        if ctx.enforce_char_uniqueness {
            let duplicate = deck[..depth]
                .iter()
                .any(|&other| pool.char_id(other) == character);
            let explicitly_repeated_numeric_slot =
                matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill)
                    && ctx.fixed_character_at(depth) == Some(character);
            if duplicate && !explicitly_repeated_numeric_slot {
                continue;
            }
        } else if depth > 0 && character != pool.char_id(deck[0]) {
            continue;
        }
        if depth == 0
            && ctx.is_final_chapter
            && ctx
                .forced_leader_character_id
                .is_some_and(|id| id != character)
        {
            continue;
        }
        // final_chapter_member_keep is optimizer state, not a public constraint.
        // Deliberately do not read it here.
        deck[depth] = card;
        enumerate(pool, ctx, depth + 1, deck, tracker, stats);
    }
}

struct BruteForceTopK {
    top_k: usize,
    minimize: bool,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
}

impl BruteForceTopK {
    fn new(top_k: usize, minimize: bool, pool: &CardPool) -> Self {
        Self {
            top_k,
            minimize,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
        }
    }

    fn insert(&mut self, candidate: DeckResult) {
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

    fn is_better(&self, candidate: &DeckResult, incumbent: &DeckResult) -> bool {
        let cmp = deck_result_cmp(candidate, incumbent);
        if self.minimize {
            cmp.is_gt()
        } else {
            cmp.is_lt()
        }
    }

    fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }

    fn same_game_card_set(&self, left: &DeckResult, right: &DeckResult) -> bool {
        self.game_card_set_key(left) == self.game_card_set_key(right)
    }

    fn game_card_set_key(&self, result: &DeckResult) -> [u16; DECK_SIZE] {
        let mut cards = result.cards.map(|card| self.game_ids[card.raw()]);
        cards.sort_unstable();
        cards
    }
}

#[inline(always)]
fn deck_result_cmp(left: &DeckResult, right: &DeckResult) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.cards.cmp(&right.cards))
}
