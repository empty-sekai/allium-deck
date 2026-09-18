use super::budget::SearchBudget;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, LiveType, ScoreTarget};

use super::bonus_reach::BonusReach;
use super::context::SearchContext;
use super::placement::evaluate_candidate;
use super::suffix::{PartialDeck, SuffixBound, UsedSet};
use super::tracker::TopKTracker;
use super::types::{DeckResult, SearchParams};
use super::warm_start::sorted_final_chapter_leaders;

const EP_DENSE_BREAK_STRIDE: usize = 4;
const EP_SHADOW_BLOCK_WIDTH: usize = 16;
const EP_SHADOW_MIN_DEPTH: usize = 3;

/// In Score/no-event search every legal result is encoded as `(live, live)`,
/// so the full 64-bit objective has exactly the same total order as `live`.
#[inline(always)]
fn score_noevent_threshold_live(threshold: u64) -> u32 {
    let live = threshold as u32;
    debug_assert_eq!(threshold >> 32, live as u64);
    live
}

#[derive(Clone, Copy)]
struct EpShadowBlock {
    upper_bounds: [u64; EP_SHADOW_BLOCK_WIDTH],
}

impl EpShadowBlock {
    #[inline(always)]
    const fn new() -> Self {
        Self {
            upper_bounds: [0; EP_SHADOW_BLOCK_WIDTH],
        }
    }
}

/// Work split by phase; these diagnostics are not solver-independent node units.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SearchDiagnostics {
    /// Seed frontier checkpoints, never proof-carrying states.
    pub seed_states: u64,
    /// Complete candidate sets evaluated by incumbent generation.
    pub seed_leaves: u64,
    /// Complete candidate sets evaluated by the exact frontier.
    pub proof_leaves: u64,
    /// Reconstruction frontier checkpoints.
    pub alternative_states: u64,
    /// Complete candidate sets evaluated during dominance reconstruction.
    pub alternative_leaves: u64,
    /// Fully processed additive power scenarios.
    pub power_scenarios_completed: u64,
    /// Auto/fixed leader jobs inspected by the proof search.
    pub leader_jobs: u64,
}

/// Search work and actual solver termination, not elapsed-time estimates.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SearchStats {
    /// Proof-frontier checkpoints (DFS prefixes, grouped states or DP transitions).
    /// Excludes seed and reconstruction work; granularity depends on the solver.
    pub visited_nodes: u64,
    /// True only when a phase stopped work because its shared deadline expired.
    pub deadline_hit: bool,
    /// Bound rejection decisions, not the number of descendant decks eliminated.
    pub bound_prunes: u64,
    /// Candidate/prefix rejection decisions caused by hard feasibility constraints.
    pub feasibility_prunes: u64,
    /// Candidates removed by certified dominance (including member-only removal).
    pub dominance_prunes: u64,
    /// Phase-specific work; seed evaluations never count as proof leaves.
    pub diagnostics: SearchDiagnostics,
    /// Compatibility counter: joint power/skill bound rejections.
    pub correlated_prunes: u64,
    /// 求值过的完整队伍数。
    pub leaf_nodes: u64,
    /// 因上界不及当前 Top-K 门限而剪掉的分支数。
    pub ub_prunes: u64,
    /// 因队长约束不满足而剪掉的分支数。
    pub leader_prunes: u64,
    /// 活动 PT 路径考察过的候选数。
    pub ep_candidates: u64,
    /// 活动 PT 路径中止整层枚举的次数。
    pub ep_break_prunes: u64,
    /// 活动 PT 路径跳过单个候选的次数。
    pub ep_continue_prunes: u64,
    /// 活动 PT 路径实际展开的候选数。
    pub ep_explored: u64,
    /// 单调性上界中止整层枚举的次数。
    pub mono_break_prunes: u64,
}

impl SearchStats {
    /// The single completion source of truth.
    pub fn completion(&self) -> super::SearchCompletion {
        if self.deadline_hit {
            super::SearchCompletion::TimedOut
        } else {
            super::SearchCompletion::Complete
        }
    }

    pub(crate) fn finalize(&mut self) {
        self.bound_prunes = self.ub_prunes
            + self.leader_prunes
            + self.correlated_prunes
            + self.ep_break_prunes
            + self.ep_continue_prunes
            + self.mono_break_prunes;
        self.diagnostics.proof_leaves = self
            .leaf_nodes
            .saturating_sub(self.diagnostics.seed_leaves)
            .saturating_sub(self.diagnostics.alternative_leaves);
    }

    /// Merge disjoint pieces of one operation, preserving any incomplete phase.
    pub fn accumulate(&mut self, part: &Self) {
        self.deadline_hit |= part.deadline_hit;
        self.visited_nodes += part.visited_nodes;
        self.leaf_nodes += part.leaf_nodes;
        self.bound_prunes += part.bound_prunes;
        self.feasibility_prunes += part.feasibility_prunes;
        self.dominance_prunes += part.dominance_prunes;
        self.ub_prunes += part.ub_prunes;
        self.leader_prunes += part.leader_prunes;
        self.correlated_prunes += part.correlated_prunes;
        self.ep_candidates += part.ep_candidates;
        self.ep_break_prunes += part.ep_break_prunes;
        self.ep_continue_prunes += part.ep_continue_prunes;
        self.ep_explored += part.ep_explored;
        self.mono_break_prunes += part.mono_break_prunes;
        self.diagnostics.seed_states += part.diagnostics.seed_states;
        self.diagnostics.seed_leaves += part.diagnostics.seed_leaves;
        self.diagnostics.proof_leaves += part.diagnostics.proof_leaves;
        self.diagnostics.alternative_states += part.diagnostics.alternative_states;
        self.diagnostics.alternative_leaves += part.diagnostics.alternative_leaves;
        self.diagnostics.power_scenarios_completed += part.diagnostics.power_scenarios_completed;
        self.diagnostics.leader_jobs += part.diagnostics.leader_jobs;
    }
}

