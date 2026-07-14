use std::time::{Duration, Instant};

use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::leaf_evaluate_checked;
use super::suffix::{PartialDeck, SuffixBound, UsedSet};
use super::types::{DeckResult, SearchParams};
use super::warm_start::sorted_final_chapter_leaders;

/// DFS 搜索统计。
#[derive(Clone, Debug, Default)]
pub struct SearchStats {
    pub leaf_nodes: u64,
    pub ub_prunes: u64,
    pub leader_prunes: u64,
    pub ep_candidates: u64,
    pub ep_break_prunes: u64,
    pub ep_continue_prunes: u64,
    pub ep_explored: u64,
    pub mono_break_prunes: u64,
}

/// 执行精确 DFS/B&B 搜索。
pub fn dfs_search(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> Vec<DeckResult> {
    dfs_search_seeded(pool, ctx, suffix, params, None)
}

/// 单次 DFS 为每个精确活动加成档位保留独立 Top-K。
pub fn dfs_search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    targets: &[i32],
) -> (Vec<DeckResult>, SearchStats) {
    dfs_search_seeded_inner(pool, ctx, suffix, params, Vec::new(), Some(targets))
}

pub(crate) fn dfs_search_seeded(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seed: Option<DeckResult>,
) -> Vec<DeckResult> {
    let seeds = seed.into_iter().collect::<Vec<_>>();
    let (results, _) = dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None);
    results
}

pub fn dfs_search_instrumented(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seed: Option<DeckResult>,
) -> (Vec<DeckResult>, SearchStats) {
    let seeds = seed.into_iter().collect::<Vec<_>>();
    dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None)
}

pub(crate) fn dfs_search_instrumented_with_seeds(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seeds: Vec<DeckResult>,
) -> (Vec<DeckResult>, SearchStats) {
    dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None)
}

fn dfs_search_seeded_inner(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seeds: Vec<DeckResult>,
    bonus_targets: Option<&[i32]>,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let deadline = if params.timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(params.timeout_ms))
    };

    let mut tracker = match bonus_targets {
        Some(targets) => SearchTracker::Bonus(BonusBucketTracker::new(params.top_k, pool, targets)),
        None => SearchTracker::TopK(TopKTracker::new(params.top_k, pool)),
    };
    for seed_result in seeds {
        tracker.insert(seed_result);
    }

    let mut state = SearchState {
        pool,
        ctx,
        suffix,
        deadline,
        tracker: &mut tracker,
        node_count: 0,
        stats: SearchStats::default(),
    };
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
            let threshold = state.tracker.threshold();
            if threshold != 0 {
                let leader_global = state.suffix.upper_bound_with_depth(1, &used, &partial);
                let leader_dense = state
                    .suffix
                    .dense_suffix_ceiling(0, &partial, DECK_SIZE - 1);
                if leader_global.min(leader_dense) <= threshold {
                    state.stats.leader_prunes += 1;
                    continue;
                }
            }
            state.recurse(1, 0, &mut deck, used, partial, Some(leader));
        }
    } else {
        state.recurse(
            0,
            0,
            &mut deck,
            UsedSet::new(),
            PartialDeck::default(),
            None,
        );
    }

    let stats = state.stats.clone();
    drop(state);
    (tracker.into_vec(), stats)
}

struct SearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    deadline: Option<Instant>,
    tracker: &'a mut SearchTracker,
    node_count: u64,
    stats: SearchStats,
}

