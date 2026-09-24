//! Search layer: exact DFS with branch and bound.
//!
//! [`search`] is the general entry point and dispatches to the routine matching
//! the objective and live type. Before recursing it eliminates dominated cards,
//! builds character-aware suffix upper bounds ([`SuffixBound`]) and seeds a
//! lower bound by warm start, so branches that cannot beat the current Top-K are
//! cut as early as possible.
//!
//! Searches are bounded by [`SearchParams::timeout_ms`]; on expiry the results
//! collected so far are returned rather than an error, so a timed-out search is
//! not necessarily a complete one.

/// 精确档位搜索的可达加成集合。
/// 穷举参考实现，用于在测试中校验剪枝搜索的结果。
pub mod bruteforce;
/// 挑战 live 搜索：五张同角色，逐角色搜索后归并。
pub use solver::challenge as challenge_search;
mod alternatives;
mod budget;
mod composition;
/// 单次搜索期间不变的上下文。
pub mod context;
mod correlated;
/// Exhaustive bound auditing; intended for tests and opt-in diagnostics.
#[cfg(any(test, feature = "diagnostics"))]
pub mod correlated_audit;
/// 通用 DFS / 分支限界搜索。
pub mod dfs;
/// 支配裁剪：剔除不可能出现在最优解里的卡。
pub mod dominance;
/// 叶子求值：把一副确定的队伍算成分数。
pub mod evaluate;
mod log_linear;
mod objective;
pub mod solver;
mod tracker;
#[cfg(test)]
use alternatives::deck_matches_fixed_slots;
use alternatives::{expand_alternatives, expand_dominated_alternatives};
use solver::{final_chapter, numeric::search_simple_target};
use tracker::{TopKTracker, deck_result_cmp};
mod placement;
mod problem;
mod skill_ceiling;
/// 角色感知的后缀上界，用于剪枝。
pub mod suffix;
mod tuning;
/// 搜索的输入参数与结果类型。
pub mod types;
/// 热启动：先用贪心加一次换位得到一个可用下界。
pub mod warm_start;

pub use bruteforce::{BruteForceStats, ExactOracle, brute_force_search};
pub use context::{SearchContext, SupportDeck};
pub use dfs::{SearchDiagnostics, SearchStats, dfs_search};
pub use dominance::eliminate_dominated;
pub use evaluate::{
    calc_event_point, decode_u18, leaf_evaluate, resolve_power_for_cards, summarize_deck,
};
pub use suffix::{PartialDeck, SuffixBound, UsedSet};
pub use types::{DeckResult, DeckResultSummary, SearchCompletion, SearchOutcome, SearchParams};
pub use warm_start::warm_start;

/// Compare two legal deck results using the same canonical total order as every
/// exact Top-K tracker. This is the stable ordering contract for public result
/// sets and cross-solver/challenge aggregation; callers must not reimplement
/// score-only ordering because objective ties have deterministic public-set and
/// placement tie-breaks.
pub fn compare_deck_results(
    pool: &CardPool,
    ctx: &SearchContext,
    left: &DeckResult,
    right: &DeckResult,
) -> std::cmp::Ordering {
    tracker::deck_result_cmp(pool, ctx, left, right)
}

#[cfg(test)]
use crate::pool::CardIdx;
use crate::pool::CardPool;
use crate::types::{DECK_SIZE, ScoreTarget};
use budget::SearchBudget;

