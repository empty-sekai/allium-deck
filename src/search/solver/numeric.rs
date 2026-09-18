//! Exact bounded search for numeric objectives and constrained power queries.
use super::power::search_power_scenarios;
use crate::pool::{CardIdx, CardPool};
use crate::search::DeckResult;
use crate::search::{
    SearchContext, SearchParams, SearchStats, SimpleTopKTracker, evaluate, placement, tuning,
};
use crate::types::{DECK_SIZE, ScoreTarget};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

pub(crate) fn search_simple_target(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
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
        return search_power_scenarios(pool, ctx, params);
    }

    search_simple_target_exact(pool, ctx, params)
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
    cards: Vec<CardIdx>,
    card_power_min: Vec<u32>,
    global_power_max: u32,
    global_power_min: u32,
    global_skill_max: u32,
    bounds_enabled: bool,
    tracker: SimpleTopKTracker,
    stats: SearchStats,
    deadline: Option<Instant>,
}

fn search_simple_target_exact(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
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
    let tuning = tuning::SearchTuning::load();
    let mut state = SimpleExactState {
        pool,
        ctx,
        cards,
        card_power_min,
        global_power_max,
        global_power_min,
        global_skill_max,
        bounds_enabled: tuning.bounds && tuning.simple_bound,
        tracker: SimpleTopKTracker::new(params.top_k, minimize, pool),
        stats: SearchStats::default(),
        deadline: (params.timeout_ms != 0)
            .then(|| Instant::now() + Duration::from_millis(params.timeout_ms)),
    };
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
    (state.tracker.into_vec(), state.stats)
}

impl SimpleExactState<'_> {
    #[inline(always)]
    fn bound_can_prune(&self, depth: usize, partial: SimpleRelaxedPartial) -> bool {
        if !self.bounds_enabled {
            return false;
        }
        let Some(threshold) = self.tracker.threshold() else {
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
        if self.stats.deadline_hit {
            return true;
        }
        let Some(deadline) = self.deadline else {
            return false;
        };
        if self.stats.visited_nodes & 1023 != 0 {
            return false;
        }
        if Instant::now() >= deadline {
            self.stats.deadline_hit = true;
            return true;
        }
        false
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
                self.tracker.insert(candidate);
            }
            return;
        }
        if self.bound_can_prune(depth, partial) {
            self.stats.ub_prunes += 1;
            return;
        }

        let is_fixed = self.ctx.is_fixed_slot(depth);
        let mut pos = if is_fixed { 0 } else { min_free_pos };
        while pos < self.cards.len() {
            if !is_fixed && self.cards.len() - pos < DECK_SIZE - depth {
                break;
            }
            let card = self.cards[pos];
            pos += 1;
            let game_id = self.pool.game_id(card);
            if selected_game_ids[..depth].contains(&game_id) {
                continue;
            }
            if self
                .ctx
                .fixed_card_at(depth)
                .is_some_and(|required| required != game_id)
            {
                continue;
            }
            let char_id = self.pool.char_id(card);
            let fixed_char = self.ctx.fixed_character_at(depth);
            if fixed_char.is_some_and(|required| required != char_id) {
                continue;
            }
            if self.ctx.enforce_char_uniqueness && used_chars & (1u32 << char_id) != 0 {
                // Preserve the public fixed-character semantics: an explicitly
                // repeated fixed character may occupy another fixed slot.
                if fixed_char != Some(char_id) {
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
