use std::time::{Duration, Instant};

use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::{card_proxy_bonus, leaf_evaluate};
use super::suffix::{PartialDeck, SuffixBound, UsedSet};
use super::types::{DeckResult, SearchParams};

/// 执行精确 DFS/B&B 搜索。
pub fn dfs_search(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> Vec<DeckResult> {
    dfs_search_seeded(pool, ctx, suffix, params, None)
}

pub(crate) fn dfs_search_seeded(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    seed: Option<DeckResult>,
) -> Vec<DeckResult> {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return Vec::new();
    }

    let deadline = if params.timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(params.timeout_ms))
    };

    let mut tracker = TopKTracker::new(params.top_k);
    if let Some(seed_result) = seed {
        tracker.insert(seed_result);
    }

    let mut state = SearchState {
        pool,
        ctx,
        suffix,
        deadline,
        tracker: &mut tracker,
        node_count: 0,
    };
    let mut deck = [CardIdx::new(0); DECK_SIZE];

    if ctx.is_final_chapter {
        for leader in pool.indices() {
            if state.timed_out() {
                break;
            }
            deck[0] = leader;
            let mut used = UsedSet::new();
            used.insert(pool.char_id(leader));
            let partial = PartialDeck {
                power: pool.power_max(leader),
                skill: pool.skill_max(leader) as u32,
                bonus: card_proxy_bonus(pool, ctx, leader, true),
                max_skill: pool.skill_max(leader),
            };
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

    tracker.into_vec()
}

struct SearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    deadline: Option<Instant>,
    tracker: &'a mut TopKTracker,
    node_count: u64,
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
            let score = leaf_evaluate(self.pool, self.ctx, deck);
            self.tracker.insert(DeckResult::new(*deck, score));
            return;
        }

        let threshold = self.tracker.threshold();
        if threshold != 0 {
            let upper_bound = self.suffix.upper_bound_with_depth(depth, &used, &partial);
            if upper_bound <= threshold {
                return;
            }
        }

        // 预计算同层 suffix 分量（一次性，3×27 循环），供循环内廉价 break 使用
        let slots = DECK_SIZE - depth;
        let pre = self.suffix.precompute_layer(&used, slots);

        let mut dense = start;
        while dense < self.pool.count() {
            let card = CardIdx::new(dense as u16);
            dense += 1;
            if fixed_leader.is_some_and(|leader| leader == card) {
                continue;
            }

            let char_id = self.pool.char_id(card);
            if used.contains(char_id) {
                continue;
            }

            // Sound ceiling pruning：双层剪枝
            if threshold != 0 {
                let eb = self.pool.event_bonus(card);
                let card_bonus = eb.base_bonus as u32 + eb.limited_bonus as u32;
                let bonus_total = partial.bonus + card_bonus
                    + pre.suffix_bonus + pre.extra_bonus_ub;
                let sorted_by_bonus = self.ctx.event_type.is_some()
                    && !matches!(self.ctx.target, ScoreTarget::Power | ScoreTarget::Skill);

                // Layer 1: break — 仅用单调分量（排序键递减 + 层级常量）
                let break_power = if sorted_by_bonus {
                    partial.power + pre.suffix_power
                } else {
                    partial.power + self.pool.power_max(card) + pre.suffix_power_rest
                };
                let layer_leader = (partial.max_skill as u32).max(pre.best_unused_skill as u32);
                let break_ceiling = self.suffix.ceiling(
                    break_power, bonus_total, pre.skill_ub, layer_leader,
                );
                if break_ceiling <= threshold {
                    break;
                }

                // Layer 2: continue — 含候选卡实际 power/skill，更紧但不单调
                let tight_power = partial.power + self.pool.power_max(card) + pre.suffix_power_rest;
                let tight_skill = partial.skill + self.pool.skill_max(card) as u32 + pre.skill_ub_rest;
                let tight_leader = (partial.max_skill as u32).max(self.pool.skill_max(card) as u32);
                let tight_ceiling = self.suffix.ceiling(
                    tight_power, bonus_total, tight_skill, tight_leader,
                );
                if tight_ceiling <= threshold {
                    continue;
                }
            }

            unsafe {
                *deck.get_unchecked_mut(depth) = card;
            }
            let mut next_used = used;
            next_used.insert(char_id);
            let next_partial = PartialDeck {
                power: partial.power + self.pool.power_max(card),
                skill: partial.skill + self.pool.skill_max(card) as u32,
                bonus: partial.bonus + card_proxy_bonus(self.pool, self.ctx, card, false),
                max_skill: partial.max_skill.max(self.pool.skill_max(card)),
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
}

struct TopKTracker {
    top_k: usize,
    results: Vec<DeckResult>,
}

impl TopKTracker {
    fn new(top_k: usize) -> Self {
        Self {
            top_k,
            results: Vec::with_capacity(top_k),
        }
    }

    fn threshold(&self) -> u64 {
        if self.results.len() < self.top_k {
            0
        } else {
            self.results.last().map(|result| result.score).unwrap_or(0)
        }
    }

    fn insert(&mut self, candidate: DeckResult) {
        if self
            .results
            .iter()
            .any(|existing| existing.cards == candidate.cards)
        {
            return;
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

    fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }
}

#[inline(always)]
fn deck_result_cmp(left: &DeckResult, right: &DeckResult) -> std::cmp::Ordering {
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
        if used.contains(char_id) {
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
        );
    }
}