impl SearchState<'_> {
    #[inline(always)]
    fn recurse(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
    ) {
        if self.timed_out() {
            return;
        }
        if depth == DECK_SIZE {
            self.stats.leaf_nodes += 1;
            if let Some(score) = leaf_evaluate_checked(self.pool, self.ctx, deck) {
                self.tracker.insert(DeckResult::new(*deck, score));
            }
            return;
        }

        if self.tracker.is_bonus() {
            let upper = self.suffix.upper_bound_with_depth(depth, &used, &partial);
            // partial.bonus 是逐卡 ceil 百分比；每张卡至多高估 0.5%，
            // 因此 2*ceil-depth 是精确 x2 bonus 的安全下界。
            let lower_bonus_x2 = partial.bonus.saturating_mul(2).saturating_sub(depth as u32);
            if self.tracker.bonus_can_prune(lower_bonus_x2, upper) {
                self.stats.ub_prunes += 1;
                return;
            }
        }

        let threshold = self.tracker.threshold();
        if threshold != 0 {
            let upper_bound =
                if matches!(self.ctx.target, ScoreTarget::Score) && self.ctx.has_event() {
                    let global = self.suffix.upper_bound_with_depth(depth, &used, &partial);
                    let dense =
                        self.suffix
                            .dense_suffix_ceiling(start, &partial, DECK_SIZE - depth);
                    global.min(dense)
                } else {
                    self.suffix.upper_bound_with_depth(depth, &used, &partial)
                };
            if upper_bound <= threshold {
                self.stats.ub_prunes += 1;
                return;
            }
        }

        let slots = DECK_SIZE - depth;

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
                    self.recurse_simple(depth, start, deck, used, partial, fixed_leader);
                }
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
                if ceil <= threshold {
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
        let pre = self.suffix.precompute_layer_score_noevent(&used, slots);
        let mut dense = start;
        while dense < self.pool.count() {
            if threshold != 0 {
                let ceil = self
                    .suffix
                    .score_noevent_dense_ceiling(dense, &partial, slots);
                if ceil <= threshold {
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
                let remaining_best_skill = if char_id == pre.best_skill_char {
                    pre.second_best_skill
                } else {
                    pre.best_unused_skill
                };
                let tight_leader = (partial.max_skill as u32)
                    .max(self.pool.skill_max(card) as u32)
                    .max(remaining_best_skill as u32);
                let ub = self
                    .suffix
                    .ceiling(tight_power, 0, tight_skill, tight_leader);
                if ub <= threshold {
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
        let pre = self.suffix.precompute_layer_ep(&used, slots);

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
                    )
                });

        let mut dense = start;
        while dense < self.pool.count() {
            if self.timed_out() {
                return;
            }
            if threshold != 0 && matches!(self.ctx.target, ScoreTarget::Score) {
                let ceil = if let Some(extra_bonus_ub) = partial_extra_bonus_ub {
                    self.suffix.dense_suffix_ceiling_with_extra(
                        dense,
                        &partial,
                        slots,
                        extra_bonus_ub,
                    )
                } else {
                    self.suffix.dense_suffix_ceiling(dense, &partial, slots)
                };
                if ceil <= threshold {
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

            let eb = self.pool.event_bonus(card);
            let card_bonus = eb.total_ceil();
            let card_base_bonus = eb.base_ceil();
            let card_limited_bonus = eb.limited_ceil();
            let card_power = self.pool.power_max(card);
            let card_skill = self.pool.skill_max(card);
            let card_skill_u32 = card_skill as u32;

            self.stats.ep_candidates += 1;

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

            let mut ub =
                self.suffix
                    .ceiling(tight_power, bonus_total_global, tight_skill, tight_leader);
            let dense_ub_global = self.suffix.dense_candidate_ceiling(
                dense,
                &partial,
                card_power,
                card_bonus,
                card_base_bonus,
                card_limited_bonus,
                card_skill_u32,
                slots,
            );
            ub = ub.min(dense_ub_global);
            if ub <= threshold {
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
                if refined <= threshold {
                    self.stats.ep_continue_prunes += 1;
                    continue;
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
            );

            let new_threshold = self.tracker.threshold();
            if new_threshold > threshold {
                threshold = new_threshold;
            }
        }
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
            if self.ctx.enforce_char_uniqueness && used.contains(char_id) {
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
            );
        }
    }

    #[inline(always)]
    fn timed_out(&mut self) -> bool {
        self.node_count = self.node_count.wrapping_add(1);
        if self.node_count & 1023 != 0 {
            return false;
        }
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    #[inline(always)]
    fn slot_matches(&self, depth: usize, card: CardIdx) -> bool {
        if self.ctx.is_final_chapter
            && depth > 0
            && !self.ctx.final_chapter_member_keep_at(card.raw())
        {
            return false;
        }
        if let Some(game_id) = self.ctx.fixed_card_at(depth) {
            if self.pool.game_id(card) != game_id {
                return false;
            }
        }
        if let Some(character_id) = self.ctx.fixed_character_at(depth) {
            if self.pool.char_id(card) != character_id {
                return false;
            }
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
    fn bonus_can_prune(&self, lower_bonus_x2: u32, upper: u64) -> bool {
        match self {
            Self::Bonus(tracker) => tracker.can_prune(lower_bonus_x2, upper),
            Self::TopK(_) => false,
        }
    }

    #[inline(always)]
    fn insert(&mut self, candidate: DeckResult) {
        match self {
            Self::TopK(tracker) => tracker.insert(candidate),
            Self::Bonus(tracker) => tracker.insert(candidate),
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
    fn new(top_k: usize, pool: &CardPool, targets: &[i32]) -> Self {
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
                .map(|target| (target, TopKTracker::new(top_k, pool)))
                .collect(),
        }
    }

    #[inline(always)]
    fn insert(&mut self, candidate: DeckResult) {
        let target = (candidate.score >> 32) as u32;
        if let Ok(index) = self
            .buckets
            .binary_search_by_key(&target, |(target, _)| *target)
        {
            self.buckets[index].1.insert(candidate);
        }
    }

    #[inline(always)]
    fn can_prune(&self, lower_bonus_x2: u32, upper: u64) -> bool {
        let max_bonus_x2 = (upper >> 32) as u32;
        let live_upper = upper as u32 as u64;
        for (target, tracker) in &self.buckets {
            if *target < lower_bonus_x2 {
                continue;
            }
            if *target > max_bonus_x2 {
                break;
            }
            let threshold = tracker.threshold();
            if threshold == 0 || live_upper >= (threshold as u32 as u64) {
                return false;
            }
        }
        true
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
    limited_count: u8,
) -> (u32, u8) {
    let eb = pool.event_bonus(card);
    let mut bonus = eb.base_ceil();
    let mut limited_inc = 0u8;
    if !ctx.is_final_chapter || (limited_count as usize) < ctx.card_bonus_count_limit {
        if eb.limited_x2() > 0 {
            bonus += eb.limited_ceil();
            limited_inc = 1;
        }
    }
    if ctx.is_final_chapter && is_leader {
        bonus += ctx.leader_honor_bonus_at(card.raw());
        bonus += ctx.leader_limit_bonus_at(card.raw());
    }
    (bonus, limited_inc)
}

pub(super) struct TopKTracker {
    top_k: usize,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
}

impl TopKTracker {
    pub(super) fn new(top_k: usize, pool: &CardPool) -> Self {
        Self {
            top_k,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
        }
    }

    pub(super) fn threshold(&self) -> u64 {
        if self.results.len() < self.top_k {
            0
        } else {
            self.results.last().map(|result| result.score).unwrap_or(0)
        }
    }

    pub(super) fn insert(&mut self, candidate: DeckResult) {
        if let Some(existing_pos) = self
            .results
            .iter()
            .position(|existing| self.same_game_card_set(existing, &candidate))
        {
            if !deck_result_cmp(&candidate, &self.results[existing_pos]).is_lt() {
                return;
            }
            self.results.remove(existing_pos);
        }
        let pos = self
            .results
            .iter()
            .position(|existing| deck_result_cmp(&candidate, existing).is_lt())
            .unwrap_or(self.results.len());
        self.results.insert(pos, candidate);
        if self.results.len() > self.top_k {
            self.results.pop();
        }
    }

    pub(super) fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }

    pub(super) fn same_game_card_set(&self, left: &DeckResult, right: &DeckResult) -> bool {
        self.game_card_set_key(left) == self.game_card_set_key(right)
    }

    pub(super) fn game_card_set_key(&self, result: &DeckResult) -> [u16; 5] {
        let mut cards = result.cards.map(|card| self.game_ids[card.raw()]);
        cards.sort_unstable();
        cards
    }
}

#[inline(always)]
pub(super) fn deck_result_cmp(left: &DeckResult, right: &DeckResult) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.cards.cmp(&right.cards))
}

#[cfg(test)]
pub(crate) fn dfs_search_power_len_for_test(
    pool: &CardPool,
    suffix: &SuffixBound,
    target_len: usize,
    top_k: usize,
    ctx: &SearchContext,
) -> Vec<DeckResult> {
    let mut tracker = TopKTracker::new(top_k, pool);
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
        tracker.insert(DeckResult::new(*deck, partial.power as u64));
        return;
    }

    let threshold = tracker.threshold();
    if threshold != 0 {
        let upper_bound =
            suffix.upper_bound_for_slots(target_len.saturating_sub(depth), &used, &partial);
        if upper_bound <= threshold {
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
