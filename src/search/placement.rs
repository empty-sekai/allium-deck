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
    if !ctx.deck_matches_slots(pool, deck) {
        return None;
    }
    let problem = DeckProblem::from_context(ctx);
    if bonus_order_observable(pool, ctx, deck) {
        let mut work = *deck;
        let mut best = None;
        permute(
            pool,
            ctx,
            &mut work,
            problem.fixed_prefix.max(usize::from(ctx.is_final_chapter)),
            &mut best,
        );
        return best;
    }
    if !problem.needs_placement_search() {
        let mut work = *deck;
        let fixed = problem.fixed_prefix.max(usize::from(ctx.is_final_chapter));
        sort_exchangeable(pool, &mut work, fixed);
        return leaf_evaluate_checked(pool, ctx, &work).map(|score| DeckResult::new(work, score));
    }
    let mut work = *deck;
    let mut best = None;
    if problem.placement == PlacementModel::SelectableLeader {
        // In this model the concrete evaluator canonically orders the members;
        // only the leader role remains observable.
        for slot in 0..DECK_SIZE {
            let mut assignment = *deck;
            assignment.swap(0, slot);
            sort_exchangeable(pool, &mut assignment, 1);
            promote(pool, ctx, &assignment, &mut best);
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
    let candidate = DeckResult::new(*deck, score);
    if best
        .as_ref()
        .is_none_or(|old| super::tracker::deck_result_cmp(pool, ctx, &candidate, old).is_lt())
    {
        *best = Some(candidate);
    }
}

/// Whether first-N limited-bonus counting distinguishes free card orders.
/// Equal positive limited amounts are exchangeable; zero amounts do not consume
/// the cap. This test also accepts a whole pool for conservative dispatch.
pub(super) fn bonus_order_observable(
    pool: &CardPool,
    ctx: &SearchContext,
    cards: &[CardIdx],
) -> bool {
    if ctx.card_bonus_count_limit == 0
        || ctx.card_bonus_count_limit >= DECK_SIZE
        || (ctx.is_world_bloom && !ctx.is_final_chapter)
        || !matches!(
            ctx.target,
            crate::types::ScoreTarget::Score
                | crate::types::ScoreTarget::Bonus
                | crate::types::ScoreTarget::Mysekai
        )
    {
        return false;
    }
    let mut first = None;
    let mut different = false;
    let mut positive = 0;
    for &card in cards {
        let amount = pool.event_bonus_exact(card).limited_x10();
        if amount != 0 {
            positive += 1;
            different |= first.is_some_and(|value| value != amount);
            first.get_or_insert(amount);
        }
    }
    different && positive > ctx.card_bonus_count_limit
}

/// Offer every tier-observable assignment before deduplicating within a tier.
/// Maximizing Bonus for the set first would discard its lower, reachable tiers.
pub(super) fn visit_bonus_candidates(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
    mut visit: impl FnMut(DeckResult),
) {
    if !ctx.deck_matches_slots(pool, deck) {
        return;
    }
    if bonus_order_observable(pool, ctx, deck) {
        let fixed = DeckProblem::from_context(ctx)
            .fixed_prefix
            .max(usize::from(ctx.is_final_chapter));
        let mut work = *deck;
        visit_bonus_permutations(pool, ctx, &mut work, fixed, &mut visit);
    } else if let Some(candidate) = evaluate_candidate(pool, ctx, deck) {
        visit(candidate);
    }
}

fn visit_bonus_permutations(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &mut [CardIdx; DECK_SIZE],
    slot: usize,
    visit: &mut impl FnMut(DeckResult),
) {
    if slot == DECK_SIZE {
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            visit(DeckResult::new(*deck, score));
        }
        return;
    }
    for other in slot..DECK_SIZE {
        deck.swap(slot, other);
        visit_bonus_permutations(pool, ctx, deck, slot + 1, visit);
        deck.swap(slot, other);
    }
}

fn sort_exchangeable(pool: &CardPool, deck: &mut [CardIdx; DECK_SIZE], fixed: usize) {
    deck[fixed..].sort_unstable_by_key(|&card| (pool.game_id(card), card.raw()));
}
