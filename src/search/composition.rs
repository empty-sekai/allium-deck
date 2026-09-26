//! Area-item composition regimes.
//!
//! A card's resolved power depends on two deck-wide facts: whether all five
//! cards contain some unit, and whether all five share one attribute. The pool
//! stores, per card and per unit profile, the four resolved powers for the
//! member keys {neither, shared attribute, shared unit, both}. Their per-card
//! maximum assumes both deck-wide bonuses at once, which overstates the power
//! of every deck that shares neither.
//!
//! Instead the feasible decks are covered by regimes. For a deck `D` let
//! `A(D)` be the attribute shared by all five cards (if any) and `U(D)` the set
//! of units contained in all five cards. Then `D` belongs to
//!
//! | regime | condition | admitted cards | member keys of the power bound |
//! | --- | --- | --- | --- |
//! | `Mixed` | `A(D)` none, `U(D)` empty | all | neither |
//! | `SharedAttr(a)` | `A(D) = a`, `U(D)` empty | attribute `a` | shared attribute |
//! | `SharedUnit(u)` | `A(D)` none, `u ∈ U(D)` | containing `u` | neither, shared unit |
//! | `SharedUnitAttr(u, a)` | `A(D) = a`, `u ∈ U(D)` | attribute `a`, containing `u` | shared attribute, both |
//!
//! Every deck satisfies at least one row. In a deck of a regime, each card
//! resolves its power as the maximum over its units `w` of the member key
//! `(w ∈ U(D), A(D) present)`; the listed keys contain every key that pair can
//! take there, so the per-card bound dominates the resolved power of every
//! deck of the regime, and the regime's search on the admitted cards is
//! complete for that regime. Decks of other regimes may also be visited and are
//! then evaluated exactly; they are never required to be found there.
//!
//! A same-character search, which takes five distinct public cards instead
//! of five characters, uses the same regimes; its plan ceiling sums the five
//! largest card values rather than the five largest character values.
//!
//! All regimes feed one canonical tracker in original pool indices. Incumbent
//! seeds are generated once on the whole pool and enter that tracker first. A
//! regime is searched with the K-th objective already reached as an external
//! floor; a branch strictly below that floor cannot enter the global Top-K
//! because K distinct legal public sets at least that good are known. A
//! regime whose ceiling, or whose log-linear bound over the coupled power,
//! skill and bonus of its cards, is below that cutoff is skipped. Seeds
//! whose cards are all admitted by a regime are also handed to its search as
//! ordering hints; they are never needed for completeness.

use super::budget::SearchBudget;
use super::dfs::canonicalize_seed_result;
use super::log_linear::{FeatureBox, LogLinearBound, mul_up, sum_up};
use super::objective::ObjectiveBound;
use super::tracker::TopKTracker;
use super::{DeckResult, SearchContext, SearchParams, SearchStats, SupportDeck};
use crate::pool::{CardIdx, CardPool};
use crate::search::evaluate::decode_u18;
use crate::types::DECK_SIZE;

const UNIT_COUNT: u8 = 6;
const ATTR_COUNT: u8 = 6;
const CHARACTER_COUNT: usize = 27;

const KEY_NEITHER: u8 = 1 << 0;
const KEY_SHARED_ATTR: u8 = 1 << 1;
const KEY_SHARED_UNIT: u8 = 1 << 2;
const KEY_BOTH: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Regime {
    Mixed,
    SharedAttr(u8),
    SharedUnit(u8),
    SharedUnitAttr(u8, u8),
}

impl Regime {
    pub(super) fn all() -> impl Iterator<Item = Self> {
        let attrs = (0..ATTR_COUNT).map(Self::SharedAttr);
        let units = (0..UNIT_COUNT).map(Self::SharedUnit);
        let both = (0..UNIT_COUNT)
            .flat_map(|unit| (0..ATTR_COUNT).map(move |attr| Self::SharedUnitAttr(unit, attr)));
        std::iter::once(Self::Mixed)
            .chain(attrs)
            .chain(units)
            .chain(both)
    }

