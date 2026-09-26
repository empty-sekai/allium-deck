//! Log-linear relaxation of the event-point ceiling.
//!
//! [`ObjectiveBound::ceiling`] relaxes every aggregate feature on its own, so
//! a subtree whose best power, best skill and best bonus come from different
//! cards receives the ceiling of a deck that does not exist. For the Score
//! target of an event, the event point of a deck with power `P`, skill `S`,
//! leader skill `L` and bonus `B` is at most a product
//!
//! ```text
//! kappa * (c + P * Q(S, L) / D) * (100 + B)
//! ```
//!
//! whose logarithm, for every deck that can reach a given threshold, is at
//! most an affine function of `(P, S, L, B)`. Each card then contributes one
//! weight, and a sum of per-card or per-group maxima bounds the logarithm of
//! the event point of every deck of a subtree. The derivation is "Log-linear
//! event-point bound" in `docs/pruning-proof.md`.

mod interval;
use interval::Interval;
use interval::div_up;
pub(crate) use interval::{add_up, mul_up, sub_down, sum_up};

use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType, ScoreTarget};

use super::objective::{LIVE_SCORE_BOUND_SCALE, ObjectiveBound, RATE_FRACTION_SCALE};

/// Largest aggregate features of every deck a bound covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FeatureBox {
    /// Deck power, the honor bonus excluded.
    pub(crate) power: u32,
    /// Deck skill, the leader included.
    pub(crate) skill: u32,
    /// Largest skill of a single card.
    pub(crate) card_skill: u32,
    /// Smallest leader skill.
    pub(crate) leader_skill_min: u32,
    /// Largest leader skill.
    pub(crate) leader_skill_max: u32,
    /// Deck bonus.
    pub(crate) bonus: u32,
}

/// Affine bound on the logarithm of the event-point ceiling: every deck in
/// the [`FeatureBox`] whose ceiling reaches `floor_ep` satisfies
/// `ln(ep) <= constant + power * P + skill * S + leader_skill * L + bonus * B`,
/// where `P` excludes the honor bonus and `L` is the leader's own skill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LogLinearBound {
    pub(crate) power: f64,
    pub(crate) skill: f64,
    pub(crate) leader_skill: f64,
    pub(crate) bonus: f64,
    pub(crate) constant: f64,
    floor_ep: u64,
}

impl LogLinearBound {
    /// The bound for decks inside `feature_box` whose event point can reach
    /// `threshold_ep`, or `None` where the relaxation does not apply.
    pub(crate) fn new(
        objective: &ObjectiveBound,
        feature_box: &FeatureBox,
        threshold_ep: u64,
    ) -> Option<Self> {
        let model = EventModel::new(objective, feature_box)?;
        if threshold_ep == 0
            || threshold_ep > u64::from(u32::MAX)
            || feature_box.power == 0
            || feature_box.leader_skill_min > feature_box.leader_skill_max
        {
            return None;
        }
        let point = Interval::point;
        let honor = f64::from(objective.honor_bonus);
        let power_max = f64::from(feature_box.power) + honor;
        let skill_max = f64::from(feature_box.skill);
        let leader_max = f64::from(feature_box.leader_skill_max);
        let bonus_max = f64::from(feature_box.bonus);
        let tau = threshold_ep as f64;
        let scale = point(LIVE_SCORE_BOUND_SCALE as f64);
        let divisor = point(model.divisor);
        let offset = point(model.offset);
        let kappa = point(model.kappa);

        // Enclose the model's exact real endpoints, then WIDEN the chord
        // interval. Inward-rounded endpoints could exclude a feasible deck.
        let rate_max = model.rate_interval(skill_max, leader_max);
        let u_hi =
            point(power_max) * (point(4.0) * rate_max + point(model.active)) / scale / divisor;
        let u_lo = point(tau) / (kappa * point(100.0 + bonus_max)) - offset;
        if !u_lo.finite() || !u_hi.finite() || !(u_lo.lo > 0.0 && u_lo.lo < u_hi.hi) {
            return None;
        }
        let lo = point(u_lo.lo);
        let sigma = chord_slope(model.offset, u_lo.lo, u_hi.hi)?;

        // This point is only a tightness heuristic. The certificate below
        // holds for ANY positive finite point, including rounded bisection.
        let relaxed = |lambda: f64| {
            let rate = model.per_power(model.rate_1m(lambda * skill_max, leader_max));
            let u = lambda * power_max * rate / model.divisor;
            model.kappa * (model.offset + u) * (100.0 + lambda * bonus_max)
        };
        let (mut low, mut high) = (0.0f64, 1.0f64);
        for _ in 0..48 {
            let mid = 0.5 * (low + high);
            if relaxed(mid) < tau {
                low = mid;
            } else {
                high = mid;
            }
        }
        if high < 1.0 / 1024.0 {
            return None;
        }
        let power_0 = point(high * power_max);
        let skill_0 = high * skill_max;
        let bonus_0 = point(high * bonus_max);
        let tangent = model.tangent(skill_0);
        // These scalar coefficients themselves bound q; treat their stored
        // f64 values as exact constants of the affine majorant.
        let intercept = point(model.per_power(tangent.intercept));
        let skill = point(model.slope(tangent.skill));
        let leader_skill = point(model.slope(tangent.leader_skill));
        let rate_0 = intercept + skill * point(skill_0) + leader_skill * point(leader_max);
        if !power_0.finite() || !rate_0.finite() || power_0.lo <= 0.0 || rate_0.lo <= 0.0 {
            return None;
        }
        let one = point(1.0);
        let hundred = point(100.0);
        let bonus_denominator = hundred + bonus_0;
        let constant = kappa.ln() + (offset + lo).ln() - sigma * lo.ln()
            + sigma
                * (power_0.ln() - one + point(honor) / power_0 + rate_0.ln() - one
                    + intercept / rate_0
                    - divisor.ln())
            + bonus_denominator.ln()
            - one
            + hundred / bonus_denominator;
        let coefficients = [
            sigma / power_0,
            sigma * skill / rate_0,
            sigma * leader_skill / rate_0,
            one / bonus_denominator,
        ];
        if !constant.finite()
            || constant.hi.abs() >= 256.0
            || coefficients
                .iter()
                .any(|value| !value.finite() || value.hi < 0.0)
        {
            return None;
        }
        // Every feature is non-negative, so choosing each upper endpoint
        // yields a pointwise majorant of the real affine certificate.
        Some(Self {
            power: coefficients[0].hi,
            skill: coefficients[1].hi,
            leader_skill: coefficients[2].hi,
            bonus: coefficients[3].hi,
            constant: constant.hi,
            floor_ep: threshold_ep,
        })
    }

