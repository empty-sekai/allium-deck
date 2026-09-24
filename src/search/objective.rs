//! Admissible relaxation of the scoring objective over aggregate deck features.
//!
//! Every search ceiling eventually asks one question: given upper bounds on a
//! deck's total power, total bonus, total skill and leader skill, what is the
//! largest encoded objective any deck with those features can reach? The
//! answer depends only on the search context (target, live type, music and
//! event constants), never on the card pool, so one [`ObjectiveBound`] serves
//! every pool restriction of the same request. All integer steps round
//! outward, which keeps each ceiling admissible for the exact evaluator.
//!
//! The evaluator itself computes in `f64` and truncates. A fixed-point
//! coefficient here is `ceil(fl(c * 10^6))`, which can lie a few roundings
//! below the real coefficient; admissibility against the floating-point value
//! comes from the integer grid instead. The live numerator is an integer on a
//! `10^-6` grid and each event-point stage is a rational on a `10^-4` (or
//! coarser) grid, while the evaluator's accumulated relative rounding is at
//! most `28 * 2^-53`. Inside the numeric domain that pool construction
//! enforces (`handler::capacity::numeric_domain`) the rounding never reaches
//! the next grid point, so every truncated ceiling dominates the truncated
//! evaluator value. The full argument is "Numeric admissibility" in
//! `docs/pruning-proof.md`.

use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType, ScoreTarget};

use super::context::SearchContext;
use super::evaluate::calc_mysekai_internal;

/// Fixed-point scale of the live-score numerators.
pub(crate) const LIVE_SCORE_BOUND_SCALE: i64 = 1_000_000;

/// The live-score numerator `N` of a deck as a product of its power and an
/// affine rate. For power `P` (honor bonus excluded), skill sum `S` and
/// leader skill at most `L`,
///
/// `divisor * N <= (P + honor) * rate + constant`, with
/// `rate = intercept + leader * L + skill * (S + max(0, floor - 4L - S))`
///
/// and non-negative coefficients; `floor` is where a Multi rate reads the
/// teammates' score-up instead of the deck's skills. "Joint power-skill
/// ceiling" in `docs/pruning-proof.md` derives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveProduct {
    pub(crate) honor: i64,
    pub(crate) intercept: i64,
    pub(crate) leader: i64,
    pub(crate) skill: i64,
    pub(crate) constant: i64,
    pub(crate) divisor: i64,
    pub(crate) floor: i64,
}

impl LiveProduct {
    /// Rate of a deck whose skill sum is `skill` plus any non-negative
    /// further skill, taken at the further skill's zero.
    #[inline]
    pub(crate) fn rate(&self, skill: u32, leader: u32) -> i128 {
        let excess = (self.floor - 4 * i64::from(leader) - i64::from(skill)).max(0);
        i128::from(self.intercept)
            + i128::from(self.leader) * i128::from(leader)
            + i128::from(self.skill) * (i128::from(skill) + i128::from(excess))
    }

    /// Live-score ceiling of a deck whose `(P + honor) * rate` is at most
    /// `numerator / denominator`, for a positive denominator.
    #[inline]
    pub(crate) fn live(&self, numerator: i128, denominator: i128) -> u32 {
        let live = (numerator + i128::from(self.constant) * denominator)
            / (denominator * i128::from(self.divisor) * i128::from(LIVE_SCORE_BOUND_SCALE));
        live.clamp(0, i128::from(u32::MAX)) as u32
    }
}

