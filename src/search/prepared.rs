//! Prepared immutable search plans.
use super::alternatives::expand_dominated_alternatives;
use super::{
    DeckResult, SearchContext, SearchParams, SearchStats, SuffixBound, dfs, eliminate_dominated,
    remap_results, warm_start,
};
use super::{SearchOutcome, budget::SearchBudget};
use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};
use std::sync::Arc;

/// Reusable immutable search data for one `CardPool` / `SearchContext` pair.
///
/// Preparing performs dominance compaction, suffix-table construction and warm
/// seeding once. Callers that already cache the pool can keep this beside it and
/// execute repeated exact searches without rebuilding those structures.
pub struct PreparedSearch {
    source_pool: Arc<()>,
    source_ctx: SearchContext,
    pool: CardPool,
    ctx: SearchContext,
    original_indices: Vec<CardIdx>,
    alternatives: Vec<Vec<CardIdx>>,
    suffix: SuffixBound,
    warm_seeds: Vec<DeckResult>,
    max_top_k: usize,
}

impl PreparedSearch {
    /// Prepares the standard character-unique DFS path.
    ///
    /// Specialized Power/Skill, challenge and Final Chapter searches keep their
    /// existing entry points and return `None` here.
    pub fn build(pool: &CardPool, ctx: &SearchContext, max_top_k: usize) -> Option<Self> {
        if max_top_k == 0
            || pool.count() < DECK_SIZE
            || matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill)
            || !ctx.enforce_char_uniqueness
            || ctx.is_final_chapter
        {
            return None;
        }

        let dominance = eliminate_dominated(pool, ctx);
        let suffix = SuffixBound::build_prepared(&dominance.pool, &dominance.ctx);
        let warm_seeds = warm_start::warm_start_seeds(&dominance.pool, &dominance.ctx, max_top_k);
        Some(Self {
            source_pool: Arc::clone(pool.instance_token()),
            source_ctx: ctx.clone(),
            pool: dominance.pool,
            ctx: dominance.ctx,
            original_indices: dominance.original_indices,
            alternatives: dominance.alternatives,
            suffix,
            warm_seeds,
            max_top_k,
        })
    }

    /// Executes an exact search for the same immutable pool instance and context.
    ///
    /// Returns `None` for a different pool, a changed context, or an unsupported
    /// `top_k`, so callers can rebuild the plan or use ordinary exact search.
    /// Moving the original pool is safe; reconstructing an equivalent pool is a
    /// different instance. No address or hash collision can validate stale data.
    pub fn search_instrumented(
        &self,
        original_pool: &CardPool,
        original_ctx: &SearchContext,
        params: &SearchParams,
    ) -> Option<(Vec<DeckResult>, SearchStats)> {
        if params.top_k == 0
            || params.top_k > self.max_top_k
            || !Arc::ptr_eq(&self.source_pool, original_pool.instance_token())
            || self.source_ctx != *original_ctx
        {
            return None;
        }
        let mut budget = SearchBudget::from_params(params);
        let seeds = self.warm_seeds.iter().copied().take(params.top_k).collect();
        let (compacted_results, mut stats) = dfs::dfs_search_with_budget(
            &self.pool,
            &self.ctx,
            &self.suffix,
            params,
            seeds,
            None,
            None,
            &mut budget,
        );
        let remapped = remap_results(compacted_results, &self.original_indices);
        let expanded = expand_dominated_alternatives(
            original_pool,
            original_ctx,
            &self.alternatives,
            params,
            remapped,
            &mut budget,
            &mut stats,
        );
        stats.deadline_hit |= budget.hit;
        stats.finalize();
        Some((expanded, stats))
    }
    /// Execute a compatible prepared query without dropping its completion record.
    pub fn search(
        &self,
        original_pool: &CardPool,
        original_ctx: &SearchContext,
        params: &SearchParams,
    ) -> Option<SearchOutcome<Vec<DeckResult>>> {
        self.search_instrumented(original_pool, original_ctx, params)
            .map(|(results, stats)| SearchOutcome::new(results, stats))
    }
}