    /// Weight of one card's power, skill and bonus.
    #[inline(always)]
    pub(crate) fn weigh(&self, power: u32, skill: u32, bonus: u32) -> f64 {
        sum_up([
            mul_up(self.power, f64::from(power)),
            mul_up(self.skill, f64::from(skill)),
            mul_up(self.bonus, f64::from(bonus)),
        ])
    }

    /// The weight below which [`Self::excludes`] rules a subtree out at
    /// `threshold_ep`; a caller that tests many subtrees against one
    /// threshold computes it once.
    #[inline(always)]
    pub(crate) fn log_cutoff(threshold_ep: u64) -> f64 {
        if threshold_ep == 0 || threshold_ep > u64::from(u32::MAX) {
            return f64::NEG_INFINITY;
        }
        Interval::point(threshold_ep as f64).ln().lo
    }

    /// Whether a subtree whose features weigh at most `value` has no deck
    /// with an event point of `threshold_ep` or more, given the
    /// [`Self::log_cutoff`] of `threshold_ep`.
    #[inline(always)]
    pub(crate) fn excludes(&self, value: f64, threshold_ep: u64, cutoff: f64) -> bool {
        debug_assert!(threshold_ep >= self.floor_ep);
        debug_assert_eq!(cutoff.to_bits(), Self::log_cutoff(threshold_ep).to_bits());
        value.is_finite() && cutoff.is_finite() && value < cutoff
    }
}

/// The chord slope is certified, not assumed accurate because ln is accurate.
/// A denominator interval reaching zero makes the optional bound unavailable.
fn chord_slope(offset: f64, lo: f64, hi: f64) -> Option<Interval> {
    let c = Interval::point(offset);
    let a = Interval::point(lo);
    let b = Interval::point(hi);
    let denominator = b.ln() - a.ln();
    if !denominator.finite() || denominator.lo <= 0.0 {
        return None;
    }
    let sigma = ((c + b).ln() - (c + a).ln()) / denominator;
    (sigma.finite() && sigma.lo > 0.0 && sigma.hi < 1.0).then_some(sigma)
}

/// Upper conversion of a non-negative fixed-point integer.
fn integer_up(value: i64) -> f64 {
    debug_assert!(value >= 0);
    let rounded = value as f64;
    if value <= (1_i64 << 53) {
        rounded
    } else {
        rounded.next_up()
    }
}

