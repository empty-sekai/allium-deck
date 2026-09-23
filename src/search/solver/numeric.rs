//! Exact bounded search for numeric objectives and constrained power queries.
use super::power::search_power_scenarios;
use crate::pool::{CardIdx, CardPool};
use crate::search::DeckResult;
use crate::search::budget::SearchBudget;
use crate::search::{
    SearchContext, SearchParams, SearchStats, TopKTracker, evaluate, placement, tuning,
};
use crate::types::{DECK_SIZE, ScoreTarget};

pub(crate) fn search_simple_target(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    // The unconstrained maximizing Power case has a stronger exact 49-scenario
    // additive DP.  Every other Power/Skill request uses the proof-carrying B&B
    // below; heuristic quality-prefix truncation is deliberately forbidden.
    if matches!(ctx.target, ScoreTarget::Power)
        && !ctx.minimize
        && ctx.enforce_char_uniqueness
        && ctx.fixed_card_ids.is_empty()
        && ctx.fixed_character_ids.is_empty()
        && ctx.forced_leader_character_id.is_none()
        && ctx.multi_live_score_up_lower_bound.is_none()
        && ctx.power_total_cap.is_none()
    {
        return search_power_scenarios(pool, ctx, params, budget);
    }

    search_simple_target_exact(pool, ctx, params, budget)
}

#[derive(Clone, Copy, Default)]
struct SimpleRelaxedPartial {
    power_max_sum: u32,
    power_min_sum: u32,
    skill_max_sum: u32,
    leader_skill_max: u32,
}

struct SimpleExactState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    /// Every card, best-first by [`Self::card_value`].
    cards: Vec<CardIdx>,
    card_power_min: Vec<u32>,
    minimize: bool,
    /// Slots `0..fixed_prefix` are fixed roles; the rest are free.
    fixed_prefix: usize,
    /// For each position, the smallest distinct game ids of `cards[pos..]`
    /// in ascending order, padded with `u16::MAX`.
    suffix_small_ids: Vec<[u16; SMALL_IDS]>,
    global_power_max: u32,
    global_power_min: u32,
    global_skill_max: u32,
    bounds_enabled: bool,
    tracker: TopKTracker,
    stats: SearchStats,
    budget: &'a mut SearchBudget,
}