/// 执行精确 DFS/B&B 搜索。
pub fn dfs_search(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> super::SearchOutcome<Vec<DeckResult>> {
    let (results, stats) = dfs_search_instrumented(pool, ctx, suffix, params, None);
    super::SearchOutcome::new(results, stats)
}

/// 单次 DFS 为每个精确活动加成档位保留独立 Top-K。
pub fn dfs_search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    targets: &[i32],
    bonus_reach: &BonusReach,
) -> (Vec<DeckResult>, SearchStats) {
    dfs_search_seeded_inner(
        pool,
        ctx,
        suffix,
        params,
        Vec::new(),
        Some(targets),
        Some(bonus_reach),
    )
}

/// 与 [`dfs_search`] 相同，额外返回剪枝统计。
pub fn dfs_search_instrumented(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seed: Option<DeckResult>,
) -> (Vec<DeckResult>, SearchStats) {
    let seeds = seed.into_iter().collect::<Vec<_>>();
    dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None, None)
}

fn dfs_search_seeded_inner(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seeds: Vec<DeckResult>,
    bonus_targets: Option<&[i32]>,
    bonus_reach: Option<&BonusReach>,
) -> (Vec<DeckResult>, SearchStats) {
    let mut budget = SearchBudget::from_params(params);
    dfs_search_with_budget(
        pool,
        ctx,
        suffix,
        params,
        seeds,
        bonus_targets,
        bonus_reach,
        &mut budget,
    )
}

pub(crate) fn dfs_search_with_budget(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seeds: Vec<DeckResult>,
    bonus_targets: Option<&[i32]>,
    bonus_reach: Option<&BonusReach>,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    // Seeds and combination leaves solve the same exact placement problem.
    // Align fixed slots first, then optimize every observable free role. A seed
    // must never be downgraded merely to match a combination-only oracle.
    let mut seed_stats = SearchStats::default();
    let mut canonical_seeds = Vec::with_capacity(seeds.len());
    for seed in seeds {
        if budget.expired_sampled() {
            break;
        }
        seed_stats.leaf_nodes += 1;
        seed_stats.diagnostics.seed_leaves += 1;
        if let Some(seed) = canonicalize_seed_result(pool, ctx, seed) {
            canonical_seeds.push(seed);
        }
    }
    let seeds = canonical_seeds;

    let mut tracker = match bonus_targets {
        Some(targets) => SearchTracker::Bonus(BonusBucketTracker::new(params.top_k, targets)),
        None => SearchTracker::TopK(TopKTracker::new(params.top_k)),
    };
    let correlated_hint = seeds.iter().max_by_key(|r| r.score).map(|r| {
        (
            r.cards.iter().map(|&c| pool.power_max(c)).sum::<u32>(),
            r.score >> 32,
        )
    });
    for seed_result in seeds {
        tracker.insert(pool, ctx, seed_result);
    }
    if budget.expired() {
        seed_stats.deadline_hit = true;
        seed_stats.finalize();
        return (tracker.into_vec(), seed_stats);
    }
    let correlated_kth = tracker.threshold() >> 32;

    let mut state = SearchState {
        correlated: super::correlated::CorrelatedBound::build(
            pool,
            ctx,
            correlated_hint,
            params.top_k,
            correlated_kth,
        ),
        pool,
        ctx,
        suffix,
        budget,
        tracker: &mut tracker,
        bonus_reach,
        node_count: 0,
        stats: seed_stats,
        avx512_candidate_mask: crate::simd::avx512_available(),
        bounds_enabled: super::tuning::SearchTuning::load().bounds,
    };
    state.budget.expired();
    let mut deck = [CardIdx::new(0); DECK_SIZE];

    if ctx.is_final_chapter {
        let leaders = sorted_final_chapter_leaders(pool, ctx)
            .into_iter()
            .filter(|leader| state.slot_matches(0, *leader))
            .collect::<Vec<_>>();
        for leader in leaders {
            if state.timed_out() {
                break;
            }
            deck[0] = leader;
            let mut used = UsedSet::new();
            used.insert(pool.char_id(leader));
            let (leader_bonus, leader_limited_inc) = partial_bonus_add(pool, ctx, leader, true, 0);
            let partial = PartialDeck {
                power: pool.power_max(leader),
                skill: pool.skill_max(leader) as u32,
                bonus: leader_bonus,
                max_skill: pool.skill_max(leader),
                limited_count: leader_limited_inc,
            };
            let threshold = state.threshold();
            if threshold != 0 {
                let leader_global = state.suffix.upper_bound_with_depth(1, &used, &partial);
                let leader_dense = state
                    .suffix
                    .dense_suffix_ceiling(0, &partial, DECK_SIZE - 1);
                if leader_global.min(leader_dense) < threshold {
                    state.stats.leader_prunes += 1;
                    continue;
                }
            }
            state.recurse(1, 0, &mut deck, used, partial, Some(leader), 0);
        }
    } else {
        state.recurse(
            0,
            0,
            &mut deck,
            UsedSet::new(),
            PartialDeck::default(),
            None,
            0,
        );
    }

    let mut stats = state.stats.clone();
    stats.visited_nodes = state.node_count;
    stats.deadline_hit = state.budget.hit;
    stats.finalize();
    (tracker.into_vec(), stats)
}

