//! Prepared immutable search plans.
use super::alternatives::expand_dominated_alternatives;
use super::{
    DeckResult, SearchContext, SearchParams, SearchStats, SuffixBound, dfs, eliminate_dominated,
    remap_results, warm_start,
};
use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};

/// Reusable immutable search data for one `CardPool` / `SearchContext` pair.
///
/// Preparing performs dominance compaction, suffix-table construction and warm
/// seeding once. Callers that already cache the pool can keep this beside it and
/// execute repeated exact searches without rebuilding those structures.
pub struct PreparedSearch {
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
            pool: dominance.pool,
            ctx: dominance.ctx,
            original_indices: dominance.original_indices,
            alternatives: dominance.alternatives,
            suffix,
            warm_seeds,
            max_top_k,
        })
    }

    /// Executes an exact search when `params.top_k` is covered by this plan.
    pub fn search_instrumented(
        &self,
        original_pool: &CardPool,
        original_ctx: &SearchContext,
        params: &SearchParams,
    ) -> Option<(Vec<DeckResult>, SearchStats)> {
        if params.top_k == 0 || params.top_k > self.max_top_k {
            return None;
        }
        let seeds = self.warm_seeds.iter().copied().take(params.top_k).collect();
        let (compacted_results, stats) = dfs::dfs_search_instrumented_with_seeds(
            &self.pool,
            &self.ctx,
            &self.suffix,
            params,
            seeds,
        );
        let remapped = remap_results(compacted_results, &self.original_indices);
        let expanded = expand_dominated_alternatives(
            original_pool,
            original_ctx,
            &self.alternatives,
            params,
            remapped,
        );
        Some((expanded, stats))
    }
}