fn search_simple_target_exact(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
    let mut cards = pool.indices().collect::<Vec<_>>();
    let card_power_min = pool
        .indices()
        .map(|card| {
            let values = pool.power_values(card);
            let lut = pool.power_lut(card);
            (0..8)
                .map(|idx| evaluate::decode_u18(values, lut, idx))
                .min()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    cards.sort_unstable_by(|left, right| {
        let ordering = match ctx.target {
            ScoreTarget::Power if minimize => {
                card_power_min[left.raw()].cmp(&card_power_min[right.raw()])
            }
            ScoreTarget::Power => pool.power_max(*right).cmp(&pool.power_max(*left)),
            ScoreTarget::Skill => pool.skill_max(*right).cmp(&pool.skill_max(*left)),
            _ => std::cmp::Ordering::Equal,
        };
        ordering.then_with(|| left.raw().cmp(&right.raw()))
    });
    let global_power_max = pool
        .indices()
        .map(|card| pool.power_max(card))
        .max()
        .unwrap_or(0);
    let global_power_min = card_power_min.iter().copied().min().unwrap_or(0);
    let global_skill_max = pool
        .indices()
        .map(|card| pool.skill_max(card) as u32)
        .max()
        .unwrap_or(0);
    let mut state = SimpleExactState {
        pool,
        ctx,
        cards,
        card_power_min,
        minimize,
        fixed_prefix: (ctx.fixed_card_ids.len() + ctx.fixed_character_ids.len()).min(DECK_SIZE),
        suffix_small_ids: Vec::new(),
        global_power_max,
        global_power_min,
        global_skill_max,
        bounds_enabled: tuning::SearchTuning::load().bounds,
        tracker: TopKTracker::new(params.top_k),
        stats: SearchStats::default(),
        budget,
    };
    state.suffix_small_ids = suffix_small_ids(pool, &state.cards);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut selected_game_ids = [u16::MAX; DECK_SIZE];
    state.recurse(
        0,
        0,
        0,
        &mut deck,
        &mut selected_game_ids,
        SimpleRelaxedPartial::default(),
    );
    state.stats.deadline_hit = state.budget.hit;
    state.stats.finalize();
    (state.tracker.into_vec(), state.stats)
}

impl SimpleExactState<'_> {
    /// The per-card objective bound the cards are ordered by: resolved power
    /// maximum or minimum, or the skill maximum.
    #[inline(always)]
    fn card_value(&self, card: CardIdx) -> u32 {
        match self.ctx.target {
            ScoreTarget::Power if self.minimize => self.card_power_min[card.raw()],
            ScoreTarget::Power => self.pool.power_max(card),
            _ => u32::from(self.pool.skill_max(card)),
        }
    }

    /// Relaxed value of `slots` free picks taken from `cards[pos..]`, as
    /// `(sum, first)`. Cards are ordered best-first by [`Self::card_value`], so
    /// the first card of a character is that character's best remaining value.
    /// With unique characters a completion uses `slots` distinct unused
    /// characters, so its sum is bounded by the first `slots` distinct values;
    /// `first` bounds (for minimization: underestimates) every single pick.
    /// `None` means fewer than `slots` characters remain.
    fn frontier(&self, pos: usize, used_chars: u32, slots: usize) -> Option<(u32, u32)> {
        let first = self.card_value(*self.cards.get(pos)?);
        if !self.ctx.enforce_char_uniqueness {
            return (self.cards.len() - pos >= slots)
                .then(|| (first.saturating_mul(slots as u32), first));
        }
        let mut seen = used_chars;
        let mut sum = 0u32;
        let mut taken = 0usize;
        for &card in &self.cards[pos..] {
            if taken == slots {
                break;
            }
            let bit = 1u32 << self.pool.char_id(card);
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;
            sum = sum.saturating_add(self.card_value(card));
            taken += 1;
        }
        (taken == slots).then_some((sum, first))
    }

    /// Objective bound (lower bound for Power minimization) of every deck
    /// that adds `slots` more cards worth at most `sum` in total, none worth
    /// more than `best`, to `partial`.
    #[inline(always)]
    fn relaxed_objective(&self, partial: SimpleRelaxedPartial, sum: u32, best: u32) -> u64 {
        match self.ctx.target {
            ScoreTarget::Power if self.minimize => {
                let lower = partial
                    .power_min_sum
                    .saturating_add(sum)
                    .saturating_add(self.ctx.honor_bonus);
                self.ctx.clamp_power_total(lower) as u64
            }
            ScoreTarget::Power => {
                let upper = partial
                    .power_max_sum
                    .saturating_add(sum)
                    .saturating_add(self.ctx.honor_bonus);
                self.ctx.clamp_power_total(upper) as u64
            }
            _ => {
                let total = partial.skill_max_sum.saturating_add(sum);
                let leader = partial.leader_skill_max.max(best);
                (2u64 * total as u64).saturating_add(8u64 * leader as u64)
            }
        }
    }

    /// Whether a relaxed objective cannot reach the current K-th result.
    #[inline(always)]
    fn beyond_cutoff(&self, relaxed: u64, cutoff: u64) -> bool {
        if self.minimize {
            relaxed > cutoff
        } else {
            relaxed < cutoff
        }
    }

    /// Whether a node whose remaining slots are all free and draw from
    /// `cards[pos..]` can be discarded: no completion exists, its frontier
    /// bound misses the cutoff, or it can at best tie the cutoff while every
    /// completion's public set is larger than the K-th result's.
    #[inline(always)]
    fn frontier_can_prune(
        &self,
        depth: usize,
        pos: usize,
        used_chars: u32,
        selected_game_ids: &[u16],
        partial: SimpleRelaxedPartial,
    ) -> bool {
        if !self.bounds_enabled || depth < self.fixed_prefix || depth == DECK_SIZE {
            return false;
        }
        if !matches!(self.ctx.target, ScoreTarget::Power | ScoreTarget::Skill) {
            return false;
        }
        let slots = DECK_SIZE - depth;
        let Some((sum, best)) = self.frontier(pos, used_chars, slots) else {
            return true;
        };
        let Some(cutoff) = self.tracker.cutoff() else {
            return false;
        };
        let relaxed = self.relaxed_objective(partial, sum, best);
        if self.beyond_cutoff(relaxed, cutoff) {
            return true;
        }
        relaxed == cutoff
            && self.tracker.cutoff_public_set().is_some_and(|kth| {
                smallest_public_set(selected_game_ids, &self.suffix_small_ids[pos], slots)
                    .is_none_or(|smallest| smallest > kth)
            })
    }

    #[inline(always)]
    fn bound_can_prune(&self, depth: usize, partial: SimpleRelaxedPartial) -> bool {
        if !self.bounds_enabled {
            return false;
        }
        let Some(threshold) = self.tracker.cutoff() else {
            return false;
        };
        let slots = DECK_SIZE - depth;
        match self.ctx.target {
            ScoreTarget::Power if self.ctx.minimize => {
                // Every resolved card power is at least the minimum of that
                // card's eight precomputed power contexts.  Reusing the global
                // minimum for every free slot only relaxes constraints further,
                // hence this is an admissible LOWER bound for minimization.
                let lower_raw = partial
                    .power_min_sum
                    .saturating_add(self.global_power_min.saturating_mul(slots as u32))
                    .saturating_add(self.ctx.honor_bonus);
                let lower = self.ctx.clamp_power_total(lower_raw) as u64;
                lower > threshold
            }
            ScoreTarget::Power => {
                // Per-card power_max is an independent relaxation of unit/attr
                // coupling. Reusing the global maximum ignores uniqueness and
                // card reuse, so it can only make the upper bound larger.
                let upper_raw = partial
                    .power_max_sum
                    .saturating_add(self.global_power_max.saturating_mul(slots as u32))
                    .saturating_add(self.ctx.honor_bonus);
                let upper = self.ctx.clamp_power_total(upper_raw) as u64;
                upper < threshold
            }
            ScoreTarget::Skill => {
                // The encoded Skill target is 10*leader + 2*sum(other four)
                // = 2*sum(all) + 8*leader. skill_max is a per-card upper bound
                // for normal/unit-count/diff/reference resolution, and ignoring
                // uniqueness/reuse is again a relaxation.
                let total_skill = partial
                    .skill_max_sum
                    .saturating_add(self.global_skill_max.saturating_mul(slots as u32));
                let leader = partial.leader_skill_max.max(self.global_skill_max);
                let upper = (2u64 * total_skill as u64).saturating_add(8u64 * leader as u64);
                upper < threshold
            }
            _ => false,
        }
    }

    #[inline(always)]
    fn timed_out(&mut self) -> bool {
        self.budget.expired_sampled()
    }

    #[allow(clippy::too_many_arguments)]
    fn recurse(
        &mut self,
        depth: usize,
        min_free_pos: usize,
        used_chars: u32,
        deck: &mut [CardIdx; DECK_SIZE],
        selected_game_ids: &mut [u16; DECK_SIZE],
        partial: SimpleRelaxedPartial,
    ) {
        self.stats.visited_nodes = self.stats.visited_nodes.wrapping_add(1);
        if self.timed_out() {
            return;
        }
        if depth == DECK_SIZE {
            self.stats.leaf_nodes += 1;
            if let Some(candidate) = placement::evaluate_candidate(self.pool, self.ctx, deck) {
                self.tracker.insert(self.pool, self.ctx, candidate);
            }
            return;
        }
        if self.bound_can_prune(depth, partial)
            || self.frontier_can_prune(
                depth,
                min_free_pos,
                used_chars,
                &selected_game_ids[..depth],
                partial,
            )
        {
            self.stats.ub_prunes += 1;
            return;
        }
        // A free pick at `pos` is followed only by picks at later positions,
        // none better than it; that child bound never improves as `pos`
        // advances, so the first child beyond the cutoff ends the loop.
        let monotone_break = self.bounds_enabled
            && !is_free_slot_blocked(self.fixed_prefix, depth)
            && matches!(self.ctx.target, ScoreTarget::Power | ScoreTarget::Skill);

        let is_fixed = self.ctx.is_fixed_slot(depth);
        let mut pos = if is_fixed { 0 } else { min_free_pos };
        while pos < self.cards.len() {
            if self.budget.expired_sampled() {
                return;
            }
            if !is_fixed && self.cards.len() - pos < DECK_SIZE - depth {
                break;
            }
            let card = self.cards[pos];
            pos += 1;
            let game_id = self.pool.game_id(card);
            if monotone_break && let Some(cutoff) = self.tracker.cutoff() {
                let value = self.card_value(card);
                let slots = DECK_SIZE - depth;
                let relaxed =
                    self.relaxed_objective(partial, value.saturating_mul(slots as u32), value);
                if self.beyond_cutoff(relaxed, cutoff) {
                    self.stats.mono_break_prunes += 1;
                    break;
                }
                // A child that can at best tie the cutoff must also beat the
                // K-th public set; the rest of its picks come after `pos`.
                if relaxed == cutoff
                    && let Some(kth) = self.tracker.cutoff_public_set()
                {
                    let mut with_card = *selected_game_ids;
                    with_card[depth] = game_id;
                    if smallest_public_set(
                        &with_card[..=depth],
                        &self.suffix_small_ids[pos],
                        slots - 1,
                    )
                    .is_none_or(|smallest| smallest > kth)
                    {
                        self.stats.ub_prunes += 1;
                        continue;
                    }
                }
            }
            if selected_game_ids[..depth].contains(&game_id) {
                self.stats.feasibility_prunes += 1;
                continue;
            }
            if self
                .ctx
                .fixed_card_at(depth)
                .is_some_and(|required| required != game_id)
            {
                self.stats.feasibility_prunes += 1;
                continue;
            }
            let char_id = self.pool.char_id(card);
            let fixed_char = self.ctx.fixed_character_at(depth);
            if fixed_char.is_some_and(|required| required != char_id) {
                self.stats.feasibility_prunes += 1;
                continue;
            }
            if self.ctx.enforce_char_uniqueness && used_chars & (1u32 << char_id) != 0 {
                // Preserve the public fixed-character semantics: an explicitly
                // repeated fixed character may occupy another fixed slot.
                if fixed_char != Some(char_id) {
                    self.stats.feasibility_prunes += 1;
                    continue;
                }
            }

            deck[depth] = card;
            selected_game_ids[depth] = game_id;
            let next = SimpleRelaxedPartial {
                power_max_sum: partial
                    .power_max_sum
                    .saturating_add(self.pool.power_max(card)),
                power_min_sum: partial
                    .power_min_sum
                    .saturating_add(self.card_power_min[card.raw()]),
                skill_max_sum: partial
                    .skill_max_sum
                    .saturating_add(self.pool.skill_max(card) as u32),
                leader_skill_max: partial
                    .leader_skill_max
                    .max(self.pool.skill_max(card) as u32),
            };
            let next_min_free = if is_fixed { min_free_pos } else { pos };
            self.recurse(
                depth + 1,
                next_min_free,
                used_chars | (1u32 << char_id),
                deck,
                selected_game_ids,
                next,
            );
        }
    }
}

/// Whether slot `depth` is still one of the fixed roles, whose candidates are
/// scanned from the start of the card order rather than from the frontier.
#[inline(always)]
fn is_free_slot_blocked(fixed_prefix: usize, depth: usize) -> bool {
    depth < fixed_prefix
}

/// Distinct ids kept per suffix: enough for four free slots after skipping
/// up to five ids already in the deck.
const SMALL_IDS: usize = 2 * DECK_SIZE;

fn suffix_small_ids(pool: &CardPool, cards: &[CardIdx]) -> Vec<[u16; SMALL_IDS]> {
    let mut table = vec![[u16::MAX; SMALL_IDS]; cards.len() + 1];
    for pos in (0..cards.len()).rev() {
        let mut ids = table[pos + 1];
        let id = pool.game_id(cards[pos]);
        if !ids.contains(&id) && id < ids[SMALL_IDS - 1] {
            ids[SMALL_IDS - 1] = id;
            ids.sort_unstable();
        }
        table[pos] = ids;
    }
    table
}

/// Lexicographically smallest sorted public set of any deck made of the
/// `selected` ids and `slots` more distinct ids taken from `small_ids` (the
/// smallest distinct ids available). Taking the smallest unused ids minimizes
/// every rank of the sorted set at once; ignoring which of them can be combined
/// only makes the result smaller. `None` when too few ids remain.
fn smallest_public_set(
    selected: &[u16],
    small_ids: &[u16; SMALL_IDS],
    slots: usize,
) -> Option<[u16; DECK_SIZE]> {
    let mut set = [u16::MAX; DECK_SIZE];
    set[..selected.len()].copy_from_slice(selected);
    let mut len = selected.len();
    for &id in small_ids {
        if len == selected.len() + slots {
            break;
        }
        if id == u16::MAX {
            return None;
        }
        if !selected.contains(&id) {
            set[len] = id;
            len += 1;
        }
    }
    if len < selected.len() + slots {
        return None;
    }
    set.sort_unstable();
    Some(set)
}