fn canonicalize_seed_result(
    pool: &CardPool,
    ctx: &SearchContext,
    seed: DeckResult,
) -> Option<DeckResult> {
    let source = seed.cards;
    let mut used_source = [false; DECK_SIZE];
    let mut deck = [source[0]; DECK_SIZE];
    let fixed_slots = (ctx.fixed_card_ids.len() + ctx.fixed_character_ids.len()).min(DECK_SIZE);

    // Fixed slots are scanned from dense index zero by the exact DFS.  For a
    // given card set choose the lexicographically first legal assignment.
    let mut depth = 0usize;
    while depth < fixed_slots {
        let mut chosen: Option<(usize, CardIdx)> = None;
        let mut source_pos = 0usize;
        while source_pos < DECK_SIZE {
            if !used_source[source_pos] {
                let card = source[source_pos];
                let game_ok = ctx
                    .fixed_card_at(depth)
                    .is_none_or(|game_id| pool.game_id(card) == game_id);
                let char_ok = ctx
                    .fixed_character_at(depth)
                    .is_none_or(|char_id| pool.char_id(card) == char_id);
                if game_ok
                    && char_ok
                    && chosen.is_none_or(|(_, current)| card.raw() < current.raw())
                {
                    chosen = Some((source_pos, card));
                }
            }
            source_pos += 1;
        }
        let (source_pos, card) = chosen?;
        used_source[source_pos] = true;
        deck[depth] = card;
        depth += 1;
    }

    let mut free = Vec::with_capacity(DECK_SIZE - fixed_slots);
    let mut source_pos = 0usize;
    while source_pos < DECK_SIZE {
        if !used_source[source_pos] {
            free.push(source[source_pos]);
        }
        source_pos += 1;
    }
    free.sort_unstable_by_key(|card| card.raw());
    for (slot, card) in (fixed_slots..DECK_SIZE).zip(free) {
        deck[slot] = card;
    }

    // Mirror the exact frontier's public-card and character feasibility checks.
    let mut used_chars = 0u32;
    let mut slot = 0usize;
    while slot < DECK_SIZE {
        let card = deck[slot];
        let game_id = pool.game_id(card);
        let mut prev = 0usize;
        while prev < slot {
            if pool.game_id(deck[prev]) == game_id {
                return None;
            }
            prev += 1;
        }
        let char_id = pool.char_id(card);
        if ctx.enforce_char_uniqueness
            && used_chars & (1u32 << char_id) != 0
            && ctx.fixed_character_at(slot) != Some(char_id)
        {
            return None;
        }
        used_chars |= 1u32 << char_id;
        slot += 1;
    }

    evaluate_candidate(pool, ctx, &deck)
}

struct SearchState<'a> {
    correlated: Option<super::correlated::CorrelatedBound>,
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    budget: &'a mut SearchBudget,
    tracker: &'a mut SearchTracker,
    /// Bonus-target reachability bitsets; `None` on every non-bucket path.
    bonus_reach: Option<&'a BonusReach>,
    node_count: u64,
    stats: SearchStats,
    avx512_candidate_mask: bool,
    bounds_enabled: bool,
}