    pub(super) fn admits(self, pool: &CardPool, card: CardIdx) -> bool {
        let has_unit = |unit: u8| pool.unit_mask_raw(card) & (1 << unit) != 0;
        match self {
            Self::Mixed => true,
            Self::SharedAttr(attr) => pool.attr(card) == attr,
            Self::SharedUnit(unit) => has_unit(unit),
            Self::SharedUnitAttr(unit, attr) => has_unit(unit) && pool.attr(card) == attr,
        }
    }

    pub(super) fn member_keys(self) -> u8 {
        match self {
            Self::Mixed => KEY_NEITHER,
            Self::SharedAttr(_) => KEY_SHARED_ATTR,
            Self::SharedUnit(_) => KEY_NEITHER | KEY_SHARED_UNIT,
            Self::SharedUnitAttr(..) => KEY_SHARED_ATTR | KEY_BOTH,
        }
    }

    /// Whether every deck of the regime shares one attribute.
    pub(super) fn shares_attr(self) -> bool {
        matches!(self, Self::SharedAttr(_) | Self::SharedUnitAttr(..))
    }
}

/// Largest resolved power of `card` over the member keys selected by `keys`
/// (bit `k` = member key `k`, i.e. `shared_unit * 2 + shared_attr`) and over
/// every unit profile of the card.
pub(super) fn power_over_keys(pool: &CardPool, card: CardIdx, keys: u8) -> u32 {
    let values = pool.power_values(card);
    let lut = pool.power_lut(card);
    let units = pool.unit_mask_raw(card);
    let mut best = 0;
    for unit in 0..UNIT_COUNT {
        if units & (1 << unit) == 0 {
            continue;
        }
        let profile = ((lut >> (16 + unit)) & 1) as usize;
        for key in 0..4 {
            if keys & (1 << key) != 0 {
                best = best.max(decode_u18(values, lut, profile * 4 + key));
            }
        }
    }
    best
}

pub(super) struct RegimePlan {
    order: usize,
    pub(super) keep: Vec<bool>,
    pub(super) power_bound: Vec<u32>,
    pub(super) ceiling: u64,
    /// Features of every deck of the regime.
    feature_box: FeatureBox,
    /// The part of every deck's extra bonus that does not depend on its
    /// leader.
    shared_extra: u32,
}

