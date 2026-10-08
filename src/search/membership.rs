//! Exact final-chapter search with mandatory members and free input roles.
//!
//! A single pool, budget and canonical tracker cover every leader. The generic
//! frontier enforces required membership at each prefix; candidate evaluation
//! solves the remaining observable placement before inserting it into Top-K.
//! Slot-dependent dominance and grouped fixed-prefix solvers are not reused.

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};

use super::budget::SearchBudget;
use super::{DeckResult, SearchContext, SearchParams, SearchStats, SuffixBound};

pub(super) fn search(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    assert!(
        ctx.is_final_chapter,
        "members mode requires a final chapter"
    );
    // With no pins, or only the chosen leader, keep the existing grouped search.
    if ctx.fixed_card_ids.is_empty()
        && (ctx.fixed_character_ids.is_empty()
            || ctx.fixed_character_ids.as_slice() == ctx.forced_leader_character_id.as_slice())
    {
        let mut slots = ctx.clone();
        slots.fixed_constraint_mode = crate::handler::FixedConstraintMode::Slots;
        return super::search_with_budget(pool, &slots, params, budget);
    }
    if matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill) {
        return super::solver::numeric::search_simple_target(pool, ctx, params, budget);
    }
    let mut context = ctx.clone();
    context.final_chapter_member_keep.fill(true);
    super::composition::search_regimes(
        pool,
        &context,
        params,
        budget,
        super::tuning::SearchTuning::load().bounds,
        |_budget, _stats| Vec::new(),
        |pool, ctx, floor, _seeds, budget| {
            let suffix = SuffixBound::build(pool, ctx);
            super::dfs::dfs_search_with_budget(
                pool,
                ctx,
                &suffix,
                params,
                Vec::new(),
                floor,
                budget,
            )
        },
    )
}

/// A prefix can only continue when each missing required member remains in the
/// frontier and the missing members fit in the remaining slots. Availability
/// ignores interactions between future picks, making rejection conservative.
pub(super) fn prefix_can_complete(
    pool: &CardPool,
    ctx: &SearchContext,
    prefix: &[CardIdx],
    available: impl Iterator<Item = CardIdx> + Clone,
) -> bool {
    let legal = |card: CardIdx| {
        prefix.iter().all(|&other| {
            pool.game_id(other) != pool.game_id(card)
                && (!ctx.enforce_char_uniqueness || pool.char_id(other) != pool.char_id(card))
        })
    };
    let mut missing = 0usize;
    let mut required_characters = 0u32;
    for &id in &ctx.fixed_card_ids {
        if prefix.iter().any(|&card| pool.game_id(card) == id) {
            continue;
        }
        let Some(card) = available
            .clone()
            .find(|&card| pool.game_id(card) == id && legal(card))
        else {
            return false;
        };
        required_characters |= 1u32 << pool.char_id(card);
        missing += 1;
    }
    for &id in ctx
        .fixed_character_ids
        .iter()
        .chain(ctx.forced_leader_character_id.iter())
    {
        if prefix.iter().any(|&card| pool.char_id(card) == id)
            || required_characters & (1u32 << id) != 0
        {
            continue;
        }
        if !available
            .clone()
            .any(|card| pool.char_id(card) == id && legal(card))
        {
            return false;
        }
        required_characters |= 1u32 << id;
        missing += 1;
    }
    missing <= DECK_SIZE - prefix.len()
}