/// Objective constants and the outward-rounded aggregate ceiling.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObjectiveBound {
    pub(super) target: ScoreTarget,
    pub(super) effective_live_type: LiveType,
    pub(super) has_event: bool,
    pub(super) music_rate_pct: u32,
    pub(super) boost_rate_pct: u32,
    /// base_rate × 1_000_000, ceil。
    pub(super) base_rate_1m: i64,
    /// (skill_rate_sum / 500) × 1_000_000, ceil。
    pub(super) srs_div500_1m: i64,
    /// Solo/Auto + Average：前 5 个 rate 的和 × 1_000_000, ceil。
    pub(super) avg_sum5_1m: i64,
    /// Solo/Auto + Average：leader 追加 slot 的 rate × 1_000_000, ceil。
    pub(super) avg_leader_rate_1m: i64,
    /// The six slot rates, largest first, each (rate / 100) × 1_000_000, ceil.
    pub(super) sorted_rates_1m: [i64; DECK_SIZE + 1],
    /// 5 × multi_teammate_score_up（Multi/Cheerful 专用）。
    pub(super) teammate_su_5x: i64,
    /// Multi/Cheerful: 75_000 (= 0.075 × 1M), 其他: 0。
    pub(super) active_1m_coeff: i64,
    pub(super) other_score: i32,
    /// Cheerful: 5750 + clamp(life, 500, 1000), 其他: 0。
    pub(super) life_rate_num: i32,
    pub(super) multi_teammate_power: Option<i32>,
    pub(super) live_skill_order: LiveSkillOrder,
    pub(super) honor_bonus: u32,
    pub(super) power_total_cap: Option<u32>,
}

impl ObjectiveBound {
    pub(crate) fn from_context(ctx: &SearchContext) -> Self {
        let base_rate: f64 = match ctx.effective_live_type() {
            LiveType::Auto | LiveType::ChallengeAuto => ctx.base_score_auto,
            LiveType::Multi | LiveType::Cheerful => ctx.base_score + ctx.fever_score * 0.5,
            _ => ctx.base_score,
        };
        let skill_rate_sum: f64 = ctx.skill_scores[match ctx.effective_live_type() {
            LiveType::Multi | LiveType::Cheerful => 1,
            LiveType::Auto | LiveType::ChallengeAuto => 2,
            _ => 0,
        }]
        .iter()
        .sum();
        let active_skill_rates = ctx.skill_scores[match ctx.effective_live_type() {
            LiveType::Multi | LiveType::Cheerful => 1,
            LiveType::Auto | LiveType::ChallengeAuto => 2,
            _ => 0,
        }];
        let avg_sum5 = active_skill_rates[..DECK_SIZE].iter().sum::<f64>();
        let avg_leader_rate = active_skill_rates[DECK_SIZE];
        let mut sorted_rates = active_skill_rates;
        sorted_rates.sort_unstable_by(|left, right| right.total_cmp(left));
        Self {
            target: ctx.target,
            effective_live_type: ctx.effective_live_type(),
            has_event: ctx.has_event(),
            music_rate_pct: ctx.music_rate_pct,
            boost_rate_pct: ctx.boost_rate_pct,
            base_rate_1m: (base_rate * 1_000_000.0).ceil() as i64,
            srs_div500_1m: (skill_rate_sum / 500.0 * 1_000_000.0).ceil() as i64,
            avg_sum5_1m: (avg_sum5 * 1_000_000.0).ceil() as i64,
            avg_leader_rate_1m: (avg_leader_rate * 1_000_000.0).ceil() as i64,
            sorted_rates_1m: sorted_rates.map(|rate| (rate / 100.0 * 1_000_000.0).ceil() as i64),
            teammate_su_5x: ctx
                .multi_teammate_score_up
                .map(|v| v as i64 * 5)
                .unwrap_or(0),
            active_1m_coeff: if matches!(
                ctx.effective_live_type(),
                LiveType::Multi | LiveType::Cheerful
            ) {
                75_000
            } else {
                0
            },
            other_score: ctx.other_score,
            life_rate_num: if matches!(ctx.effective_live_type(), LiveType::Cheerful) {
                5750 + ctx.life.clamp(500, 1000)
            } else {
                0
            },
            multi_teammate_power: ctx.multi_teammate_power,
            live_skill_order: ctx.live_skill_order,
            honor_bonus: ctx.honor_bonus,
            power_total_cap: ctx.power_total_cap,
        }
    }