impl RegimePlan {
    /// Admitted cards, their regime power bounds and an admissible ceiling on
    /// every deck of the regime, or `None` when the regime has no legal deck.
    pub(super) fn new(
        pool: &CardPool,
        ctx: &SearchContext,
        objective: &ObjectiveBound,
        regime: Regime,
        order: usize,
    ) -> Option<Self> {
        let keys = regime.member_keys();
        let mut keep = vec![false; pool.count()];
        let mut power_bound = vec![0u32; pool.count()];
        let mut best_power = [0u32; CHARACTER_COUNT];
        let mut best_skill = [0u32; CHARACTER_COUNT];
        let mut best_bonus = [0u32; CHARACTER_COUNT];
        let mut characters = 0u32;
        let mut attrs = 0u8;
        let mut leader_bonus = 0u32;
        let mut leader_skill_min = u32::MAX;
        // A same-character deck takes five distinct public cards instead.
        let unique = ctx.enforce_char_uniqueness;
        let mut cards = Vec::new();
        for card in pool.indices() {
            if !regime.admits(pool, card) {
                continue;
            }
            let character = usize::from(pool.char_id(card));
            let power = power_over_keys(pool, card, keys);
            keep[card.raw()] = true;
            power_bound[card.raw()] = power;
            best_power[character] = best_power[character].max(power);
            best_skill[character] = best_skill[character].max(u32::from(pool.skill_max(card)));
            best_bonus[character] = best_bonus[character].max(pool.event_bonus(card).total_ceil());
            characters |= 1 << character;
            attrs |= 1 << pool.attr(card);
            if may_lead(pool, ctx, card) {
                leader_skill_min = leader_skill_min.min(u32::from(pool.skill_max(card)));
            }
            if !unique {
                cards.push((
                    pool.game_id(card),
                    power,
                    u32::from(pool.skill_max(card)),
                    pool.event_bonus(card).total_ceil(),
                ));
            }
            if ctx.is_final_chapter {
                leader_bonus = leader_bonus.max(ctx.leader_bonus_upper_at(card.raw()));
            }
        }
        let (power_sum, bonus_sum, skill_sum, skill_peak) = if unique {
            if characters.count_ones() < DECK_SIZE as u32 {
                return None;
            }
            (
                top_sum(best_power),
                top_sum(best_bonus),
                top_sum(best_skill),
                best_skill.into_iter().max().unwrap_or(0),
            )
        } else {
            let mut ids = cards.iter().map(|card| card.0).collect::<Vec<_>>();
            ids.sort_unstable();
            ids.dedup();
            if ids.len() < DECK_SIZE {
                return None;
            }
            let top = |value: fn(&(u16, u32, u32, u32)) -> u32| {
                let mut values = cards.iter().map(value).collect::<Vec<_>>();
                values.sort_unstable_by(|left, right| right.cmp(left));
                values[..DECK_SIZE].iter().sum::<u32>()
            };
            (
                top(|card| card.1),
                top(|card| card.3),
                top(|card| card.2),
                cards.iter().map(|card| card.2).max().unwrap_or(0),
            )
        };
        if !admits_constraints(pool, ctx, &keep) {
            return None;
        }

        let (shared_extra, support_extra) = if ctx.is_world_bloom {
            let attribute_count = if regime.shares_attr() {
                1
            } else {
                attrs.count_ones() as usize
            };
            let diversity = (1..=attribute_count.min(DECK_SIZE))
                .map(|count| u32::from(ctx.diff_attr_bonus[count]))
                .max()
                .unwrap_or(0);
            (diversity, support_bonus_ceiling(ctx))
        } else {
            (ctx.extra_bonus_ub, 0)
        };
        let bonus = bonus_sum + shared_extra + support_extra + leader_bonus;
        let ceiling = objective.ceiling(power_sum, bonus, skill_sum, skill_peak);
        Some(Self {
            order,
            keep,
            power_bound,
            ceiling,
            feature_box: FeatureBox {
                power: power_sum,
                skill: skill_sum,
                card_skill: skill_peak,
                leader_skill_min,
                leader_skill_max: skill_peak,
                bonus,
            },
            shared_extra,
        })
    }