/// The event-point ceiling as
/// `kappa * (offset + P * per_power(rate_1m(S, L)) / divisor) * (100 + B)`.
struct EventModel {
    kappa: f64,
    offset: f64,
    divisor: f64,
    rate: RateModel,
    /// Active-score coefficient per unit power, on the live numerator scale.
    active: f64,
}

/// Upper bound on the fixed-point live rate `rate_1m` of
/// [`ObjectiveBound::calc_live_score_bound_numerator`].
enum RateModel {
    Affine(Tangent),
    /// `intercept + sum_j rates[j] * clamp(S - j * cap, 0, cap)`, concave in `S`.
    Sorted {
        intercept: f64,
        cap: f64,
        rates: [f64; DECK_SIZE],
    },
}

/// `intercept + skill * S + leader_skill * L`.
#[derive(Clone, Copy)]
struct Tangent {
    intercept: f64,
    skill: f64,
    leader_skill: f64,
}

impl EventModel {
    fn new(objective: &ObjectiveBound, feature_box: &FeatureBox) -> Option<Self> {
        if !matches!(objective.target, ScoreTarget::Score) || !objective.has_event {
            return None;
        }
        let (offset, divisor) = match objective.effective_live_type {
            LiveType::Solo | LiveType::Auto => (100.0, 20_000.0),
            LiveType::Multi => {
                let other = if objective.other_score == 0 {
                    13
                } else {
                    (i64::from(objective.other_score) / 340_000).min(13)
                };
                ((110 + other) as f64, 17_000.0)
            }
            _ => return None,
        };
        let scale = LIVE_SCORE_BOUND_SCALE as f64;
        let kappa = div_up(
            mul_up(
                f64::from(objective.music_rate_pct),
                f64::from(objective.boost_rate_pct),
            ),
            scale,
        );
        let non_negative = objective.base_rate_1m >= 0
            && objective.srs_div500_q >= 0
            && objective.avg_sum5_1m >= 0
            && objective.avg_leader_rate_1m >= 0
            && objective.sorted_rates_q.iter().all(|&rate| rate >= 0)
            && objective
                .multi_teammate_power
                .is_none_or(|power| power >= 0);
        if kappa <= 0.0 || !non_negative {
            return None;
        }
        // The live numerator is 4 * rate_1m * P + active_1m, with active_1m =
        // coefficient * (P + 4 * teammate) or coefficient * 5 * P.
        let coefficient = integer_up(objective.active_1m_coeff);
        let (active, fixed) = match objective.multi_teammate_power {
            Some(power) => (
                coefficient,
                mul_up(mul_up(coefficient, 4.0), f64::from(power)),
            ),
            None => (mul_up(coefficient, DECK_SIZE as f64), 0.0),
        };
        let base = integer_up(objective.base_rate_1m);
        let rate = match objective.effective_live_type {
            LiveType::Multi => {
                // max(4L + S, t) <= 4L + S + max(0, t - 4 L_min).
                let excess =
                    (objective.teammate_su_5x - 4 * i64::from(feature_box.leader_skill_min)).max(0);
                let excess = integer_up(excess);
                // The skill term rounds up by less than one.
                let srs = div_up(integer_up(objective.srs_div500_q), RATE_FRACTION_SCALE);
                RateModel::Affine(Tangent {
                    intercept: add_up(add_up(base, 1.0), mul_up(excess, srs)),
                    skill: srs,
                    leader_skill: mul_up(4.0, srs),
                })
            }
            _ if matches!(objective.live_skill_order, LiveSkillOrder::Average) => {
                // Each ceil_div_positive adds less than one.
                RateModel::Affine(Tangent {
                    intercept: add_up(base, 2.0),
                    skill: div_up(integer_up(objective.avg_sum5_1m), 500.0),
                    leader_skill: div_up(integer_up(objective.avg_leader_rate_1m), 100.0),
                })
            }
            _ => {
                // The peak slot is at most the largest card skill, and the
                // rate is non-decreasing in the peak.
                // The skill terms round up by less than one together.
                let cap = f64::from(feature_box.card_skill);
                let sorted = objective
                    .sorted_rates_q
                    .map(|rate| div_up(integer_up(rate), RATE_FRACTION_SCALE));
                RateModel::Sorted {
                    intercept: add_up(add_up(base, 1.0), mul_up(cap, sorted[0])),
                    cap,
                    rates: core::array::from_fn(|slot| sorted[slot + 1]),
                }
            }
        };
        Some(Self {
            kappa,
            offset: add_up(offset, div_up(div_up(fixed, scale), divisor)),
            divisor,
            rate,
            active,
        })
    }