/// Execute the complete search pipeline and return its completion certificate.
/// Only `SearchCompletion::Complete` certifies canonical Top-K; `TimedOut`
/// contains legal, exactly evaluated incumbents, not a proven ranking.
pub fn search(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> SearchOutcome<Vec<DeckResult>> {
    search_outcome(pool, ctx, params)
}

/// 带统计信息的搜索。
pub fn search_instrumented(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    let outcome = search_outcome(pool, ctx, params);
    (outcome.results, outcome.stats)
}

/// Search with an explicit completion certificate and phase-aware work record.
/// The single cooperative deadline includes preparation, seeds and reconstruction.
fn search_outcome(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> SearchOutcome<Vec<DeckResult>> {
    let mut budget = SearchBudget::from_params(params);
    let (results, mut stats) = search_with_budget(pool, ctx, params, &mut budget);
    stats.deadline_hit |= budget.hit;
    SearchOutcome::new(results, stats)
}

fn search_with_budget(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let problem = problem::DeckProblem::from_context(ctx);

    // 挑战 live 的队伍必须五张同角色，该约束对所有 target 生效，必须先于
    // Power/Skill 通用路径分发：`simple_target_recurse` 无条件要求角色唯一，
    // 会在 challenge 下永远凑不齐 5 张而静默返回空集。
    if problem.family == problem::SolverFamily::SameCharacter {
        let suffix = SuffixBound::build(pool, ctx);
        // 池里只剩一个角色时（调用方已指定 challenge_live_character_id）直接搜；
        // 留着多个角色则是 challenge_all，必须逐角色搜索后归并——无约束搜索
        // 会产出跨角色的非法卡组，组合数也是逐角色之和的数个量级。
        return match single_challenge_character(pool) {
            Some(_) => challenge_search::search_with_budget(pool, ctx, &suffix, params, budget),
            None => challenge_search::search_all_characters_with_budget(
                pool, ctx, &suffix, params, budget,
            ),
        };
    }

    if problem.family == problem::SolverFamily::NumericObjective {
        return search_simple_target(pool, ctx, params, budget);
    }

    if let Some(power_ctx) = mysekai_power_view(pool, ctx) {
        let (results, stats) = search_simple_target(pool, &power_ctx, params, budget);
        return (rescore(pool, ctx, results), stats);
    }

    let bounds_enabled = tuning::SearchTuning::load().bounds;
    // Final Chapter solvers seed per leader job; every other family seeds
    // once on the whole pool and shares the incumbents across regimes.
    let seed = |budget: &mut SearchBudget, stats: &mut SearchStats| {
        if ctx.is_final_chapter {
            Vec::new()
        } else {
            warm_start::warm_start_seeds_with_budget(pool, ctx, params.top_k, budget, stats)
        }
    };
    composition::search_regimes(
        pool,
        ctx,
        params,
        budget,
        bounds_enabled,
        seed,
        |pool, ctx, floor, seeds, budget| {
            search_unique_characters(pool, ctx, params, floor, seeds, budget)
        },
    )
}

/// Exact search of one composition regime: character-unique decks whose
/// per-card power bound is admissible for every deck the caller needs found.
/// `floor` is a primary objective already reached by K known public sets;
/// `seeds` are known legal decks of this pool, used only as incumbents.
fn search_unique_characters(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    floor: u64,
    seeds: Vec<DeckResult>,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    let mut phase_stats = SearchStats::default();
    let dominance = eliminate_dominated(pool, ctx);
    phase_stats.dominance_prunes = (dominance.before - dominance.after) as u64;
    if budget.expired() {
        phase_stats.deadline_hit = true;
        return (Vec::new(), phase_stats);
    }
    let mut search_pool = dominance.pool;
    let mut search_ctx = dominance.ctx;
    let mut original_indices = dominance.original_indices;
    let alternatives = dominance.alternatives;
    if search_ctx.is_final_chapter {
        let member = dominance::compute_member_dominance(&search_pool, &search_ctx);
        phase_stats.dominance_prunes += member.keep.iter().filter(|&&keep| !keep).count() as u64;
        if budget.expired() {
            phase_stats.deadline_hit = true;
            return (Vec::new(), phase_stats);
        }
        // member 裁剪的替代记录映射回原始索引，并与第一轮 alternatives 做跨轮链闭包：
        // 真实次优卡组的 member 位可能是第一轮就被裁的卡（根 x），而 x 又被 member 轮
        // 裁掉（根 r）——从 r 出发必须能一步回换到它们（issue #7）。
        let mut member_alternatives = vec![Vec::new(); pool.count()];
        for (dense, alts) in member.alternatives.iter().enumerate() {
            if alts.is_empty() {
                continue;
            }
            let root = original_indices[dense].raw();
            for &alt_dense in alts {
                let alt = original_indices[alt_dense.raw()];
                member_alternatives[root].push(alt);
                member_alternatives[root].extend_from_slice(&alternatives[alt.raw()]);
            }
        }
        let member_keep = member.keep;
        if let Some(leader_char) = search_ctx.final_chapter_leader_character() {
            let keep = search_pool
                .indices()
                .map(|card| {
                    search_pool.char_id(card) == leader_char
                        || (member_keep.get(card.raw()).copied().unwrap_or(true)
                            && search_pool.char_id(card) != leader_char)
                })
                .collect::<Vec<_>>();
            original_indices = original_indices
                .into_iter()
                .zip(keep.iter().copied())
                .filter_map(|(idx, keep)| keep.then_some(idx))
                .collect();
            search_pool = search_pool.compact(&keep);
            search_ctx = search_ctx.remap(&keep);
            search_ctx.final_chapter_member_keep = vec![true; search_pool.count()];
        } else {
            search_ctx.final_chapter_member_keep = member_keep;
        }
        // Grouped Final search currently models only an optional leader role.
        // Multiple fixed slots use the complete slot-aware DFS, never a grouped
        // solver that silently omits their constraints.
        let grouped_constraints = matches!(
            search_ctx.target,
            ScoreTarget::Score | ScoreTarget::Bonus | ScoreTarget::Mysekai
        ) && search_ctx.fixed_card_ids.is_empty()
            && search_ctx.fixed_character_ids.len() <= 1
            && !placement::bonus_order_observable(
                &search_pool,
                &search_ctx,
                &search_pool.indices().collect::<Vec<_>>(),
            );
        let (compacted_results, mut stats) =
            if grouped_constraints && search_ctx.final_chapter_leader_character().is_some() {
                final_chapter::search_fixed_leader(&search_pool, &search_ctx, params, floor, budget)
            } else if grouped_constraints && !search_ctx.has_fixed_leader() {
                final_chapter::search_auto_leader(&search_pool, &search_ctx, params, floor, budget)
            } else {
                let suffix = SuffixBound::build(&search_pool, &search_ctx);
                // A single incumbent raises the cutoff only once it fills the Top-K.
                let seeds = if params.top_k == 1 {
                    warm_start::warm_start_best_with_budget(
                        &search_pool,
                        &search_ctx,
                        budget,
                        &mut phase_stats,
                    )
                    .into_iter()
                    .collect()
                } else {
                    Vec::new()
                };
                dfs::dfs_search_with_budget(
                    &search_pool,
                    &search_ctx,
                    &suffix,
                    params,
                    seeds,
                    floor,
                    budget,
                )
            };
        stats.accumulate(&phase_stats);
        let remapped = remap_results(compacted_results, &original_indices);
        let expanded = expand_alternatives(
            pool,
            ctx,
            &alternatives,
            &member_alternatives,
            params,
            remapped,
            budget,
            &mut stats,
        );
        stats.deadline_hit |= budget.hit;
        stats.finalize();
        return (expanded, stats);
    }
    let suffix = SuffixBound::build(&search_pool, &search_ctx);
    let seeds = compact_results(seeds, &original_indices, pool.count());
    let (compacted_results, mut stats) = dfs::dfs_search_with_budget(
        &search_pool,
        &search_ctx,
        &suffix,
        params,
        seeds,
        floor,
        budget,
    );
    stats.accumulate(&phase_stats);
    let remapped = remap_results(compacted_results, &original_indices);
    let expanded = expand_dominated_alternatives(
        pool,
        ctx,
        &alternatives,
        params,
        remapped,
        budget,
        &mut stats,
    );
    stats.deadline_hit |= budget.hit;
    stats.finalize();
    (expanded, stats)
}

/// 在一次 DFS 中为每个指定活动加成档位返回独立 Top-K。
pub fn search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    targets: &[i32],
) -> (Vec<DeckResult>, SearchStats) {
    let mut budget = SearchBudget::from_params(params);
    let (results, mut stats) =
        search_bonus_targets_with_budget(pool, ctx, params, targets, &mut budget);
    stats.deadline_hit |= budget.hit;
    stats.finalize();
    (results, stats)
}

fn search_bonus_targets_with_budget(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    targets: &[i32],
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0
        || pool.count() < DECK_SIZE
        || targets.is_empty()
        || !matches!(ctx.target, ScoreTarget::Bonus)
    {
        return (Vec::new(), SearchStats::default());
    }
    solver::bonus_tiers::search(pool, ctx, params, targets, budget)
}

/// 统一搜索入口（engine 与 wasm 共用，避免入口分叉）：
/// 无档位列表走完整搜索流水线；指定活动加成档位时逐档保留独立 Top-K。
pub fn search_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    target_bonus_list: &[i32],
) -> SearchOutcome<Vec<DeckResult>> {
    search_targets_outcome(pool, ctx, params, target_bonus_list)
}