    /// Whether the log-linear bound rules out every deck of the regime at the
    /// event point of `threshold` (pruning-proof Section 13).
    pub(super) fn log_linear_excludes(
        &self,
        pool: &CardPool,
        ctx: &SearchContext,
        objective: &ObjectiveBound,
        threshold: u64,
    ) -> bool {
        if !ctx.enforce_char_uniqueness {
            return false;
        }
        let threshold_ep = threshold >> 32;
        let Some(bound) = LogLinearBound::new(objective, &self.feature_box, threshold_ep) else {
            return false;
        };
        // Per character, the largest weight of a member card, and the
        // largest weight of a card that leads, whose skill and
        // leader-dependent bonus count once more.
        let mut member = [0.0f64; CHARACTER_COUNT];
        let mut leader = [f64::NEG_INFINITY; CHARACTER_COUNT];
        for card in pool.indices().filter(|card| self.keep[card.raw()]) {
            let character = usize::from(pool.char_id(card));
            let skill = u32::from(pool.skill_max(card));
            let weight = bound.weigh(
                self.power_bound[card.raw()],
                skill,
                pool.event_bonus(card).total_ceil(),
            );
            member[character] = member[character].max(weight);
            if may_lead(pool, ctx, card) {
                let leading = sum_up([
                    weight,
                    mul_up(bound.leader_skill, f64::from(skill)),
                    mul_up(bound.bonus, f64::from(leader_extra(pool, ctx, card))),
                ]);
                leader[character] = leader[character].max(leading);
            }
        }
        let mut ranked: [usize; CHARACTER_COUNT] = core::array::from_fn(|character| character);
        ranked.sort_unstable_by(|&left, &right| member[right].total_cmp(&member[left]));
        let best = (0..CHARACTER_COUNT)
            .filter(|&character| leader[character].is_finite())
            .map(|character| {
                sum_up(
                    std::iter::once(leader[character]).chain(
                        ranked
                            .iter()
                            .filter(|&&other| other != character)
                            .take(DECK_SIZE - 1)
                            .map(|&other| member[other]),
                    ),
                )
            })
            .fold(f64::NEG_INFINITY, f64::max);
        let value = sum_up([
            bound.constant,
            mul_up(bound.bonus, f64::from(self.shared_extra)),
            best,
        ]);
        bound.excludes(
            value,
            threshold_ep,
            LogLinearBound::log_cutoff(threshold_ep),
        )
    }
}

/// Whether `card` may lead a deck: any card, or only one of the forced
/// leader character.
fn may_lead(pool: &CardPool, ctx: &SearchContext, card: CardIdx) -> bool {
    ctx.forced_leader_character_id
        .is_none_or(|character| pool.char_id(card) == character)
}

/// The part of a deck's extra bonus that depends on its leader `card`: its
/// Final Chapter leader bonus and, under World Bloom, the support profile of
/// its character.
fn leader_extra(pool: &CardPool, ctx: &SearchContext, card: CardIdx) -> u32 {
    let leader_bonus = if ctx.is_final_chapter {
        ctx.leader_bonus_upper_at(card.raw())
    } else {
        0
    };
    let support = if ctx.is_world_bloom {
        support_top(ctx.support_deck_for_leader(pool.char_id(card)))
    } else {
        0
    };
    leader_bonus + support
}

/// Fixed cards, fixed characters and a forced leader must all be admitted.
fn admits_constraints(pool: &CardPool, ctx: &SearchContext, keep: &[bool]) -> bool {
    let admitted = || pool.indices().filter(|card| keep[card.raw()]);
    ctx.fixed_card_ids
        .iter()
        .all(|&id| admitted().any(|card| pool.game_id(card) == id))
        && ctx
            .fixed_character_ids
            .iter()
            .chain(ctx.forced_leader_character_id.iter())
            .all(|&character| admitted().any(|card| pool.char_id(card) == character))
}

/// The counted support entries of any one profile, rounded up.
fn support_bonus_ceiling(ctx: &SearchContext) -> u32 {
    std::iter::once(&ctx.support_deck)
        .chain(&ctx.support_decks_by_character)
        .map(support_top)
        .max()
        .unwrap_or(0)
}

/// The first `count` entries of a support profile, rounded up. Profiles are
/// stored in non-increasing bonus order, so this is the largest sum any
/// main-deck exclusion can leave.
fn support_top(deck: &SupportDeck) -> u32 {
    deck.cards
        .iter()
        .take(usize::from(deck.count))
        .map(|(_, bonus)| *bonus)
        .sum::<f64>()
        .ceil() as u32
}

/// Sum of the five largest per-character values.
fn top_sum(mut values: [u32; CHARACTER_COUNT]) -> u32 {
    values.sort_unstable_by(|left, right| right.cmp(left));
    values[..DECK_SIZE].iter().sum()
}

