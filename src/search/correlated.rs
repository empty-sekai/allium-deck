//! EXPERIMENT v2: role-aware power/skill envelope with an incumbent-derived slope.
//!
//! Let P be power, S skill sum, L leader skill, and rate <= C+B*S+D*L.
//! For any positive slope lambda, maximize the LINEAR expression
//!   B*R*P + lambda*(B*S+D*L) <= T
//! over character groups, with exactly one card allowed to take the leader role.
//! Member/leader maxima are computed separately PER CHARACTER; assigning a
//! leader replaces that character's member contribution, never counts it twice.
//! The selected prefix may supply the leader, or one remaining character may.
//! Then F <= 4*P*(C+B*S+D*L)/Q is bounded by a concave quadratic:
//!   A=lambda*C+T, H=B*R; UB=ceil(A²/(lambda*H*Q))
//! unless its vertex lies beyond P=T/H, where UB=ceil(4*C*T/(H*Q)).
//! Everything after coefficient preparation uses upward integer arithmetic.
//! No Mask512 changes. Unsupported contexts retain existing bounds. Root gain
//! <3% disables allocation/per-node work. Disable explicitly with env value 0.
use super::{PartialDeck, SearchContext, UsedSet};
use crate::{
    pool::{CardIdx, CardPool},
    types::{LiveSkillOrder, LiveType, ScoreTarget},
};
const Q: u128 = 1_000_000_000_000;
const R: u128 = 256;
const SLOPES: [u32; 7] = [32, 64, 96, 128, 192, 256, 512];
struct Plane {
    lambda: u32,
    member: Vec<[u64; 32]>,
    leader: Vec<[u64; 32]>,
}
pub(super) struct CorrelatedBound {
    base: u128,
    skill: u128,
    leader: u128,
    planes: Vec<Plane>,
}
fn top(values: &[u64; 32], used: u32, k: usize) -> (u128, u64, u32) {
    let mut best = [0u64; 5];
    let mut ids = [32usize; 5];
    if k == 0 {
        return (0, 0, 0);
    }
    for (ch, &v) in values.iter().enumerate() {
        if used & (1u32 << ch) != 0 || v <= best[k - 1] {
            continue;
        }
        let mut j = k - 1;
        while j > 0 && v > best[j - 1] {
            best[j] = best[j - 1];
            ids[j] = ids[j - 1];
            j -= 1;
        }
        best[j] = v;
        ids[j] = ch;
    }
    let mask = ids[..k]
        .iter()
        .filter(|&&ch| ch < 32)
        .fold(0, |m, &ch| m | (1u32 << ch));
    (
        best[..k].iter().map(|&v| v as u128).sum(),
        best[k - 1],
        mask,
    )
}
fn role_envelope(m: &[u64; 32], l: &[u64; 32], used: u32, k: usize, prefix_leader: u128) -> u128 {
    let (sum, cutoff, selected) = top(m, used, k);
    let mut value = sum + prefix_leader;
    if k > 0 {
        for ch in 0..32 {
            if used & (1u32 << ch) != 0 {
                continue;
            }
            let removed = if selected & (1u32 << ch) != 0 {
                m[ch]
            } else {
                cutoff
            };
            value = value.max(sum - removed as u128 + l[ch] as u128);
        }
    }
    value
}
fn quadratic(t: u128, lambda: u32, c: u128, b: u128) -> u64 {
    let h = b * R;
    let lambda = lambda as u128;
    let a = lambda * c + t;
    let (num, den) = if a >= 2 * t {
        (4 * c * t, h * Q)
    } else {
        (a * a, lambda * h * Q)
    };
    // Coefficients are upper-rounded at 1e12. Bounds on base/rates/P/skill below
    // put the arithmetic below u128::MAX. +1 also covers final bounded FP noise.
    num.div_ceil(den).saturating_add(1).min(i32::MAX as u128) as u64
}
#[inline]
fn plane_upper_score(
    plane: &Plane,
    base: u128,
    skill: u128,
    leader: u128,
    start: usize,
    slots: usize,
    used: u32,
    partial: &PartialDeck,
) -> u64 {
    let Some(m) = plane.member.get(start) else {
        return u64::MAX;
    };
    let l = &plane.leader[start];
    let prefix =
        skill * R * partial.power as u128 + plane.lambda as u128 * skill * partial.skill as u128;
    let prefix_leader = plane.lambda as u128 * leader * partial.max_skill as u128;
    let t = prefix + role_envelope(m, l, used, slots, prefix_leader);
    quadratic(t, plane.lambda, base, skill)
}
impl CorrelatedBound {
    pub(super) fn build(
        pool: &CardPool,
        ctx: &SearchContext,
        hint: Option<(u32, u64)>,
        top_k: usize,
        kth_threshold: u64,
    ) -> Option<Self> {
        let tuning = super::tuning::SearchTuning::load();
        if !tuning.bounds
            || !tuning.correlated_bound
            || ctx.target != ScoreTarget::Score
            || ctx.has_event()
            || ctx.is_final_chapter
            || !ctx.enforce_char_uniqueness
            || ctx.honor_bonus != 0
            || ctx.leader_honor_bonus_x10.iter().any(|&x| x != 0)
            || ctx.live_skill_order != LiveSkillOrder::Average
        {
            return None;
        }
        let (base, idx) = match ctx.effective_live_type() {
            LiveType::Solo => (ctx.base_score, 0),
            LiveType::Auto => (ctx.base_score_auto, 2),
            _ => return None,
        };
        let rates = ctx.skill_scores[idx];
        if !base.is_finite()
            || !(0.0..=4.0).contains(&base)
            || rates
                .iter()
                .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return None;
        }
        let base = (base * Q as f64).ceil() as u128 + 1;
        let skill = (rates[..5].iter().sum::<f64>() / 500.0 * Q as f64).ceil() as u128 + 1;
        let leader = (rates[5] / 100.0 * Q as f64).ceil() as u128 + 1;
        let mut slopes: Vec<u32> = SLOPES.iter().map(|&x| x * R as u32).collect();
        if let Some((p, f)) = hint.filter(|&(p, f)| p > 0 && f > 0) {
            let slope =
                (4.0 * skill as f64 * (p as f64).powi(2) / (Q as f64 * f as f64)).clamp(1.0, 512.0);
            for factor in [0.9, 1.0, 1.1] {
                slopes.push(
                    (slope * factor * R as f64)
                        .round()
                        .clamp(1.0, 512.0 * R as f64) as u32,
                );
            }
        }
        slopes.sort_unstable();
        slopes.dedup();
        let mut members = vec![[0u64; 32]; slopes.len()];
        let mut leaders = members.clone();
        let mut pmax = [0u64; 32];
        let mut smax = [0u64; 32];
        for card in pool.indices() {
            let ch = pool.char_id(card) as usize;
            let p = pool.power_max(card);
            let s = pool.skill_max(card) as u128;
            if ch >= 32 || p > 262_143 {
                return None;
            }
            pmax[ch] = pmax[ch].max(p as u64);
            smax[ch] = smax[ch].max(s as u64);
            for (i, &lambda) in slopes.iter().enumerate() {
                let member = skill * R * p as u128 + lambda as u128 * skill * s;
                let lead = member + lambda as u128 * leader * s;
                members[i][ch] = members[i][ch].max(u64::try_from(member).ok()?);
                leaders[i][ch] = leaders[i][ch].max(u64::try_from(lead).ok()?);
            }
        }
        let independent = (4
            * top(&pmax, 0, 5).0
            * (base
                + skill * top(&smax, 0, 5).0
                + leader * (*smax.iter().max().unwrap_or(&0)) as u128))
            .div_ceil(Q)
            + 1;
        let mut choices: Vec<_> = slopes
            .iter()
            .enumerate()
            .map(|(i, &lambda)| {
                (
                    quadratic(
                        role_envelope(&members[i], &leaders[i], 0, 5, 0),
                        lambda,
                        base,
                        skill,
                    ),
                    i,
                )
            })
            .collect();
        choices.sort_unstable();
        if choices[0].0 as u128 * 100 >= independent * 97 {
            return None;
        }
        let auto_planes = tuning.correlated_planes.is_none();
        let plane_count = tuning
            .correlated_planes
            .unwrap_or(if top_k > 1 { 2 } else { 1 });
        let n = pool.count();
        let mut planes = Vec::new();
        // Multiple individually-admissible planes stay admissible under min().
        for &(_, i) in choices.iter().take(plane_count) {
            let lambda = slopes[i];
            let mut member = vec![[0u64; 32]; n + 1];
            let mut lead = member.clone();
            for dense in (0..n).rev() {
                member[dense] = member[dense + 1];
                lead[dense] = lead[dense + 1];
                let card = CardIdx::new(dense as u16);
                let ch = pool.char_id(card) as usize;
                let s = pool.skill_max(card) as u128;
                let m = skill * R * pool.power_max(card) as u128 + lambda as u128 * skill * s;
                let l = m + lambda as u128 * leader * s;
                member[dense][ch] = member[dense][ch].max(u64::try_from(m).ok()?);
                lead[dense][ch] = lead[dense][ch].max(u64::try_from(l).ok()?);
            }
            planes.push(Plane {
                lambda,
                member,
                leader: lead,
            });
        }
        if auto_planes && planes.len() > 1 {
            let mut improved = 0usize;
            let mut material = 0usize;
            let mut gain_1pct = 0usize;
            let mut max_delta = 0u64;
            let mut max_gain_ppm = 0u64;
            let mut direct_prunes = 0usize;
            let mut gap_quarter = 0usize;
            let mut gap_half = 0usize;
            let mut gap_sum = 0u128;
            let mut gap_saved = 0u128;
            for card in pool.indices() {
                let ch = pool.char_id(card);
                let mut used = UsedSet::new();
                used.insert(ch);
                let partial = PartialDeck {
                    power: pool.power_max(card),
                    skill: pool.skill_max(card) as u32,
                    bonus: 0,
                    max_skill: pool.skill_max(card),
                    limited_count: 0,
                };
                let start = card.raw() + 1;
                let first = plane_upper_score(
                    &planes[0],
                    base,
                    skill,
                    leader,
                    start,
                    4,
                    used.bits(),
                    &partial,
                );
                let second = plane_upper_score(
                    &planes[1],
                    base,
                    skill,
                    leader,
                    start,
                    4,
                    used.bits(),
                    &partial,
                );
                if second < first {
                    improved += 1;
                    let delta = first - second;
                    max_delta = max_delta.max(delta);
                    let gain_ppm = delta.saturating_mul(1_000_000) / first.max(1);
                    max_gain_ppm = max_gain_ppm.max(gain_ppm);
                    if delta.saturating_mul(1000) >= first {
                        material += 1;
                    }
                    if gain_ppm >= 10_000 {
                        gain_1pct += 1;
                    }
                    if kth_threshold > 0 && first > kth_threshold {
                        let gap = first - kth_threshold;
                        let saved = delta.min(gap);
                        gap_sum += gap as u128;
                        gap_saved += saved as u128;
                        if second <= kth_threshold {
                            direct_prunes += 1;
                        }
                        if saved.saturating_mul(4) >= gap {
                            gap_quarter += 1;
                        }
                        if saved.saturating_mul(2) >= gap {
                            gap_half += 1;
                        }
                    }
                }
            }
            // Keep the second admissible plane only when a cheap depth-1 probe predicts
            // material search-tree reduction.  A direct crossing of the current kth
            // incumbent is decisive; otherwise require at least 6% relative tightening.
            // The 6% selector separated all >=20% node-reduction cases in the AS
            // synthetic selector sweep (70 fixtures) without false positives.
            let keep_second = direct_prunes > 0 || max_gain_ppm >= 60_000;
            if tuning.correlated_trace {
                eprintln!(
                    "correlated-auto top_k={top_k} kth={kth_threshold} improved={improved} material={material} gain_1pct={gain_1pct} max_delta={max_delta} max_gain_ppm={max_gain_ppm} direct_prunes={direct_prunes} gap_quarter={gap_quarter} gap_half={gap_half} gap_saved={gap_saved} gap_sum={gap_sum} keep_second={keep_second}"
                );
            }
            if !keep_second {
                planes.truncate(1);
            }
        }
        Some(Self {
            base,
            skill,
            leader,
            planes,
        })
    }
    #[inline]
    pub(super) fn upper_bound(
        &self,
        start: usize,
        slots: usize,
        used: &UsedSet,
        partial: &PartialDeck,
    ) -> u64 {
        let mut upper = u64::MAX;
        for plane in &self.planes {
            upper = upper.min(plane_upper_score(
                plane,
                self.base,
                self.skill,
                self.leader,
                start,
                slots,
                used.bits(),
                partial,
            ));
        }
        (upper << 32) | upper
    }
}