/// A Power view of a MySekai search in which every deck has the same total
/// bonus. The MySekai value is then a non-decreasing function of resolved
/// power, which is also its first tie-break, so both targets share one
/// canonical order and one Top-K.
fn mysekai_power_view(pool: &CardPool, ctx: &SearchContext) -> Option<SearchContext> {
    if ctx.target != ScoreTarget::Mysekai || ctx.is_world_bloom || ctx.is_final_chapter {
        return None;
    }
    let first = pool.event_bonus_exact(crate::pool::CardIdx::new(0));
    pool.indices()
        .all(|card| pool.event_bonus_exact(card) == first)
        .then(|| SearchContext {
            target: ScoreTarget::Power,
            minimize: false,
            ..ctx.clone()
        })
}

/// Replace each result's objective by its value under `ctx`.
fn rescore(pool: &CardPool, ctx: &SearchContext, results: Vec<DeckResult>) -> Vec<DeckResult> {
    results
        .into_iter()
        .filter_map(|mut result| {
            let score = evaluate::leaf_evaluate_checked(pool, ctx, &result.cards);
            debug_assert!(
                score.is_some(),
                "a legal deck stays legal under a new target"
            );
            result.score = score?;
            Some(result)
        })
        .collect()
}

/// 池里只有一个角色时返回它；challenge 池保留多角色即为 challenge_all。
fn single_challenge_character(pool: &CardPool) -> Option<u8> {
    let mut only = None;
    for card in pool.indices() {
        let char_id = pool.char_id(card);
        match only {
            None => only = Some(char_id),
            Some(seen) if seen == char_id => {}
            Some(_) => return None,
        }
    }
    only
}

