//! Exact role assignment for a selected card set.
//!
//! The scoring function deliberately remains an evaluator of a concrete deck.
//! Search must not confuse a combination's arbitrary dense order with the set
//! of legal slot assignments. At most 5! arrangements need evaluation, and the
//! exchangeable common path still evaluates only once.
use super::evaluate::leaf_evaluate_checked;
use super::problem::{DeckProblem, PlacementModel};
use super::{DeckResult, SearchContext};
use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

#[inline]
pub(super) fn evaluate_candidate(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> Option<DeckResult> {
    let problem = DeckProblem::from_context(ctx);
    if !problem.needs_placement_search() {
        return leaf_evaluate_checked(pool, ctx, deck).map(|score| DeckResult::new(*deck, score));
    }
    let mut work = *deck;
    let mut best = None;
    if problem.placement == PlacementModel::SelectableLeader {
        // In this model the concrete evaluator canonically orders the members;
        // only the leader role remains observable.
        for slot in 0..DECK_SIZE {
            work.swap(0, slot);
            promote(pool, ctx, &work, &mut best);
            work.swap(0, slot);
        }
    } else {
        permute(pool, ctx, &mut work, problem.fixed_prefix, &mut best);
    }
    best
}

fn permute(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &mut [CardIdx; DECK_SIZE],
    slot: usize,
    best: &mut Option<DeckResult>,
) {
    if slot == DECK_SIZE {
        promote(pool, ctx, deck, best);
        return;
    }
    for other in slot..DECK_SIZE {
        deck.swap(slot, other);
        permute(pool, ctx, deck, slot + 1, best);
        deck.swap(slot, other);
    }
}

#[inline]
fn promote(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
    best: &mut Option<DeckResult>,
) {
    let Some(score) = leaf_evaluate_checked(pool, ctx, deck) else {
        return;
    };
    if best
        .as_ref()
        .is_none_or(|old| score > old.score || (score == old.score && *deck < old.cards))
    {
        *best = Some(DeckResult::new(*deck, score));
    }
}
