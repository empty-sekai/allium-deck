use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, LiveType, ScoreTarget};

use super::bonus_reach::BonusReach;
use super::context::SearchContext;
use super::evaluate::leaf_evaluate_checked;
use super::suffix::{PartialDeck, SuffixBound, UsedSet};
use super::types::{DeckResult, SearchParams};
use super::warm_start::sorted_final_chapter_leaders;

const EP_DENSE_BREAK_STRIDE: usize = 4;
const EP_SHADOW_BLOCK_WIDTH: usize = 16;
const EP_SHADOW_MIN_DEPTH: usize = 3;

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

pub(crate) fn dfs_search_seeded(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seed: Option<DeckResult>,
) -> Vec<DeckResult> {
    let seeds = seed.into_iter().collect::<Vec<_>>();
    let (results, _) = dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None, None);
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
    dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None, None)
}

pub(crate) fn dfs_search_instrumented_with_seeds(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seeds: Vec<DeckResult>,
) -> (Vec<DeckResult>, SearchStats) {
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
        bonus_reach,
        node_count: 0,
        deadline_hit: false,
        stats: SearchStats::default(),
        avx512_candidate_mask: crate::simd::avx512_available(),
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

    let stats = state.stats.clone();
    (tracker.into_vec(), stats)
}

struct SearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    deadline: Option<Instant>,
    tracker: &'a mut SearchTracker,
    /// Bonus-target reachability bitsets; `None` on every non-bucket path.
    bonus_reach: Option<&'a BonusReach>,
    node_count: u64,
    deadline_hit: bool,
    stats: SearchStats,
    avx512_candidate_mask: bool,
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
        bonus_x10: u32,
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
            // World Bloom 的加成合计走 limited-count 分支，与逐卡求和模型不一致，
            // 该场景保持旧行为（不做可达性剪枝）。
            let reach = if self.ctx.is_world_bloom {
                None
            } else {
                self.bonus_reach
            };
            if self
                .tracker
                .bonus_can_prune(lower_bonus_x2, upper, reach, start, depth, bonus_x10)
            {
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
                } else if matches!(self.ctx.target, ScoreTarget::Score) {
                    self.suffix.upper_bound_score_noevent(
                        self.pool,
                        &deck[..depth],
                        &used,
                        &partial,
                        DECK_SIZE - depth,
                    )
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
                    self.recurse_simple(depth, start, deck, used, partial, fixed_leader, bonus_x10);
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
                if ceil <= threshold {
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
                        mask_block = u16::MAX;
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
                if dense_upper <= threshold || slots < 3 {
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
            if dense_ub_global <= threshold {
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
                if global_ub <= threshold {
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

            let new_threshold = self.tracker.threshold();
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
                    if ceil <= threshold {
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
                threshold = threshold.max(self.tracker.threshold());

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
                    let new_threshold = self.tracker.threshold();
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
            if let Some(score) = leaf_evaluate_checked(self.pool, self.ctx, deck) {
                self.tracker.insert(DeckResult::new(*deck, score));
            }
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
                bonus_x10 + self.pool.event_bonus(card).total_x10() as u32,
            );
        }
    }

    #[inline(always)]
    fn timed_out(&mut self) -> bool {
        // 粘性中止：一旦过线，后续所有调用都立刻返回 true，整棵搜索树逐层退出；
        // 否则 1024 抽检里未命中的调用会继续推进搜索，timeout_ms 形同虚设。
        if self.deadline_hit {
            return true;
        }
        let Some(deadline) = self.deadline else {
            return false;
        };
        self.node_count = self.node_count.wrapping_add(1);
        if self.node_count & 1023 != 0 {
            return false;
        }
        if Instant::now() >= deadline {
            self.deadline_hit = true;
            return true;
        }
        false
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
                // round(x10 / 5) == x2 holds exactly for
                // x10 in [5*x2 - 2, 5*x2 + 2].
                let center = target.saturating_mul(5);
                let lo = center.saturating_sub(2);
                let hi = center.saturating_add(2);
                // reach covers only the remaining cards' sum, so the already
                // picked bonus is subtracted from the target window.
                if reach.any_in_range(
                    start,
                    remaining,
                    lo.saturating_sub(bonus_x10),
                    hi.saturating_sub(bonus_x10),
                ) {
                    satisfiable = true;
                }
                continue;
            }
            if live_upper >= threshold {
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
    limited_count: u8,
) -> (u32, u8) {
    let eb = pool.event_bonus(card);
    if !ctx.is_final_chapter {
        return (eb.total_ceil(), 0);
    }
    let exact = pool.event_bonus_exact(card);
    let mut bonus = exact.base_ceil();
    let mut limited_inc = 0u8;
    if (limited_count as usize) < ctx.card_bonus_count_limit && exact.limited_x10() > 0 {
        bonus += exact.limited_ceil();
        limited_inc = 1;
    }
    if is_leader {
        bonus += ctx.leader_honor_bonus_at(card.raw());
        bonus += ctx.leader_limit_bonus_at(card.raw());
    }
    (bonus, limited_inc)
}

pub(super) struct TopKTracker {
    top_k: usize,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
    keys: Vec<[u16; DECK_SIZE]>,
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

impl TopKTracker {
    pub(super) fn new(top_k: usize, pool: &CardPool) -> Self {
        Self {
            top_k,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
            keys: Vec::with_capacity(top_k),
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
        if self.results.len() >= self.top_k
            && let Some(last) = self.results.last()
            && !deck_result_cmp(&candidate, last).is_lt()
        {
            return;
        }
        let candidate_key = self.game_card_set_key(&candidate);
        if let Some(existing_pos) = self.keys.iter().position(|key| *key == candidate_key) {
            if !deck_result_cmp(&candidate, &self.results[existing_pos]).is_lt() {
                return;
            }
            self.results.remove(existing_pos);
            self.keys.remove(existing_pos);
        }
        let pos = self
            .results
            .iter()
            .position(|existing| deck_result_cmp(&candidate, existing).is_lt())
            .unwrap_or(self.results.len());
        self.results.insert(pos, candidate);
        self.keys.insert(pos, candidate_key);
        if self.results.len() > self.top_k {
            self.results.pop();
            self.keys.pop();
        }
    }

    pub(super) fn into_vec(self) -> Vec<DeckResult> {
        self.results
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