    fn rate_1m(&self, skill: f64, leader_skill: f64) -> f64 {
        match &self.rate {
            RateModel::Affine(tangent) => {
                tangent.intercept + tangent.skill * skill + tangent.leader_skill * leader_skill
            }
            RateModel::Sorted {
                intercept,
                cap,
                rates,
            } => {
                let mut rate = *intercept;
                for (slot, &coefficient) in rates.iter().enumerate() {
                    rate += coefficient * (skill - slot as f64 * cap).clamp(0.0, *cap);
                }
                rate
            }
        }
    }

    /// Exact-real rate of the stored, upward-prepared model, enclosed.
    fn rate_interval(&self, skill: f64, leader: f64) -> Interval {
        let p = Interval::point;
        match &self.rate {
            RateModel::Affine(t) => {
                p(t.intercept) + p(t.skill) * p(skill) + p(t.leader_skill) * p(leader)
            }
            RateModel::Sorted {
                intercept,
                cap,
                rates,
            } => {
                let mut value = p(*intercept);
                for (slot, &coefficient) in rates.iter().enumerate() {
                    // Callers pass integer skill sums; these differences and
                    // clamps are exact and within the u32 range.
                    value =
                        value + p(coefficient) * p((skill - slot as f64 * cap).clamp(0.0, *cap));
                }
                value
            }
        }
    }

    /// Live score per unit power at fixed-point rate `rate_1m`.
    fn per_power(&self, rate_1m: f64) -> f64 {
        div_up(
            add_up(mul_up(4.0, rate_1m), self.active),
            LIVE_SCORE_BOUND_SCALE as f64,
        )
    }

    /// Live score per unit power of one unit of a fixed-point rate slope.
    fn slope(&self, rate_1m: f64) -> f64 {
        div_up(mul_up(4.0, rate_1m), LIVE_SCORE_BOUND_SCALE as f64)
    }

