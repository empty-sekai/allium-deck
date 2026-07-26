use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::leaf_evaluate_checked;
use super::types::{DeckResult, SearchParams};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BruteForceStats {
    pub candidates: u64,
    pub evaluated: u64,
    pub invalid: u64,
}

pub fn brute_force_search(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, BruteForceStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), BruteForceStats::default());
    }

    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
    let mut tracker = BruteForceTopK::new(params.top_k, minimize, pool);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut stats = BruteForceStats::default();
    recurse(pool, ctx, 0, 0, &mut deck, &mut tracker, &mut stats);
    (tracker.into_vec(), stats)
}

fn recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    depth: usize,
    min_free_idx: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    tracker: &mut BruteForceTopK,
    stats: &mut BruteForceStats,
) {
    if depth == DECK_SIZE {
        stats.candidates += 1;
        let Some(score) = leaf_evaluate_checked(pool, ctx, deck) else {
            stats.invalid += 1;
            return;
        };
        stats.evaluated += 1;
        tracker.insert(DeckResult::new(*deck, score));
        return;
    }

    let remaining = DECK_SIZE - depth;
    let is_fixed = ctx.is_fixed_slot(depth);
    let mut dense = if is_fixed { 0 } else { min_free_idx };
    while dense < pool.count() {
        if !is_fixed && pool.count() - dense < remaining {
            break;
        }
        let card = CardIdx::new(dense as u16);
        dense += 1;
        // 已选卡直接扫 deck 前缀（<= 4 项），不受池大小限制（issue #24 的 u64 位图溢出）。
        if selected_card_idx(deck, depth, card) {
            continue;
        }

        if !slot_matches(pool, ctx, depth, card) {
            continue;
        }
        if selected_game_id(pool, deck, depth, card) {
            continue;
        }
        if ctx.enforce_char_uniqueness && selected_character(pool, deck, depth, card) {
            let fixed_character_slot = ctx.fixed_character_at(depth) == Some(pool.char_id(card));
            let simple_target = matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill);
            if !(simple_target && fixed_character_slot) {
                continue;
            }
        }
        if ctx.is_final_chapter && depth > 0 && !ctx.final_chapter_member_keep_at(card.raw()) {
            continue;
        }

        deck[depth] = card;
        let next_min_free = if is_fixed { min_free_idx } else { dense };
        recurse(pool, ctx, depth + 1, next_min_free, deck, tracker, stats);
    }
}

#[inline(always)]
fn selected_card_idx(deck: &[CardIdx; DECK_SIZE], depth: usize, card: CardIdx) -> bool {
    let mut idx = 0usize;
    while idx < depth {
        if deck[idx] == card {
            return true;
        }
        idx += 1;
    }
    false
}

#[inline(always)]
fn slot_matches(pool: &CardPool, ctx: &SearchContext, depth: usize, card: CardIdx) -> bool {
    if let Some(game_id) = ctx.fixed_card_at(depth) {
        if pool.game_id(card) != game_id {
            return false;
        }
    }
    if let Some(character_id) = ctx.fixed_character_at(depth) {
        if pool.char_id(card) != character_id {
            return false;
        }
    }
    true
}

#[inline(always)]
fn selected_game_id(
    pool: &CardPool,
    deck: &[CardIdx; DECK_SIZE],
    depth: usize,
    card: CardIdx,
) -> bool {
    let game_id = pool.game_id(card);
    let mut idx = 0usize;
    while idx < depth {
        if pool.game_id(deck[idx]) == game_id {
            return true;
        }
        idx += 1;
    }
    false
}

#[inline(always)]
fn selected_character(
    pool: &CardPool,
    deck: &[CardIdx; DECK_SIZE],
    depth: usize,
    card: CardIdx,
) -> bool {
    let char_id = pool.char_id(card);
    let mut idx = 0usize;
    while idx < depth {
        if pool.char_id(deck[idx]) == char_id {
            return true;
        }
        idx += 1;
    }
    false
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