/// Seeds the shared tracker once with `seed`, then runs `solve` once per regime
/// that can still reach the shared threshold and merges every legal result
/// through that canonical tracker. `solve` receives the regime's pool and
/// context, the external floor, and the seeds that lie inside the regime in
/// the regime's dense indices.
pub(super) fn search_regimes(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
    bounds_enabled: bool,
    seed: impl FnOnce(&mut SearchBudget, &mut SearchStats) -> Vec<DeckResult>,
    mut solve: impl FnMut(
        &CardPool,
        &SearchContext,
        u64,
        Vec<DeckResult>,
        &mut SearchBudget,
    ) -> (Vec<DeckResult>, SearchStats),
) -> (Vec<DeckResult>, SearchStats) {
    let objective = ObjectiveBound::from_context(ctx);
    let mut plans = Regime::all()
        .enumerate()
        .filter_map(|(order, regime)| RegimePlan::new(pool, ctx, &objective, regime, order))
        .collect::<Vec<_>>();
    plans.sort_unstable_by(|left, right| {
        right
            .ceiling
            .cmp(&left.ceiling)
            .then(left.order.cmp(&right.order))
    });

    let mut tracker = TopKTracker::new(params.top_k);
    tracker.set_bounds_enabled(bounds_enabled);
    let mut stats = SearchStats::default();
    let seeds = seed(budget, &mut stats)
        .into_iter()
        .filter_map(|seed| canonicalize_seed_result(pool, ctx, seed))
        .collect::<Vec<_>>();
    for &seed in &seeds {
        tracker.insert(pool, ctx, seed);
    }
    for plan in plans {
        if budget.expired() {
            break;
        }
        let cutoff = tracker.rank_threshold(ctx.target);
        if cutoff != 0
            && (plan.ceiling < cutoff || plan.log_linear_excludes(pool, ctx, &objective, cutoff))
        {
            stats.diagnostics.regimes_pruned += 1;
            continue;
        }
        stats.diagnostics.regimes_searched += 1;
        let mut dense = vec![None; pool.count()];
        let mut original = Vec::new();
        for (index, _) in plan.keep.iter().enumerate().filter(|(_, keep)| **keep) {
            dense[index] = Some(CardIdx::new(original.len() as u16));
            original.push(CardIdx::new(index as u16));
        }
        let regime_seeds = seeds
            .iter()
            .filter_map(|&seed| {
                let mut local = seed;
                for card in &mut local.cards {
                    *card = dense[card.raw()]?;
                }
                Some(local)
            })
            .collect();
        let restricted = pool.restrict(&plan.keep, &plan.power_bound);
        let restricted_ctx = ctx.remap(&plan.keep);
        debug_assert!(restricted.count() == original.len());
        let (results, part) = solve(
            &restricted,
            &restricted_ctx,
            tracker.threshold(),
            regime_seeds,
            budget,
        );
        stats.accumulate(&part);
        for mut result in results {
            for card in &mut result.cards {
                *card = original[card.raw()];
            }
            tracker.insert(pool, ctx, result);
        }
    }
    stats.deadline_hit |= budget.hit;
    (tracker.into_vec(), stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_regime_is_listed_once() {
        let regimes = Regime::all().collect::<Vec<_>>();
        assert_eq!(regimes.len(), 1 + 6 + 6 + 36);
        for (index, regime) in regimes.iter().enumerate() {
            assert!(!regimes[..index].contains(regime));
        }
    }

    #[test]
    fn member_keys_cover_every_key_of_the_regime() {
        // (shared unit reachable, shared attribute) -> key bit
        for regime in Regime::all() {
            let keys = regime.member_keys();
            let attr = regime.shares_attr();
            let key_of =
                |shared_unit: bool| 1u8 << (usize::from(shared_unit) * 2 + usize::from(attr));
            assert_ne!(keys & key_of(false), 0, "{regime:?}");
            if matches!(regime, Regime::SharedUnit(_) | Regime::SharedUnitAttr(..)) {
                assert_ne!(keys & key_of(true), 0, "{regime:?}");
            }
        }
    }
}
