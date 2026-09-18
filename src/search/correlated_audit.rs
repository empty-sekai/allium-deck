//! Exhaustive auditor for correlated upper-bound admissibility.
//! Compiled only for tests or the explicit diagnostics feature.
#![allow(missing_docs)]
use super::correlated::CorrelatedBound;
use super::evaluate::leaf_evaluate_checked;
use super::warm_start::warm_start_best;
use super::{PartialDeck, SearchContext, UsedSet};
use crate::{
    pool::{CardIdx, CardPool},
    types::DECK_SIZE,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct BoundViolation {
    pub depth: usize,
    pub start: usize,
    pub prefix_game_ids: Vec<u16>,
    pub upper_score: u32,
    pub exact_best_score: u32,
    pub upper_key: u64,
    pub exact_best_key: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CorrelatedAuditReport {
    pub active: bool,
    pub original_pool_size: usize,
    pub compacted_pool_size: usize,
    pub prefixes_checked: u64,
    pub leaves_evaluated: u64,
    pub valid_subtrees: u64,
    pub violations: u64,
    pub min_slack_score: Option<i64>,
    pub max_slack_score: Option<i64>,
    pub first_violation: Option<BoundViolation>,
}

pub fn audit_correlated_bound(pool: &CardPool, ctx: &SearchContext) -> CorrelatedAuditReport {
    let dominance = super::eliminate_dominated(pool, ctx);
    let search_pool = dominance.pool;
    let search_ctx = dominance.ctx;
    let mut report = CorrelatedAuditReport {
        original_pool_size: pool.count(),
        compacted_pool_size: search_pool.count(),
        ..CorrelatedAuditReport::default()
    };
    if search_pool.count() < DECK_SIZE {
        return report;
    }
    let hint = warm_start_best(&search_pool, &search_ctx).map(|r| {
        let p = r
            .cards
            .iter()
            .map(|&c| search_pool.power_max(c))
            .sum::<u32>();
        (p, r.score >> 32)
    });
    let Some(bound) = CorrelatedBound::build(&search_pool, &search_ctx, hint, 30, 0) else {
        return report;
    };
    report.active = true;
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    recurse(
        &search_pool,
        &search_ctx,
        &bound,
        0,
        0,
        &mut deck,
        UsedSet::new(),
        PartialDeck::default(),
        &mut report,
    );
    report
}

#[allow(clippy::too_many_arguments)]
fn recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    bound: &CorrelatedBound,
    depth: usize,
    start: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    used: UsedSet,
    partial: PartialDeck,
    report: &mut CorrelatedAuditReport,
) -> Option<u64> {
    if depth == DECK_SIZE {
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            report.leaves_evaluated += 1;
            return Some(score);
        }
        return None;
    }

    let mut exact_best: Option<u64> = None;
    let remaining = DECK_SIZE - depth;
    let mut dense = start;
    while dense < pool.count() {
        if pool.count() - dense < remaining {
            break;
        }
        let card = CardIdx::new(dense as u16);
        dense += 1;
        let char_id = pool.char_id(card);
        if ctx.enforce_char_uniqueness && used.contains(char_id) {
            continue;
        }
        if !slot_matches(pool, ctx, depth, card) {
            continue;
        }
        deck[depth] = card;
        let mut next_used = used;
        next_used.insert(char_id);
        let next_partial = PartialDeck {
            power: partial.power.saturating_add(pool.power_max(card)),
            skill: partial.skill.saturating_add(pool.skill_max(card) as u32),
            bonus: partial.bonus,
            max_skill: partial.max_skill.max(pool.skill_max(card)),
            limited_count: partial.limited_count,
        };
        if let Some(score) = recurse(
            pool,
            ctx,
            bound,
            depth + 1,
            dense,
            deck,
            next_used,
            next_partial,
            report,
        ) {
            exact_best = Some(exact_best.map_or(score, |best| best.max(score)));
        }
    }

    let exact_best = exact_best?;
    report.valid_subtrees += 1;
    let upper = bound.upper_bound(start, DECK_SIZE - depth, &used, &partial);
    report.prefixes_checked += 1;
    let upper_score = (upper >> 32) as u32;
    let exact_score = (exact_best >> 32) as u32;
    let slack = upper_score as i64 - exact_score as i64;
    report.min_slack_score = Some(report.min_slack_score.map_or(slack, |x| x.min(slack)));
    report.max_slack_score = Some(report.max_slack_score.map_or(slack, |x| x.max(slack)));
    if upper < exact_best {
        report.violations += 1;
        if report.first_violation.is_none() {
            report.first_violation = Some(BoundViolation {
                depth,
                start,
                prefix_game_ids: deck[..depth].iter().map(|&c| pool.game_id(c)).collect(),
                upper_score,
                exact_best_score: exact_score,
                upper_key: upper,
                exact_best_key: exact_best,
            });
        }
    }
    Some(exact_best)
}

#[inline]
fn slot_matches(pool: &CardPool, ctx: &SearchContext, depth: usize, card: CardIdx) -> bool {
    if ctx
        .fixed_card_at(depth)
        .is_some_and(|id| pool.game_id(card) != id)
    {
        return false;
    }
    if ctx
        .fixed_character_at(depth)
        .is_some_and(|id| pool.char_id(card) != id)
    {
        return false;
    }
    true
}