/// Inverse of [`remap_results`]: keeps the decks whose cards all survive the
/// compaction and rewrites them in compacted indices.
fn compact_results(
    results: Vec<DeckResult>,
    original_indices: &[crate::pool::CardIdx],
    original_count: usize,
) -> Vec<DeckResult> {
    let mut compacted = vec![None; original_count];
    for (dense, original) in original_indices.iter().enumerate() {
        compacted[original.raw()] = Some(crate::pool::CardIdx::new(dense as u16));
    }
    results
        .into_iter()
        .filter_map(|mut result| {
            for card in &mut result.cards {
                *card = compacted[card.raw()]?;
            }
            Some(result)
        })
        .collect()
}

fn remap_results(
    results: Vec<DeckResult>,
    original_indices: &[crate::pool::CardIdx],
) -> Vec<DeckResult> {
    results
        .into_iter()
        .map(|mut result| {
            for card in &mut result.cards {
                let dense = card.raw();
                debug_assert!(
                    dense < original_indices.len(),
                    "compacted search result index must have original mapping",
                );
                if let Some(original) = original_indices.get(dense).copied() {
                    *card = original;
                }
            }
            result
        })
        .collect()
}

#[cfg(test)]
mod tests;

/// Unified ordinary/tiered search, preserving completion and all work statistics.
/// Each requested exact bonus tier has its own canonical Top-K tracker, but all
/// tiers share this operation's deadline.
fn search_targets_outcome(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    target_bonus_list: &[i32],
) -> SearchOutcome<Vec<DeckResult>> {
    if target_bonus_list.is_empty() {
        search_outcome(pool, ctx, params)
    } else {
        let (results, stats) = search_bonus_targets(pool, ctx, params, target_bonus_list);
        SearchOutcome::new(results, stats)
    }
}
