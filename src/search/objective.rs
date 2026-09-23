//! Admissible relaxation of the scoring objective over aggregate deck features.
//!
//! Every search ceiling eventually asks one question: given upper bounds on a
//! deck's total power, total bonus, total skill and leader skill, what is the
//! largest encoded objective any deck with those features can reach? The
//! answer depends only on the search context (target, live type, music and
//! event constants), never on the card pool, so one [`ObjectiveBound`] serves
//! every pool restriction of the same request. All integer steps round
//! outward, which keeps each ceiling admissible for the exact evaluator.

use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType, ScoreTarget};

use super::context::SearchContext;
use super::evaluate::calc_mysekai_internal;

/// Fixed-point scale of the live-score numerators.
pub(crate) const LIVE_SCORE_BOUND_SCALE: i64 = 1_000_000;

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

    /// Generic target-aware ceiling from admissible aggregate inputs.
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
                let ep = self.calc_event_point_bound(live, bonus_total);
                ((ep as u64) << 32) | (live as u32 as u64)
            }
            ScoreTarget::Mysekai => calc_mysekai_internal(power_ub, bonus_total as f64) as u64,
        }
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
                // 每个技能槽的 score_up 不超过全队最大技能 L（含 leader 复发槽），
                // 因此 Σ su_i·r_i ≤ L·Σr_i = L·srs。旧值 5*skill_total(=S·srs/100)
                // 对 Solo/Auto 高估约 5 倍。
                self.base_rate_1m + 5 * (leader_ub as i64) * self.srs_div500_1m
            }
        };
        let power_sum: i64 = if let Some(tp) = self.multi_teammate_power {
            power_total as i64 + tp as i64 * (DECK_SIZE as i64 - 1)
        } else {
            DECK_SIZE as i64 * power_total as i64
        };
        let active_1m = self.active_1m_coeff * power_sum;
        match self.effective_live_type {
            LiveType::Mysekai => 0,
            _ => rate_1m * power_total as i64 * 4 + active_1m,
        }
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

    #[inline(always)]
    pub(crate) fn clamp_power_total(&self, power_total: u32) -> u32 {
        self.power_total_cap
            .map_or(power_total, |cap| power_total.min(cap))
    }

    #[inline(always)]
    pub(crate) fn joint_event_point_upper(
        &self,
        power_ub: u32,
        bonus_ub: u32,
        skill_ub: u32,
        leader_ub: u32,
        support_ub: u32,
        bonus_weight: u32,
    ) -> u32 {
        let max_slot_5x = (4 * leader_ub as i64 + skill_ub as i64).max(self.teammate_su_5x);
        let rate_1m = self.base_rate_1m + max_slot_5x * self.srs_div500_1m;
        let (power_multiplier, power_constant) = match self.multi_teammate_power {
            Some(teammate_power) => (1i128, teammate_power as i128 * (DECK_SIZE as i128 - 1)),
            None => (DECK_SIZE as i128, 0),
        };
        let live_power_coeff =
            4i128 * rate_1m as i128 + self.active_1m_coeff as i128 * power_multiplier;
        let live_constant = self.active_1m_coeff as i128 * power_constant;

        let capped_power = self
            .power_total_cap
            .map_or(power_ub, |cap| power_ub.min(cap));
        let support_ub = self.power_total_cap.map_or(support_ub, |cap| {
            support_ub.min(cap.saturating_add(bonus_weight.saturating_mul(bonus_ub)))
        });

        let capped_other = if self.other_score == 0 {
            13i128
        } else {
            (self.other_score as i128 / 340_000).min(13)
        };
        let capped_bound = maximize_joint_event_numerator(
            capped_power,
            bonus_ub,
            support_ub,
            bonus_weight,
            123,
            17_000_000_000,
            live_power_coeff,
            live_constant,
            1,
        );
        let capped_ep = ceil_div_i128(
            capped_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
            17_000_000_000i128 * 1_000_000,
        );

        let selected_ep = if self.other_score == 0 {
            let uncapped_bound = maximize_joint_event_numerator(
                capped_power,
                bonus_ub,
                support_ub,
                bonus_weight,
                110,
                85_000_000_000,
                live_power_coeff,
                live_constant,
                6,
            );
            let uncapped_ep = ceil_div_i128(
                uncapped_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
                85_000_000_000i128 * 1_000_000,
            );
            capped_ep.min(uncapped_ep)
        } else {
            let fixed_other_bound = maximize_joint_event_numerator(
                capped_power,
                bonus_ub,
                support_ub,
                bonus_weight,
                110 + capped_other as i64,
                17_000_000_000,
                live_power_coeff,
                live_constant,
                1,
            );
            ceil_div_i128(
                fixed_other_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
                17_000_000_000i128 * 1_000_000,
            )
        };
        selected_ep.clamp(0, u32::MAX as i128) as u32
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

/// Round outward for a positive denominator without an unstable signed API.
#[inline(always)]
pub(crate) fn ceil_div_positive(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(denominator > 0);
    let quotient = numerator / denominator;
    quotient + i64::from(numerator % denominator > 0)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn maximize_joint_event_numerator(
    power_ub: u32,
    bonus_ub: u32,
    support_ub: u32,
    bonus_weight: u32,
    base_constant: i64,
    base_denominator: i128,
    live_power_coeff: i128,
    live_constant: i128,
    live_multiplier: i128,
) -> i128 {
    let max_bonus = bonus_ub.min(support_ub / bonus_weight);
    let linear_power = live_multiplier * live_power_coeff;
    let linear_constant =
        base_constant as i128 * base_denominator + live_multiplier * live_constant;

    let evaluate = |bonus: u32| -> i128 {
        let supported_power = support_ub.saturating_sub(bonus_weight.saturating_mul(bonus));
        let power = power_ub.min(supported_power) as i128;
        (linear_constant + linear_power * power) * (bonus as i128 + 100)
    };

    let mut best = evaluate(0).max(evaluate(max_bonus));
    if support_ub > power_ub {
        let flat_end = ((support_ub - power_ub) / bonus_weight).min(max_bonus);
        best = best.max(evaluate(flat_end));
        if flat_end < max_bonus {
            best = best.max(evaluate(flat_end + 1));
        }
    }

    let quadratic = linear_power * bonus_weight as i128;
    if quadratic > 0 {
        let vertex_numerator =
            linear_constant + linear_power * support_ub as i128 - quadratic * 100;
        if vertex_numerator > 0 {
            let vertex = vertex_numerator / (2 * quadratic);
            for candidate in [vertex - 1, vertex, vertex + 1] {
                if candidate >= 0 && candidate <= max_bonus as i128 {
                    best = best.max(evaluate(candidate as u32));
                }
            }
        }
    }
    best
}

#[inline(always)]
fn ceil_div_i128(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(numerator >= 0 && denominator > 0);
    numerator.saturating_add(denominator - 1) / denominator
}