impl SearchState<'_> {
    #[inline(always)]
    fn threshold(&self) -> u64 {
        if self.bounds_enabled {
            self.tracker.threshold()
        } else {
            0
        }
    }

    #[inline(always)]
    fn recurse(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        bonus_x10: u32,
    ) {
        if self.timed_out() {
            return;
        }
        if depth == DECK_SIZE {
            self.stats.leaf_nodes += 1;
            self.tracker.consider(self.pool, self.ctx, deck);
            return;
        }

        // Fixed slots are roles, not points on the ascending free-card frontier.
        // After filling any fixed prefix slot, the free frontier must restart
        // from zero; otherwise a high-index fixed card hides earlier legal cards.
        if self.ctx.is_fixed_slot(depth) {
            self.recurse_fixed_slot(depth, deck, used, partial, fixed_leader, bonus_x10);
            return;
        }

        if self.bounds_enabled && self.tracker.is_bonus() {
            // The character-aware suffix assumes uniqueness. Challenge tiers
            // instead keep only the independent additive reachability proof.
            let upper = if self.ctx.enforce_char_uniqueness {
                self.suffix.upper_bound_with_depth(depth, &used, &partial)
            } else {
                u64::MAX
            };
            // An outward-rounded upper component is never a valid lower
            // bound. In the additive model, exact tenths accumulate monotonically
            // and floor(x10 / 5) is a conservative encoded lower bound. With a
            // limited-count/support model even raw card sums are not additive,
            // so neither that lower bound nor additive reachability is used.
            let additive_bonus = !self.ctx.is_world_bloom
                && !self.ctx.is_final_chapter
                && self.ctx.card_bonus_count_limit >= DECK_SIZE;
            let lower_bonus_x2 = if additive_bonus { bonus_x10 / 5 } else { 0 };
            let reach = if additive_bonus {
                self.bonus_reach
            } else {
                None
            };
            if self
                .tracker
                .bonus_can_prune(lower_bonus_x2, upper, reach, start, depth, bonus_x10)
            {
                self.stats.ub_prunes += 1;
                return;
            }
        }

        let threshold = self.threshold();
        if threshold != 0 {
            let prunable = if matches!(self.ctx.target, ScoreTarget::Score) && self.ctx.has_event()
            {
                let global = self.suffix.upper_bound_with_depth(depth, &used, &partial);
                let dense = self
                    .suffix
                    .dense_suffix_ceiling(start, &partial, DECK_SIZE - depth);
                global.min(dense) < threshold
            } else if matches!(self.ctx.target, ScoreTarget::Score) {
                self.suffix.upper_bound_score_noevent_live(
                    self.pool,
                    &deck[..depth],
                    &used,
                    &partial,
                    DECK_SIZE - depth,
                ) < score_noevent_threshold_live(threshold)
            } else {
                self.suffix.upper_bound_with_depth(depth, &used, &partial) < threshold
            };
            if prunable {
                self.stats.ub_prunes += 1;
                return;
            }
        }

        let slots = DECK_SIZE - depth;
        if threshold != 0
            && self
                .correlated
                .as_ref()
                .is_some_and(|bound| bound.upper_bound(start, slots, &used, &partial) < threshold)
        {
            self.stats.correlated_prunes += 1;
            return;
        }

        match self.ctx.target {
            ScoreTarget::Power | ScoreTarget::Skill => {
                self.recurse_monotonic(
                    depth,
                    start,
                    deck,
                    used,
                    partial,
                    fixed_leader,
                    slots,
                    threshold,
                );
            }
            ScoreTarget::Score if !self.ctx.has_event() => {
                self.recurse_score_noevent_monotonic(
                    depth,
                    start,
                    deck,
                    used,
                    partial,
                    fixed_leader,
                    slots,
                    threshold,
                );
            }
            _ => {
                if threshold != 0 {
                    self.recurse_ep(
                        depth,
                        start,
                        deck,
                        used,
                        partial,
                        fixed_leader,
                        slots,
                        threshold,
                    );
                } else {
                    self.recurse_simple(depth, start, deck, used, partial, fixed_leader, bonus_x10);
                }
            }
        }
    }

    fn recurse_fixed_slot(
        &mut self,
        depth: usize,
        deck: &mut [CardIdx; DECK_SIZE],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        bonus_x10: u32,
    ) {
        for dense in 0..self.pool.count() {
            let card = CardIdx::new(dense as u16);
            let character = self.pool.char_id(card);
            if !self.slot_matches(depth, card)
                || fixed_leader == Some(card)
                || (self.ctx.enforce_char_uniqueness && used.contains(character))
                || (!self.ctx.enforce_char_uniqueness
                    && depth > 0
                    && character != self.pool.char_id(deck[0]))
                || deck[..depth]
                    .iter()
                    .any(|&other| self.pool.game_id(other) == self.pool.game_id(card))
            {
                continue;
            }
            deck[depth] = card;
            let mut next_used = used;
            next_used.insert(character);
            let (bonus, limited) =
                partial_bonus_add(self.pool, self.ctx, card, depth == 0, partial.limited_count);
            let next = PartialDeck {
                power: partial.power + self.pool.power_max(card),
                skill: partial.skill + self.pool.skill_max(card) as u32,
                bonus: partial.bonus + bonus,
                max_skill: partial.max_skill.max(self.pool.skill_max(card)),
                limited_count: partial.limited_count + limited,
            };
            self.recurse(
                depth + 1,
                0,
                deck,
                next_used,
                next,
                fixed_leader,
                bonus_x10 + self.pool.event_bonus(card).total_x10() as u32,
            );
            if self.budget.hit {
                return;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_monotonic(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        slots: usize,
        threshold: u64,
    ) {
        let pre = self.suffix.precompute_layer(&used, slots);
        let mut dense = start;
        while dense < self.pool.count() {
            let card = CardIdx::new(dense as u16);
            dense += 1;
            if fixed_leader.is_some_and(|leader| leader == card) {
                continue;
            }
            let char_id = self.pool.char_id(card);
            if !self.slot_matches(depth, card) {
                continue;
            }
            if self.ctx.enforce_char_uniqueness && used.contains(char_id) {
                continue;
            }

            if threshold != 0 {
                let eb = self.pool.event_bonus(card);
                let card_bonus = eb.total_ceil();
                let bonus_total =
                    partial.bonus + card_bonus + pre.suffix_bonus + pre.extra_bonus_ub;
                let tight_power = partial.power + self.pool.power_max(card) + pre.suffix_power_rest;
                let tight_skill =
                    partial.skill + self.pool.skill_max(card) as u32 + pre.skill_ub_rest;
                let tight_leader = (partial.max_skill as u32).max(self.pool.skill_max(card) as u32);
                let ceil = self
                    .suffix
                    .ceiling(tight_power, bonus_total, tight_skill, tight_leader);
                if ceil < threshold {
                    self.stats.mono_break_prunes += 1;
                    break;
                }
            }

            unsafe {
                *deck.get_unchecked_mut(depth) = card;
            }
            let mut next_used = used;
            next_used.insert(char_id);
            let (card_bonus, limited_inc) =
                partial_bonus_add(self.pool, self.ctx, card, false, partial.limited_count);
            let next_partial = PartialDeck {
                power: partial.power + self.pool.power_max(card),
                skill: partial.skill + self.pool.skill_max(card) as u32,
                bonus: partial.bonus + card_bonus,
                max_skill: partial.max_skill.max(self.pool.skill_max(card)),
                limited_count: partial.limited_count + limited_inc,
            };
            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                0,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_score_noevent_monotonic(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        slots: usize,
        threshold: u64,
    ) {
        let threshold_live = if threshold == 0 {
            0
        } else {
            score_noevent_threshold_live(threshold)
        };
        let pre = self.suffix.precompute_layer_score_noevent(&used, slots);
        let mut dense = start;
        while dense < self.pool.count() {
            if threshold != 0 {
                let live_ceil = self
                    .suffix
                    .score_noevent_dense_live_ceiling(dense, &partial, slots);
                if live_ceil < threshold_live {
                    self.stats.mono_break_prunes += 1;
                    break;
                }
            }
            let card = CardIdx::new(dense as u16);
            dense += 1;
            if fixed_leader.is_some_and(|leader| leader == card) {
                continue;
            }
            let char_id = self.pool.char_id(card);
            if !self.slot_matches(depth, card) {
                continue;
            }
            if self.ctx.enforce_char_uniqueness && used.contains(char_id) {
                continue;
            }

            if threshold != 0 {
                let tight_power = partial.power + self.pool.power_max(card) + pre.suffix_power_rest
                    - pre.power_delta(char_id);
                let tight_skill =
                    partial.skill + self.pool.skill_max(card) as u32 + pre.skill_ub_rest
                        - pre.skill_delta(char_id);
                // 队长上界只对未来仍可填充的槽位计入未用角色的最优技能。
                let remaining_best_skill = if char_id == pre.best_skill_char {
                    pre.second_best_skill
                } else {
                    pre.best_unused_skill
                };
                let tight_leader = (partial.max_skill as u32)
                    .max(self.pool.skill_max(card) as u32)
                    .max(remaining_best_skill as u32);
                let live_ub =
                    self.suffix
                        .score_noevent_live_ceiling(tight_power, tight_skill, tight_leader);
                if live_ub < threshold_live {
                    self.stats.ep_continue_prunes += 1;
                    continue;
                }
            }

            unsafe {
                *deck.get_unchecked_mut(depth) = card;
            }
            let mut next_used = used;
            next_used.insert(char_id);
            let (card_bonus, limited_inc) =
                partial_bonus_add(self.pool, self.ctx, card, false, partial.limited_count);
            let next_partial = PartialDeck {
                power: partial.power + self.pool.power_max(card),
                skill: partial.skill + self.pool.skill_max(card) as u32,
                bonus: partial.bonus + card_bonus,
                max_skill: partial.max_skill.max(self.pool.skill_max(card)),
                limited_count: partial.limited_count + limited_inc,
            };
            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                0,
            );
        }
    }

    /// Exclusion-aware suffix-max 剪枝：Score/Mysekai 专用。
    /// 单遍扫描：即算 ceiling 即决定 explore/skip，无栈数组。
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_ep(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        slots: usize,
        mut threshold: u64,
    ) {
        let use_multi_score_event_fast_path = matches!(self.ctx.target, ScoreTarget::Score)
            && self.ctx.has_event()
            && matches!(self.ctx.effective_live_type(), LiveType::Multi)
            && !self.ctx.is_world_bloom
            && !self.ctx.is_final_chapter;
        let pre = (!use_multi_score_event_fast_path)
            .then(|| self.suffix.precompute_layer_ep(&used, slots));

        let world_bloom_parts = self.ctx.is_world_bloom.then(|| {
            let mut attr_set = 0u8;
            let mut selected = [0u16; DECK_SIZE];
            let mut selected_len = 0usize;
            let mut pos = 0usize;
            while pos < depth {
                let card = deck[pos];
                attr_set |= 1u8 << self.pool.attr(card);
                selected[selected_len] = self.pool.game_id(card);
                selected_len += 1;
                pos += 1;
            }
            (attr_set, selected, selected_len)
        });
        let partial_extra_bonus_ub =
            world_bloom_parts
                .as_ref()
                .map(|(attr_set, selected, selected_len)| {
                    self.suffix.world_bloom_extra_bonus_bound_from_parts(
                        *attr_set,
                        selected,
                        *selected_len,
                        slots,
                        start,
                        used.bits(),
                    )
                });

        let use_avx512_candidate_mask = self.avx512_candidate_mask
            && fixed_leader.is_none()
            && !self.ctx.is_final_chapter
            && self.ctx.enforce_char_uniqueness
            && self.ctx.fixed_card_ids.is_empty()
            && self.ctx.fixed_character_ids.is_empty();
        if use_multi_score_event_fast_path
            && use_avx512_candidate_mask
            && depth >= EP_SHADOW_MIN_DEPTH
        {
            self.recurse_ep_multi_shadow(depth, start, deck, used, partial, slots, threshold);
            return;
        }
        let mut mask_block_start = usize::MAX;
        let mut mask_block = 0u16;
        let mut dense = start;
        while dense < self.pool.count() {
            if self.timed_out() {
                return;
            }
            if threshold != 0
                && matches!(self.ctx.target, ScoreTarget::Score)
                && (dense - start).is_multiple_of(EP_DENSE_BREAK_STRIDE)
            {
                let ceil = if use_multi_score_event_fast_path {
                    self.suffix
                        .dense_suffix_ceiling_multi_score_event(dense, &partial, slots)
                } else if let Some(extra_bonus_ub) = partial_extra_bonus_ub {
                    self.suffix.dense_suffix_ceiling_with_extra(
                        dense,
                        &partial,
                        slots,
                        extra_bonus_ub,
                    )
                } else {
                    self.suffix.dense_suffix_ceiling(dense, &partial, slots)
                };
                if ceil < threshold {
                    self.stats.mono_break_prunes += 1;
                    break;
                }
            }
            let card = CardIdx::new(dense as u16);
            dense += 1;
            if use_avx512_candidate_mask {
                let candidate_dense = dense - 1;
                let block_start = candidate_dense & !15;
                if block_start != mask_block_start {
                    mask_block_start = block_start;
                    if block_start + 16 <= self.pool.count() {
                        mask_block = unsafe {
                            crate::simd::unused_character_mask_16_avx512_unchecked(
                                self.pool.char_ids().as_ptr().add(block_start),
                                used.bits(),
                            )
                        };
                    } else {
                        // AVX-512 loads require a full 16-card block.  The old
                        // tail fallback used u16::MAX and then skipped the scalar
                        // uniqueness check entirely, allowing duplicate-character
                        // decks from the final partial block.  Build the exact
                        // legal mask lane-by-lane for the tail instead.
                        let block_len = (self.pool.count() - block_start).min(16);
                        let mut tail_mask = 0u16;
                        let mut lane = 0usize;
                        while lane < block_len {
                            let tail_card = CardIdx::new((block_start + lane) as u16);
                            if !used.contains(self.pool.char_id(tail_card)) {
                                tail_mask |= 1u16 << lane;
                            }
                            lane += 1;
                        }
                        mask_block = tail_mask;
                    }
                }
                if mask_block & (1u16 << (candidate_dense - block_start)) == 0 {
                    continue;
                }
            }
            let char_id = self.pool.char_id(card);
            if !use_avx512_candidate_mask {
                if fixed_leader.is_some_and(|leader| leader == card) {
                    continue;
                }
                if !self.slot_matches(depth, card) {
                    continue;
                }
                if self.ctx.enforce_char_uniqueness && used.contains(char_id) {
                    continue;
                }
            }

            let eb = self.pool.event_bonus(card);
            let card_bonus = eb.total_ceil();
            let (card_base_bonus, card_limited_bonus) = if self.ctx.is_final_chapter {
                let exact = self.pool.event_bonus_exact(card);
                (exact.base_ceil(), exact.limited_ceil())
            } else {
                (card_bonus, 0)
            };
            let card_power = self.pool.power_max(card);
            let card_skill = self.pool.skill_max(card);
            let card_skill_u32 = card_skill as u32;

            self.stats.ep_candidates += 1;

            let dense_ub_global = if use_multi_score_event_fast_path {
                let dense_upper = self.suffix.dense_candidate_ceiling_multi_score_event(
                    dense,
                    &partial,
                    card_power,
                    card_bonus,
                    card_skill_u32,
                    slots,
                );
                if dense_upper < threshold || slots < 3 {
                    dense_upper
                } else {
                    dense_upper.min(self.suffix.dense_candidate_joint_ceiling_multi_score_event(
                        dense,
                        &partial,
                        card_power,
                        card_bonus,
                        card_skill_u32,
                        slots,
                    ))
                }
            } else {
                self.suffix.dense_candidate_ceiling(
                    dense,
                    &partial,
                    card_power,
                    card_bonus,
                    card_base_bonus,
                    card_limited_bonus,
                    card_skill_u32,
                    slots,
                )
            };
            if dense_ub_global < threshold {
                self.stats.ep_continue_prunes += 1;
                continue;
            }

            if !use_multi_score_event_fast_path {
                let pre = unsafe { pre.as_ref().unwrap_unchecked() };
                let tight_power =
                    partial.power + card_power + pre.suffix_power_rest - pre.power_delta(char_id);
                let bonus_total_global = partial.bonus + card_bonus + pre.suffix_bonus
                    - pre.bonus_delta(char_id)
                    + pre.extra_bonus_ub;
                let tight_skill =
                    partial.skill + card_skill_u32 + pre.skill_ub_rest - pre.skill_delta(char_id);
                let remaining_best_skill = if char_id == pre.best_skill_char {
                    pre.second_best_skill
                } else {
                    pre.best_unused_skill
                };
                let tight_leader = (partial.max_skill as u32)
                    .max(card_skill_u32)
                    .max(remaining_best_skill as u32);

                let global_ub =
                    self.suffix
                        .ceiling(tight_power, bonus_total_global, tight_skill, tight_leader);
                if global_ub < threshold {
                    self.stats.ep_continue_prunes += 1;
                    continue;
                }

                if let Some((selected_attr_set, selected, selected_len)) = world_bloom_parts {
                    let card_attr_bit = 1u8 << self.pool.attr(card);
                    let card_game_id = self.pool.game_id(card);
                    let extra_bonus_ub = self
                        .suffix
                        .world_bloom_extra_bonus_bound_for_candidate_parts(
                            selected_attr_set | card_attr_bit,
                            &selected,
                            selected_len,
                            card_game_id,
                            slots.saturating_sub(1),
                            dense,
                            used.bits() | (1u32 << char_id),
                        );
                    let bonus_total = partial.bonus + card_bonus + pre.suffix_bonus
                        - pre.bonus_delta(char_id)
                        + extra_bonus_ub;
                    let refined = self
                        .suffix
                        .ceiling(tight_power, bonus_total, tight_skill, tight_leader)
                        .min(self.suffix.dense_candidate_ceiling_with_extra(
                            dense,
                            &partial,
                            card_power,
                            card_bonus,
                            card_base_bonus,
                            card_limited_bonus,
                            card_skill_u32,
                            slots,
                            extra_bonus_ub,
                        ));
                    if refined < threshold {
                        self.stats.ep_continue_prunes += 1;
                        continue;
                    }
                }
            }

            self.stats.ep_explored += 1;

            unsafe {
                *deck.get_unchecked_mut(depth) = card;
            }
            let mut next_used = used;
            next_used.insert(char_id);
            let (card_bonus_add, limited_inc) =
                partial_bonus_add(self.pool, self.ctx, card, false, partial.limited_count);
            let next_partial = PartialDeck {
                power: partial.power + card_power,
                skill: partial.skill + card_skill_u32,
                bonus: partial.bonus + card_bonus_add,
                max_skill: partial.max_skill.max(card_skill),
                limited_count: partial.limited_count + limited_inc,
            };
            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                0,
            );

            let new_threshold = self.threshold();
            if new_threshold > threshold {
                threshold = new_threshold;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_ep_multi_shadow(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; DECK_SIZE],
        used: UsedSet,
        partial: PartialDeck,
        slots: usize,
        mut threshold: u64,
    ) {
        let count = self.pool.count();
        let mut dense = start;
        while dense < count {
            if self.timed_out() {
                return;
            }

            let block_len = (count - dense).min(EP_SHADOW_BLOCK_WIDTH);
            let mut legal_mask = if block_len == EP_SHADOW_BLOCK_WIDTH {
                unsafe {
                    crate::simd::unused_character_mask_16_avx512_unchecked(
                        self.pool.char_ids().as_ptr().add(dense),
                        used.bits(),
                    )
                }
            } else {
                let mut mask = 0u16;
                let mut lane = 0usize;
                while lane < block_len {
                    let card = CardIdx::new((dense + lane) as u16);
                    if !used.contains(self.pool.char_id(card)) {
                        mask |= 1u16 << lane;
                    }
                    lane += 1;
                }
                mask
            };
            let mut block = EpShadowBlock::new();
            let block_start = dense;
            let mut lane = 0usize;
            while lane < block_len {
                if lane.is_multiple_of(EP_DENSE_BREAK_STRIDE) {
                    let ceil = self.suffix.dense_suffix_ceiling_multi_score_event(
                        block_start + lane,
                        &partial,
                        slots,
                    );
                    if ceil < threshold {
                        self.stats.mono_break_prunes += 1;
                        legal_mask &= (1u16 << lane).wrapping_sub(1);
                        break;
                    }
                }

                if legal_mask & (1u16 << lane) != 0 {
                    let candidate_dense = block_start + lane;
                    let card = CardIdx::new(candidate_dense as u16);
                    let card_power = self.pool.power_max(card);
                    let card_skill = self.pool.skill_max(card);
                    let card_bonus = self.pool.event_bonus(card).total_ceil();
                    block.upper_bounds[lane] =
                        self.suffix.dense_candidate_ceiling_multi_score_event(
                            candidate_dense + 1,
                            &partial,
                            card_power,
                            card_bonus,
                            card_skill as u32,
                            slots,
                        );
                    self.stats.ep_candidates += 1;
                }
                lane += 1;
            }

            if legal_mask == 0 {
                if lane < block_len {
                    break;
                }
                dense += block_len;
                continue;
            }

            let mut surviving = legal_mask
                & unsafe {
                    crate::simd::upper_bound_mask_16_avx512_unchecked(
                        block.upper_bounds.as_ptr(),
                        threshold,
                    )
                };
            self.stats.ep_continue_prunes += (legal_mask ^ surviving).count_ones() as u64;

            if surviving != 0 {
                let first_lane = highest_upper_bound_lane(&block.upper_bounds, surviving);
                surviving &= !(1u16 << first_lane);
                self.explore_ep_shadow_lane(
                    depth,
                    deck,
                    used,
                    partial,
                    block_start,
                    first_lane,
                    slots,
                );
                threshold = threshold.max(self.threshold());

                let refreshed = unsafe {
                    crate::simd::upper_bound_mask_16_avx512_unchecked(
                        block.upper_bounds.as_ptr(),
                        threshold,
                    )
                };
                let filtered = surviving & !refreshed;
                self.stats.ep_continue_prunes += filtered.count_ones() as u64;
                surviving &= refreshed;

                while surviving != 0 {
                    let next_lane = surviving.trailing_zeros() as usize;
                    surviving &= surviving - 1;
                    self.explore_ep_shadow_lane(
                        depth,
                        deck,
                        used,
                        partial,
                        block_start,
                        next_lane,
                        slots,
                    );
                    let new_threshold = self.threshold();
                    if new_threshold > threshold {
                        threshold = new_threshold;
                        let refreshed = unsafe {
                            crate::simd::upper_bound_mask_16_avx512_unchecked(
                                block.upper_bounds.as_ptr(),
                                threshold,
                            )
                        };
                        let filtered = surviving & !refreshed;
                        self.stats.ep_continue_prunes += filtered.count_ones() as u64;
                        surviving &= refreshed;
                    }
                }
            }

            if lane < block_len {
                break;
            }
            dense += block_len;
        }
    }

    #[inline(always)]
    fn explore_ep_shadow_lane(
        &mut self,
        depth: usize,
        deck: &mut [CardIdx; DECK_SIZE],
        used: UsedSet,
        partial: PartialDeck,
        block_start: usize,
        lane: usize,
        slots: usize,
    ) {
        self.stats.ep_explored += 1;
        let candidate_dense = block_start + lane;
        let card = CardIdx::new(candidate_dense as u16);
        unsafe {
            *deck.get_unchecked_mut(depth) = card;
        }

        if slots == 1 {
            self.stats.leaf_nodes += 1;
            self.tracker.consider(self.pool, self.ctx, deck);
            return;
        }

        let char_id = self.pool.char_id(card);
        let mut next_used = used;
        next_used.insert(char_id);
        let card_power = self.pool.power_max(card);
        let card_skill = self.pool.skill_max(card);
        let card_bonus = self.pool.event_bonus(card).total_ceil();
        let next_partial = PartialDeck {
            power: partial.power + card_power,
            skill: partial.skill + card_skill as u32,
            bonus: partial.bonus + card_bonus,
            max_skill: partial.max_skill.max(card_skill),
            limited_count: partial.limited_count,
        };
        self.recurse(
            depth + 1,
            candidate_dense + 1,
            deck,
            next_used,
            next_partial,
            None,
            0,
        );
    }

    #[inline(always)]
    fn recurse_simple(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        bonus_x10: u32,
    ) {
        let mut dense = start;
        while dense < self.pool.count() {
            let card = CardIdx::new(dense as u16);
            dense += 1;
            if fixed_leader.is_some_and(|leader| leader == card) {
                continue;
            }
            let char_id = self.pool.char_id(card);
            if !self.slot_matches(depth, card) {
                continue;
            }
            if (self.ctx.enforce_char_uniqueness && used.contains(char_id))
                || (!self.ctx.enforce_char_uniqueness
                    && depth > 0
                    && char_id != self.pool.char_id(deck[0]))
                || deck[..depth]
                    .iter()
                    .any(|&other| self.pool.game_id(other) == self.pool.game_id(card))
            {
                continue;
            }
            unsafe {
                *deck.get_unchecked_mut(depth) = card;
            }
            let mut next_used = used;
            next_used.insert(char_id);
            let (card_bonus, limited_inc) =
                partial_bonus_add(self.pool, self.ctx, card, false, partial.limited_count);
            let next_partial = PartialDeck {
                power: partial.power + self.pool.power_max(card),
                skill: partial.skill + self.pool.skill_max(card) as u32,
                bonus: partial.bonus + card_bonus,
                max_skill: partial.max_skill.max(self.pool.skill_max(card)),
                limited_count: partial.limited_count + limited_inc,
            };
            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                bonus_x10 + self.pool.event_bonus(card).total_x10() as u32,
            );
        }
    }

    #[inline(always)]
    fn timed_out(&mut self) -> bool {
        if self.budget.hit {
            return true;
        }
        self.node_count = self.node_count.wrapping_add(1);
        self.budget.expired_sampled()
    }

    #[inline(always)]
    fn slot_matches(&self, depth: usize, card: CardIdx) -> bool {
        if self.ctx.is_final_chapter
            && depth > 0
            && !self.ctx.final_chapter_member_keep_at(card.raw())
        {
            return false;
        }
        if let Some(game_id) = self.ctx.fixed_card_at(depth)
            && self.pool.game_id(card) != game_id
        {
            return false;
        }
        if let Some(character_id) = self.ctx.fixed_character_at(depth)
            && self.pool.char_id(card) != character_id
        {
            return false;
        }
        true
    }
}

enum SearchTracker {
    TopK(TopKTracker),
    Bonus(BonusBucketTracker),
}

impl SearchTracker {
    #[inline(always)]
    fn is_bonus(&self) -> bool {
        matches!(self, Self::Bonus(_))
    }

    #[inline(always)]
    fn threshold(&self) -> u64 {
        match self {
            Self::TopK(tracker) => tracker.threshold(),
            Self::Bonus(_) => 0,
        }
    }

    #[inline(always)]
    fn bonus_can_prune(
        &self,
        lower_bonus_x2: u32,
        upper: u64,
        bonus_reach: Option<&BonusReach>,
        start: usize,
        depth: usize,
        bonus_x10: u32,
    ) -> bool {
        match self {
            Self::Bonus(tracker) => {
                tracker.can_prune(lower_bonus_x2, upper, bonus_reach, start, depth, bonus_x10)
            }
            Self::TopK(_) => false,
        }
    }

    fn consider(&mut self, pool: &CardPool, ctx: &SearchContext, deck: &[CardIdx; DECK_SIZE]) {
        match self {
            Self::TopK(tracker) => {
                if let Some(candidate) = evaluate_candidate(pool, ctx, deck) {
                    tracker.insert(pool, ctx, candidate);
                }
            }
            Self::Bonus(tracker) => {
                super::placement::visit_bonus_candidates(pool, ctx, deck, |candidate| {
                    // The encoded objective quantizes bonus to half-percent units;
                    // that key is not the membership predicate of an exact tier.
                    let total = super::evaluate::resolve_total_bonus(pool, ctx, &candidate.cards);
                    let target = (candidate.score >> 32) as u32;
                    if total == f64::from(target) / 2.0 {
                        tracker.insert(pool, ctx, candidate);
                    }
                })
            }
        }
    }

    #[inline(always)]
    fn insert(&mut self, pool: &CardPool, ctx: &SearchContext, candidate: DeckResult) {
        match self {
            Self::TopK(tracker) => tracker.insert(pool, ctx, candidate),
            Self::Bonus(tracker) => tracker.insert(pool, ctx, candidate),
        }
    }

    fn into_vec(self) -> Vec<DeckResult> {
        match self {
            Self::TopK(tracker) => tracker.into_vec(),
            Self::Bonus(tracker) => tracker.into_vec(),
        }
    }
}

struct BonusBucketTracker {
    buckets: Vec<(u32, TopKTracker)>,
}

impl BonusBucketTracker {
    fn new(top_k: usize, targets: &[i32]) -> Self {
        let mut target_x2 = targets
            .iter()
            .copied()
            .filter(|target| *target >= 0)
            .map(|target| (target as u32).saturating_mul(2))
            .collect::<Vec<_>>();
        target_x2.sort_unstable();
        target_x2.dedup();
        Self {
            buckets: target_x2
                .into_iter()
                .map(|target| (target, TopKTracker::new(top_k)))
                .collect(),
        }
    }

    #[inline(always)]
    fn insert(&mut self, pool: &CardPool, ctx: &SearchContext, candidate: DeckResult) {
        let target = (candidate.score >> 32) as u32;
        if let Ok(index) = self
            .buckets
            .binary_search_by_key(&target, |(target, _)| *target)
        {
            self.buckets[index].1.insert(pool, ctx, candidate);
        }
    }

    #[inline(always)]
    fn can_prune(
        &self,
        lower_bonus_x2: u32,
        upper: u64,
        bonus_reach: Option<&BonusReach>,
        start: usize,
        depth: usize,
        bonus_x10: u32,
    ) -> bool {
        let max_bonus_x2 = (upper >> 32) as u32;
        let live_upper = upper as u32 as u64;
        let remaining = DECK_SIZE - depth.min(DECK_SIZE);
        // The subtree can only ever produce buckets that are (a) already
        // populated and still under their live threshold, or (b) empty but
        // reachable by some combination of the remaining cards. Anything else
        // is provably dead weight and gets pruned.
        let mut satisfiable = false;
        for (target, tracker) in &self.buckets {
            if *target < lower_bonus_x2 {
                continue;
            }
            if *target > max_bonus_x2 {
                break;
            }
            let threshold = tracker.threshold();
            if threshold == 0 {
                let Some(reach) = bonus_reach else {
                    satisfiable = true;
                    continue;
                };
                // Exact tier membership uses tenths directly, not the rounded
                // half-percent ranking key. Subtraction must not turn an
                // already-overshot target into a spurious zero requirement.
                if let Some(needed) = target.saturating_mul(5).checked_sub(bonus_x10)
                    && reach.any_in_range(start, remaining, needed, needed)
                {
                    satisfiable = true;
                }
                continue;
            }
            // Bucket thresholds carry the bucket id in the high 32 bits and
            // live_score in the low 32 bits.  Comparing a live-score ceiling
            // against the full encoded threshold makes every populated bucket
            // look unreachable and can discard a later, better deck in the same
            // exact bonus tier.
            let live_threshold = threshold as u32 as u64;
            if live_upper >= live_threshold {
                satisfiable = true;
            }
        }
        !satisfiable
    }

    fn into_vec(self) -> Vec<DeckResult> {
        self.buckets
            .into_iter()
            .rev()
            .flat_map(|(_, tracker)| tracker.into_vec())
            .collect()
    }
}

#[inline(always)]
fn partial_bonus_add(
    pool: &CardPool,
    ctx: &SearchContext,
    card: CardIdx,
    is_leader: bool,
    _limited_count: u8,
) -> (u32, u8) {
    // Free roles may be permuted at a leaf. Counting only the first limited
    // cards of the traversal prefix can underestimate a different legal order.
    // Counting every selected limited amount is an order-independent upper
    // relaxation; the exact evaluator alone applies the event's cap.
    let mut bonus = pool.event_bonus(card).total_ceil();
    if ctx.is_final_chapter && is_leader {
        bonus += ctx.leader_bonus_upper_at(card.raw());
    }
    (bonus, 0)
}

#[inline(always)]
fn highest_upper_bound_lane(upper_bounds: &[u64; EP_SHADOW_BLOCK_WIDTH], mask: u16) -> usize {
    let mut remaining = mask;
    let mut best_lane = remaining.trailing_zeros() as usize;
    let mut best = upper_bounds[best_lane];
    remaining &= remaining - 1;
    while remaining != 0 {
        let lane = remaining.trailing_zeros() as usize;
        let upper = upper_bounds[lane];
        if upper > best {
            best = upper;
            best_lane = lane;
        }
        remaining &= remaining - 1;
    }
    best_lane
}

#[cfg(test)]
pub(crate) fn dfs_search_power_len_for_test(
    pool: &CardPool,
    suffix: &SuffixBound,
    target_len: usize,
    top_k: usize,
    ctx: &SearchContext,
) -> Vec<DeckResult> {
    let mut tracker = TopKTracker::new(top_k);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    recurse_power_len_for_test(
        pool,
        suffix,
        target_len,
        0,
        0,
        &mut deck,
        UsedSet::new(),
        PartialDeck::default(),
        &mut tracker,
        ctx,
    );
    tracker.into_vec()
}

#[cfg(test)]
fn recurse_power_len_for_test(
    pool: &CardPool,
    suffix: &SuffixBound,
    target_len: usize,
    depth: usize,
    start: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    used: UsedSet,
    partial: PartialDeck,
    tracker: &mut TopKTracker,
    ctx: &SearchContext,
) {
    if depth == target_len {
        tracker.insert(pool, ctx, DeckResult::new(*deck, partial.power as u64));
        return;
    }

    let threshold = tracker.threshold();
    if threshold != 0 {
        let upper_bound =
            suffix.upper_bound_for_slots(target_len.saturating_sub(depth), &used, &partial);
        if upper_bound < threshold {
            return;
        }
    }

    let mut dense = start;
    while dense < pool.count() {
        let card = CardIdx::new(dense as u16);
        dense += 1;
        let char_id = pool.char_id(card);
        if ctx.enforce_char_uniqueness && used.contains(char_id) {
            continue;
        }

        unsafe {
            *deck.get_unchecked_mut(depth) = card;
        }
        let mut next_used = used;
        next_used.insert(char_id);
        recurse_power_len_for_test(
            pool,
            suffix,
            target_len,
            depth + 1,
            dense,
            deck,
            next_used,
            PartialDeck {
                power: partial.power + pool.power_max(card),
                ..partial
            },
            tracker,
            ctx,
        );
    }
}