    /// An affine function of `(S, L)` that bounds [`Self::rate_1m`] for every
    /// `S >= 0` and meets it at `skill_0`.
    fn tangent(&self, skill_0: f64) -> Tangent {
        match &self.rate {
            RateModel::Affine(tangent) => *tangent,
            RateModel::Sorted {
                intercept,
                cap,
                rates,
            } => {
                // Compare exact integer breakpoints instead of flooring a
                // rounded quotient that could select the wrong segment.
                let segment = (0..DECK_SIZE)
                    .find(|&slot| skill_0 < (slot + 1) as f64 * cap)
                    .unwrap_or(DECK_SIZE);
                let slope = rates.get(segment).copied().unwrap_or(0.0);
                let p = Interval::point;
                let mut constant = p(*intercept);
                for &rate in &rates[..segment] {
                    constant = constant + p(*cap) * (p(rate) - p(slope));
                }
                Tangent {
                    intercept: constant.hi,
                    skill: slope,
                    leader_skill: 0.0,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::context::{SearchContext, SupportDeck};
    use crate::types::{EventType, SkillReferenceStrategy};

    fn context(live_type: LiveType, order: LiveSkillOrder) -> SearchContext {
        SearchContext {
            target: ScoreTarget::Score,
            fixed_card_ids: Vec::new(),
            fixed_character_ids: Vec::new(),
            forced_leader_character_id: None,
            music_rate_pct: 120,
            boost_rate_pct: 100,
            base_score: 1.1,
            base_score_auto: 0.9,
            fever_score: 0.3,
            skill_scores: [
                [1.2, 0.8, 1.0, 1.4, 0.6, 1.3],
                [0.9, 1.0, 1.1, 1.2, 0.7, 1.5],
                [1.0, 1.0, 0.8, 1.1, 0.9, 1.2],
            ],
            other_score: 0,
            life: 1_000,
            diff_attr_bonus: [0; 6],
            support_deck: SupportDeck::default(),
            support_decks_by_character: vec![SupportDeck::default(); 27],
            is_world_bloom: true,
            is_final_chapter: true,
            enforce_char_uniqueness: true,
            minimize: false,
            live_type,
            event_type: Some(EventType::WorldBloom),
            skill_reference_strategy: SkillReferenceStrategy::Average,
            best_skill_as_leader: false,
            live_skill_order: order,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            multi_live_score_up_lower_bound: None,
            extra_bonus_ub: 0,
            w_power: 2.0,
            w_bonus: 1.0,
            skill_ub_global: 0,
            card_bonus_count_limit: 5,
            honor_bonus: 1_200,
            power_total_cap: None,
            leader_honor_bonus_x10: Vec::new(),
            leader_honors: Vec::new(),
            leader_limit_bonus_x10: Vec::new(),
            final_chapter_member_keep: Vec::new(),
        }
    }

    #[test]
    fn affine_weights_bound_the_event_point_ceiling() {
        let mut state = 0x9e37_79b9_u32;
        let mut next = move |bound: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % bound.max(1)
        };
        let feature_box = FeatureBox {
            power: 300_000,
            skill: 700,
            card_skill: 150,
            leader_skill_min: 60,
            leader_skill_max: 150,
            bonus: 900,
        };
        let mut checked = 0usize;
        for live_type in [LiveType::Multi, LiveType::Solo, LiveType::Auto] {
            for order in [
                LiveSkillOrder::Average,
                LiveSkillOrder::Best,
                LiveSkillOrder::Worst,
            ] {
                for (other_score, teammate) in [(0, None), (2_000_000, Some(250_000))] {
                    let mut ctx = context(live_type, order);
                    ctx.other_score = other_score;
                    ctx.multi_teammate_power = teammate;
                    ctx.multi_teammate_score_up = teammate.map(|_| 180);
                    let objective = ObjectiveBound::from_context(&ctx);
                    let top = objective.ceiling(feature_box.power, feature_box.bonus, 700, 150);
                    for step in 1..=8u64 {
                        let threshold = ((top >> 32) * (8 + step) / 17).max(1);
                        let bound = LogLinearBound::new(&objective, &feature_box, threshold)
                            .expect("the relaxation applies to event Score lives");
                        for _ in 0..2_000 {
                            let leader = feature_box.leader_skill_min
                                + next(
                                    feature_box.leader_skill_max - feature_box.leader_skill_min + 1,
                                );
                            // The upper half of every range reaches the thresholds.
                            let upper = |max: u32, draw: u32| max - draw % (max / 2 + 1);
                            let members = upper(
                                (feature_box.skill - leader).min(4 * feature_box.card_skill),
                                next(u32::MAX),
                            );
                            let power = upper(feature_box.power, next(u32::MAX));
                            let bonus = upper(feature_box.bonus, next(u32::MAX));
                            let skill = leader + members;
                            let peak = leader.max(members.min(feature_box.card_skill));
                            let leader_input = if matches!(live_type, LiveType::Multi)
                                || order == LiveSkillOrder::Average
                            {
                                leader
                            } else {
                                peak
                            };
                            let ep = objective.ceiling(power, bonus, skill, leader_input) >> 32;
                            if ep < threshold {
                                continue;
                            }
                            checked += 1;
                            let value = bound.constant
                                + bound.power * f64::from(power)
                                + bound.skill * f64::from(skill)
                                + bound.leader_skill * f64::from(leader)
                                + bound.bonus * f64::from(bonus);
                            assert!(
                                !bound.excludes(value, ep, LogLinearBound::log_cutoff(ep)),
                                "live={live_type:?} order={order:?} power={power} skill={skill} leader={leader} bonus={bonus} ep={ep} value={value}"
                            );
                            assert!(value >= (ep as f64).ln() - 1e-12);
                        }
                    }
                }
            }
        }
        assert!(
            checked > 10_000,
            "only {checked} points reached a threshold"
        );
    }

    #[test]
    fn nearly_coincident_chord_endpoints_fall_back() {
        assert!(chord_slope(100.0, 100.0, 100.0_f64.next_up()).is_none());
        let slope = chord_slope(100.0, 100.0, 200.0).unwrap();
        let reference = 1.5_f64.ln() / 2.0_f64.ln();
        assert!(slope.lo <= reference && slope.hi >= reference);
    }

    #[test]
    fn unsupported_objectives_have_no_bound() {
        let feature_box = FeatureBox {
            power: 300_000,
            skill: 700,
            card_skill: 150,
            leader_skill_min: 60,
            leader_skill_max: 150,
            bonus: 900,
        };
        let mut ctx = context(LiveType::Multi, LiveSkillOrder::Best);
        ctx.target = ScoreTarget::Power;
        assert!(
            LogLinearBound::new(&ObjectiveBound::from_context(&ctx), &feature_box, 100).is_none()
        );
        let mut ctx = context(LiveType::Multi, LiveSkillOrder::Best);
        ctx.event_type = Some(EventType::CheerfulCarnival);
        assert!(
            LogLinearBound::new(&ObjectiveBound::from_context(&ctx), &feature_box, 100).is_none()
        );
        let ctx = context(LiveType::Multi, LiveSkillOrder::Best);
        assert!(
            LogLinearBound::new(&ObjectiveBound::from_context(&ctx), &feature_box, 0).is_none()
        );
    }
}