    /// Score/no-event has a strictly live-score-ordered objective because its
    /// public key is `(live_score, live_score)`.  Bound pruning can compare the
    /// pre-division numerator against `threshold * 1_000_000`: for non-negative
    /// N, `floor(N / D) < T` iff `N < T * D`.
    #[inline(always)]
    pub(crate) fn score_noevent_live_numerator_ceiling(
        &self,
        power_ub: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> i64 {
        debug_assert!(matches!(self.target, ScoreTarget::Score));
        debug_assert!(!self.has_event);
        let power_ub = self.clamp_power_total(power_ub + self.honor_bonus);
        let numerator = self.calc_live_score_bound_numerator(power_ub, skill_ub, leader_ub);
        debug_assert!(numerator >= 0);
        numerator
    }

    #[cfg(test)]
    #[inline(always)]
    pub(crate) fn score_noevent_live_ceiling(
        &self,
        power_ub: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> u32 {
        (self.score_noevent_live_numerator_ceiling(power_ub, skill_ub, leader_ub)
            / LIVE_SCORE_BOUND_SCALE) as u32
    }

    /// Generic target-aware ceiling from admissible aggregate inputs. A
    /// MySekai ceiling is a [`mysekai_rank`], to be compared only with
    /// `TopKTracker::rank_threshold`.
    #[inline(always)]
    pub(crate) fn ceiling(
        &self,
        power_ub: u32,
        bonus_total: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> u64 {
        let power_ub = self.clamp_power_total(power_ub + self.honor_bonus);
        match self.target {
            ScoreTarget::Power => power_ub as u64,
            ScoreTarget::Skill => (2 * skill_ub + 8 * leader_ub) as u64,
            ScoreTarget::Bonus => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                (((bonus_total.saturating_mul(2)) as u64) << 32) | (live.max(0) as u32 as u64)
            }
            ScoreTarget::Score => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                self.pack_score(live, bonus_total)
            }
            ScoreTarget::Mysekai => mysekai_rank(power_ub, bonus_total),
        }
    }

    /// Score-target ceiling of a Solo, Auto or Challenge deck whose power is
    /// at most `power_ub` and whose six slot score-ups, largest first, are at
    /// most `slots`: every order of the slots over the rates is at most the
    /// sorted pairing.
    #[inline(always)]
    pub(crate) fn score_ceiling_from_slots(
        &self,
        power_ub: u32,
        bonus_total: u32,
        slots: &[u32; DECK_SIZE + 1],
    ) -> u64 {
        debug_assert!(matches!(self.target, ScoreTarget::Score));
        debug_assert!(!matches!(
            self.effective_live_type,
            LiveType::Multi | LiveType::Cheerful | LiveType::Mysekai
        ));
        let power_ub = self.clamp_power_total(power_ub + self.honor_bonus);
        let rate_1m = self.base_rate_1m
            + slots
                .iter()
                .zip(&self.sorted_rates_1m)
                .map(|(&score_up, &rate)| i64::from(score_up) * rate)
                .sum::<i64>();
        let live = (self.live_numerator(power_ub, rate_1m) / LIVE_SCORE_BOUND_SCALE) as i32;
        self.pack_score(live, bonus_total)
    }

    /// Event point in the high 32 bits and live score in the low 32 bits.
    #[inline(always)]
    fn pack_score(&self, live: i32, bonus_total: u32) -> u64 {
        let ep = self.calc_event_point_bound(live, bonus_total);
        ((ep as u64) << 32) | (live as u32 as u64)
    }

    #[inline(always)]
    pub(crate) fn ceiling_of(&self, inputs: CeilingInputs) -> u64 {
        self.ceiling(inputs.power, inputs.bonus, inputs.skill, inputs.leader)
    }

    /// Smallest live score from `start` on whose Score key at `bonus_total`
    /// reaches `threshold`, or `i32::MAX + 1` when none does. Every live
    /// score below `start` is known not to reach it.
    ///
    /// The key is non-decreasing in a non-negative live score: the event
    /// point bound is a non-decreasing base score times non-negative rates
    /// (the Cheerful life rate is positive), floored, and the low 32 bits
    /// hold the live score itself.
    fn needed_live(&self, bonus_total: u32, threshold: u64, start: i64) -> i64 {
        const LIMIT: i64 = i32::MAX as i64;
        if start > LIMIT {
            return start;
        }
        let reaches = |live: i64| self.pack_score(live as i32, bonus_total) >= threshold;
        if reaches(start) {
            return start;
        }
        // Gallop from `start`, then bisect the last step.
        let mut below = start;
        let mut step = 1;
        let mut above = loop {
            let probe = (below + step).min(LIMIT);
            if reaches(probe) {
                break probe;
            }
            if probe == LIMIT {
                return LIMIT + 1;
            }
            below = probe;
            step *= 2;
        };
        while above - below > 1 {
            let middle = below + (above - below) / 2;
            if reaches(middle) {
                above = middle;
            } else {
                below = middle;
            }
        }
        above
    }

    #[inline(always)]
    pub(crate) fn ceiling_multi_score_event(
        &self,
        power_ub: u32,
        bonus_total: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> u64 {
        let power_total = self.clamp_power_total(power_ub + self.honor_bonus);
        let max_slot_5x = (4 * leader_ub as i64 + skill_ub as i64).max(self.teammate_su_5x);
        let rate_1m = self.base_rate_1m + max_slot_5x * self.srs_div500_1m;
        let power_sum = if let Some(teammate_power) = self.multi_teammate_power {
            power_total as i64 + teammate_power as i64 * (DECK_SIZE as i64 - 1)
        } else {
            DECK_SIZE as i64 * power_total as i64
        };
        let active_1m = self.active_1m_coeff * power_sum;
        let live_score = ((rate_1m * power_total as i64 * 4 + active_1m) / 1_000_000) as i32;
        let other_score = if self.other_score == 0 {
            (live_score as i64).saturating_mul(4)
        } else {
            self.other_score as i64
        };
        let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
        let inner = base_score * self.music_rate_pct as i64 * (bonus_total as i64 + 100) / 10_000;
        let event_point = (inner * self.boost_rate_pct as i64 / 100) as i32;
        ((event_point as u64) << 32) | (live_score as u32 as u64)
    }

    #[inline(always)]
    pub(crate) fn calc_live_score_bound_numerator(
        &self,
        power_total: u32,
        skill_total: u32,
        leader_ub: u32,
    ) -> i64 {
        // The evaluator gives a MySekai live no live score.
        if matches!(self.effective_live_type, LiveType::Mysekai) {
            return 0;
        }
        let rate_1m = match self.effective_live_type {
            LiveType::Multi | LiveType::Cheerful => {
                let max_slot_5x =
                    (4 * leader_ub as i64 + skill_total as i64).max(self.teammate_su_5x);
                self.base_rate_1m + max_slot_5x * self.srs_div500_1m
            }
            LiveType::Solo | LiveType::Auto
                if matches!(self.live_skill_order, LiveSkillOrder::Average) =>
            {
                self.base_rate_1m
                    + ceil_div_positive(skill_total as i64 * self.avg_sum5_1m, 500)
                    + ceil_div_positive(leader_ub as i64 * self.avg_leader_rate_1m, 100)
            }
            _ => {
                // The six slots hold the five members and the leader again.
                // Each is at most `leader_ub` and at most `skill_total`, and
                // the five members sum to at most `skill_total`. Every pairing
                // of slots with rates is at most the one that puts the leader
                // slot on the largest rate and spreads the member sum over the
                // others, largest rates first, at most the peak each.
                let peak = i64::from(leader_ub.min(skill_total));
                let mut rest = i64::from(skill_total);
                let mut rate = self.base_rate_1m + peak * self.sorted_rates_1m[0];
                for &coefficient in &self.sorted_rates_1m[1..] {
                    let value = rest.min(peak);
                    rate += value * coefficient;
                    rest -= value;
                }
                rate
            }
        };
        self.live_numerator(power_total, rate_1m)
    }

    #[inline(always)]
    fn live_numerator(&self, power_total: u32, rate_1m: i64) -> i64 {
        let power_sum: i64 = if let Some(tp) = self.multi_teammate_power {
            power_total as i64 + tp as i64 * (DECK_SIZE as i64 - 1)
        } else {
            DECK_SIZE as i64 * power_total as i64
        };
        let active_1m = self.active_1m_coeff * power_sum;
        rate_1m * power_total as i64 * 4 + active_1m
    }

    #[inline(always)]
    pub(crate) fn calc_live_score_bound(
        &self,
        power_total: u32,
        skill_total: u32,
        leader_ub: u32,
    ) -> i32 {
        (self.calc_live_score_bound_numerator(power_total, skill_total, leader_ub)
            / LIVE_SCORE_BOUND_SCALE) as i32
    }

    /// The live-score numerator as a [`LiveProduct`] when the rate is
    /// affine in the skill terms: Multi and Cheerful, and Solo and Auto
    /// under the average skill order.
    pub(crate) fn live_product(&self) -> Option<LiveProduct> {
        let non_negative = self.base_rate_1m >= 0
            && self.srs_div500_1m >= 0
            && self.avg_sum5_1m >= 0
            && self.avg_leader_rate_1m >= 0
            && self.multi_teammate_power.is_none_or(|power| power >= 0);
        if !non_negative {
            return None;
        }
        let honor = i64::from(self.honor_bonus);
        match self.effective_live_type {
            LiveType::Multi | LiveType::Cheerful => {
                // N = 4 * rate * P + active * power_sum, rate = base +
                // max(4L + S, t) * srs, power_sum = 5P or P plus the four
                // teammates' power.
                let active = self.active_1m_coeff;
                let (per_power, constant) = match self.multi_teammate_power {
                    Some(power) => (active, 4 * active * i64::from(power)),
                    None => (DECK_SIZE as i64 * active, 0),
                };
                Some(LiveProduct {
                    honor,
                    intercept: 4 * self.base_rate_1m + per_power,
                    leader: 16 * self.srs_div500_1m,
                    skill: 4 * self.srs_div500_1m,
                    constant,
                    divisor: 1,
                    floor: self.teammate_su_5x,
                })
            }
            LiveType::Solo | LiveType::Auto
                if matches!(self.live_skill_order, LiveSkillOrder::Average) =>
            {
                // N = 4 * rate * P; each rounded-up rate term adds less than
                // one, so 500 * rate <= 500 * base + 1000 + 5 * L * leader
                // rate + S * sum5.
                Some(LiveProduct {
                    honor,
                    intercept: 4 * (500 * self.base_rate_1m + 1000),
                    leader: 20 * self.avg_leader_rate_1m,
                    skill: 4 * self.avg_sum5_1m,
                    constant: 0,
                    divisor: 500,
                    floor: 0,
                })
            }
            _ => None,
        }
    }

    #[inline(always)]
    pub(crate) fn clamp_power_total(&self, power_total: u32) -> u32 {
        self.power_total_cap
            .map_or(power_total, |cap| power_total.min(cap))
    }

    #[inline(always)]
    pub(crate) fn calc_event_point_bound(&self, live_score: i32, total_bonus: u32) -> i32 {
        if !self.has_event {
            return live_score;
        }
        match self.effective_live_type {
            LiveType::Challenge | LiveType::ChallengeAuto => (100 + live_score / 20_000) * 120,
            LiveType::Solo | LiveType::Auto => {
                let base_score = (100 + live_score / 20_000) as i64;
                let inner =
                    base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100) / 10_000;
                (inner * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Multi => {
                let other_score = if self.other_score == 0 {
                    (live_score as i64).saturating_mul(4)
                } else {
                    self.other_score as i64
                };
                let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
                let inner =
                    base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100) / 10_000;
                (inner * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Cheerful => {
                let other_score = if self.other_score == 0 {
                    (live_score as i64).saturating_mul(4)
                } else {
                    self.other_score as i64
                };
                let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
                let inner = (base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100)
                    / 10_000) as i32;
                let with_life = inner as i64 * self.life_rate_num as i64 / 5000;
                (with_life * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Mysekai => 0,
        }
    }

    #[inline(always)]
    pub(crate) const fn score_noevent_threshold_numerator(live: u32) -> i64 {
        live as i64 * LIVE_SCORE_BOUND_SCALE
    }
}

/// Admissible aggregate inputs of a ceiling: power, bonus total, skill sum
/// and leader skill bounds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CeilingInputs {
    pub(crate) power: u32,
    pub(crate) bonus: u32,
    pub(crate) skill: u32,
    pub(crate) leader: u32,
}

/// Bonus totals whose Score cutoff is cached.
const CUTOFF_BONUSES: usize = 2048;

/// Decides whether a ceiling reaches a pruning threshold. A Score key at a
/// fixed bonus total is non-decreasing in the live score, so a ceiling
/// reaches a threshold exactly when its live score reaches the smallest one
/// whose key does. That live score is cached per bonus total and, as the
/// threshold of a search only rises, advanced from its previous value.
pub(crate) struct ScoreCutoff {
    /// `(threshold, needed live score)` per bonus total.
    needed: Vec<(u64, i64)>,
}

impl ScoreCutoff {
    pub(crate) fn new(objective: &ObjectiveBound) -> Self {
        let len = if matches!(objective.target, ScoreTarget::Score) {
            CUTOFF_BONUSES
        } else {
            0
        };
        Self {
            needed: vec![(0, 0); len],
        }
    }

    /// Whether `objective.ceiling_of(inputs) >= threshold`.
    #[inline(always)]
    pub(crate) fn reaches(
        &mut self,
        objective: &ObjectiveBound,
        inputs: CeilingInputs,
        threshold: u64,
    ) -> bool {
        let Some(entry) = self.needed.get_mut(inputs.bonus as usize) else {
            return objective.ceiling_of(inputs) >= threshold;
        };
        if entry.0 != threshold {
            let start = if entry.0 < threshold { entry.1 } else { 0 };
            *entry = (
                threshold,
                objective.needed_live(inputs.bonus, threshold, start),
            );
        }
        let power = objective.clamp_power_total(inputs.power + objective.honor_bonus);
        let live = objective.calc_live_score_bound(power, inputs.skill, inputs.leader);
        if live < 0 {
            return objective.ceiling_of(inputs) >= threshold;
        }
        i64::from(live) >= entry.1
    }
}

/// Ceiling of the rank key `(objective, resolved power)` that orders a
/// MySekai Top-K, whose objective ties break by resolved power: the MySekai
/// value of `power_total` and `total_bonus` in the high 32 bits and
/// `power_total` itself in the low 32 bits, so the numeric order is the
/// lexicographic one.
#[inline(always)]
pub(crate) fn mysekai_rank(power_total: u32, total_bonus: u32) -> u64 {
    (u64::from(calc_mysekai_internal(power_total, f64::from(total_bonus))) << 32)
        | u64::from(power_total)
}

/// Round outward for a positive denominator without an unstable signed API.
#[inline(always)]
pub(crate) fn ceil_div_positive(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(denominator > 0);
    let quotient = numerator / denominator;
    quotient + i64::from(numerator % denominator > 0)
}
