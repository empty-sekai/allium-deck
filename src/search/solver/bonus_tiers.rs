//! Exact per-tier Top-K for `target = bonus` with a requested tier list.
//!
//! Every requested tier `T` independently keeps the canonical Top-K of legal
//! ordered decks whose evaluated total bonus is exactly `T`, ranked by live
//! score. The proof is `docs/pruning-proof.md` §17; this comment only maps it
//! to the code.
//!
//! * A deck takes one card from each of five distinct *groups*. The fixed
//!   roles come first in slot order: fixed cards, fixed characters and, in the
//!   Final Chapter, the leader slot. Free groups follow: one per character when
//!   characters are unique, one per public card inside a Challenge character.
//!   Fixed roles and a forced leader's character are mandatory groups.
//! * Every card has an integer *key* `κ`, a *slack* `σ ≥ 0` (in ticks: a
//!   tick is a tenth of a percent divided by the request's tick scale, the
//!   smallest step at which every support entry is whole) and a count `q` of
//!   support entries it can displace, such that
//!   every deck `D` of a diversity class `v` satisfies
//!   `Σκ + E_lo - X(Σq) ≤ 10·total(D) ≤ Σκ + Σσ + E_hi`, where
//!   `[E_lo, E_hi]` bounds the deck-level terms and `X` is a monotone excess
//!   bound with `X(0) = X(1) = 0`. Without World Bloom `κ` is the exact
//!   counted card bonus and slack, excess and deck terms vanish; with World
//!   Bloom each card's own support loss is folded into `κ`, and the
//!   attribute-diversity bonus is fixed by `v`.
//! * The feasible decks are covered by the area-item composition regimes of
//!   `composition.rs`. Inside a regime every admitted card has an additive
//!   power bound, so the per-regime search below is complete for the decks of
//!   that regime; decks of other regimes it happens to visit are evaluated
//!   exactly and never needed.
//! * Per regime a suffix table over the group order stores, for every
//!   position, remaining card count, limited-bonus counting state and key sum,
//!   the componentwise maxima of power bound, skill sum and maximum skill over
//!   all selections of the suffix groups. A missing entry proves the key sum
//!   unreachable. Bit rows that follow the same steps without the maxima
//!   first find the sums of the regime's full decks; a tier and diversity
//!   class they miss has no deck there, and the table is built only when
//!   some requested pair remains. The regime that admits every card does
//!   the same for its whole scope.
//! * When the live score is a product of power and a rate affine in skill,
//!   the table also keeps each entry's largest weighted sum of power and
//!   skill, and the product is bounded over the box of the separate maxima
//!   cut by that half-plane.
//! * A depth-first branch and bound per regime, tier and diversity class takes
//!   or skips one group at a time. A branch survives only while some
//!   completion can still hit the tier and the live-score ceiling of those
//!   completions is at least the tier's K-th live score; a selected card whose
//!   skill depends on the deck composition enters that ceiling with the
//!   largest value the reachable compositions allow. Leaves are evaluated
//!   with the shared placement semantics and inserted into every tier they
//!   hit exactly.
//! * Once the selected cards hold a diversity class's largest attribute
//!   count, every completion keeps to their attributes; a large enough search
//!   continues in the regime restricted to the cards of those attributes,
//!   with its own suffix table.
//! * With one support profile of whole ticks every deck's total is exact in
//!   its cards, its count of removed counted entries and the positions past
//!   them it holds. The refill of the selected cards bounds every
//!   completion's, and a large enough search builds the scope's certificate
//!   of the tiers no deck reaches and ends when its tier is one of them.
use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::pool::{CardIdx, CardPool};
use crate::search::SearchStats;
use crate::search::budget::SearchBudget;
use crate::search::composition::{Regime, power_over_keys};
use crate::search::context::{SearchContext, SupportDeck};
use crate::search::evaluate::resolve_total_bonus;
use crate::search::objective::{LiveProduct, ObjectiveBound};
use crate::search::placement::visit_bonus_candidates;
use crate::search::skill_ceiling::{CeilingSet, Composition, SkillCeiling};
use crate::search::tracker::TopKTracker;
use crate::search::tuning::SearchTuning;
use crate::search::types::{DeckResult, SearchParams};
use crate::types::{ATTR_COUNT, DECK_SIZE, LiveType};

/// Card counts `0..=DECK_SIZE` of a suffix selection.
const COUNTS: usize = DECK_SIZE + 1;
/// Visited nodes of one regime between tests whether its table is rebuilt
/// with the joint column, so that the table pass stays small next to the
/// search.
const JOIN_AFTER: u64 = 1 << 16;
/// A regime gains the joint column once its bound tests prune at least once
/// per this many feasibility prunes; the column tightens only bound tests.
const JOIN_RATIO: u64 = 4;
/// Nodes of a regime's searches visited with one attribute set at the
/// largest attribute count of a diversity class before the completions get
/// the view and table of that set, so that small searches build none.
const RESTRICT_AFTER: u64 = 1 << 11;
/// Nodes of one tier search before it builds and consults the scope's tier
/// certificate, so that small searches build none.
const CERTIFY_AFTER: u64 = 1 << 12;
/// Cards holding support entries past the counted ones that a certificate
/// enumerates as subsets; more leave the scope without one.
const CERTIFY_HELD_CARDS: usize = 16;
/// Unreachable table entry. Adding five card values keeps it negative.
const NO_POWER: i32 = i32::MIN / 4;
const NO_SKILL: i16 = i16::MIN / 4;
const NO_JOINT: i32 = i32::MIN / 4;
/// Deck-level terms without a finite bound.
const UNBOUNDED: (i64, i64) = (i64::MIN / 4, i64::MAX / 4);
/// Tolerance for real-valued support sums converted to ticks. Every bound
/// is compared with the integer tier in ticks, so an accumulated error below
/// one tick cannot change a comparison.
const ROUNDING: f64 = 1e-6;
/// Candidate numbers of ticks per tenth of a percent, smallest first.
const TICK_SCALES: [i64; 6] = [1, 2, 4, 5, 10, 20];

/// Searches every requested tier exactly; results are grouped per tier in
/// descending tier order, each group in canonical order.
pub(crate) fn search(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    targets: &[i32],
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    let mut stats = SearchStats::default();
    let mut targets = targets
        .iter()
        .copied()
        .filter(|target| *target >= 0)
        .collect::<Vec<_>>();
    targets.sort_unstable_by(|left, right| right.cmp(left));
    targets.dedup();
    if params.top_k == 0 || targets.is_empty() || pool.count() < DECK_SIZE {
        return (Vec::new(), stats);
    }
    // The live-score relaxation has no MySekai arm, so that live type keeps
    // only the exact reachability proof.
    let bounds_enabled =
        SearchTuning::load().bounds && ctx.effective_live_type() != LiveType::Mysekai;
    let mut trackers = targets
        .iter()
        .map(|_| {
            let mut tracker = TopKTracker::new(params.top_k);
            tracker.set_bounds_enabled(bounds_enabled);
            tracker
        })
        .collect::<Vec<_>>();
    let ceilings = pool
        .indices()
        .map(|card| SkillCeiling::new(pool, card, ctx.skill_reference_strategy))
        .collect::<Vec<_>>();
    let mut problems = scopes(pool, ctx)
        .into_iter()
        .filter_map(|scope| Problem::new(pool, ctx, &targets, &ceilings, scope))
        .collect::<Vec<_>>();
    // The regimes of every scope, strongest first: the shared tier cutoffs
    // then rise as early as the regime ceilings allow, whatever scope the
    // strong regimes belong to.
    let powers = RegimePowers::new(pool);
    let mut table = SuffixTable::default();
    for problem in &mut problems {
        problem.find_unreachable(powers.of(Regime::Mixed), &mut table);
    }
    let mut regimes = problems
        .iter()
        .enumerate()
        .flat_map(|(scope, problem)| {
            let powers = &powers;
            Regime::all().filter_map(move |regime| {
                problem
                    .regime_bounds(regime, powers.of(regime))
                    .map(|bounds| (scope, bounds))
            })
        })
        .collect::<Vec<_>>();
    regimes.sort_by_key(|(_, bounds)| std::cmp::Reverse(bounds.ceiling));
    let mut joined = vec![false; problems.len()];
    for (scope, bounds) in regimes {
        if budget.expired() {
            break;
        }
        problems[scope].solve(
            &bounds,
            powers.of(bounds.regime),
            &mut joined[scope],
            &mut table,
            &mut trackers,
            budget,
            &mut stats,
        );
    }
    stats.deadline_hit |= budget.hit;
    let results = trackers
        .into_iter()
        .flat_map(TopKTracker::into_vec)
        .collect();
    (results, stats)
}

/// The decks one [`Problem`] searches; together the scopes of a request
/// cover every legal deck exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Character-unique decks of the whole pool.
    Unique,
    /// Character-unique Final Chapter decks whose leader has this character,
    /// which fixes the support profile.
    Leader(u8),
    /// Challenge decks of this character.
    Character(u8),
}

impl Scope {
    fn admits(self, pool: &CardPool, card: CardIdx) -> bool {
        match self {
            Self::Unique | Self::Leader(_) => true,
            Self::Character(character) => pool.char_id(card) == character,
        }
    }
}

/// Challenge decks split by character. Final Chapter decks split by leader
/// character, so that each search folds one support profile into its card
/// keys instead of the extremes over every leader's profile.
fn scopes(pool: &CardPool, ctx: &SearchContext) -> Vec<Scope> {
    let characters = |cards: &mut dyn Iterator<Item = CardIdx>| {
        let mut characters = cards.map(|card| pool.char_id(card)).collect::<Vec<_>>();
        characters.sort_unstable();
        characters.dedup();
        characters
    };
    if !ctx.enforce_char_uniqueness {
        return characters(&mut pool.indices())
            .into_iter()
            .map(Scope::Character)
            .collect();
    }
    if ctx.is_final_chapter && ctx.is_world_bloom {
        let leaders = pool
            .indices()
            .filter(|&card| ctx.card_matches_slot(pool, 0, card));
        return characters(&mut { leaders })
            .into_iter()
            .map(Scope::Leader)
            .collect();
    }
    vec![Scope::Unique]
}

/// How the event counts limited card bonuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Counting {
    /// Every card contributes its whole bonus.
    All,
    /// Base bonus always counts; limited bonus counts only for the first
    /// `cap` cards (in slot order) whose limited bonus is positive.
    FirstN(u8),
}

impl Counting {
    fn capacity(self) -> u8 {
        match self {
            Self::All => 0,
            Self::FirstN(cap) => cap,
        }
    }
}

/// A role group of one scope: the deck takes at most one of its cards.
#[derive(Clone, Debug)]
struct Group {
    /// Occupies a fixed slot, which precedes every free slot.
    fixed_role: bool,
    /// Final Chapter leader slot: its card also adds the leader-only bonus.
    leader_role: bool,
    /// The deck must take one card of this group.
    mandatory: bool,
    cards: Vec<CardIdx>,
}

/// Cards of one group sharing their bonus parts, with componentwise maxima.
#[derive(Clone, Copy, Debug)]
struct Class {
    /// Key `κ` without the limited bonus, in ticks, including the support
    /// adjustment and, on the Final leader role, the leader-only bonus.
    key_ticks: i32,
    /// Limited bonus of the class, in ticks; zero under `Counting::All`.
    limited_ticks: u32,
    /// Slack `σ` of the class, in ticks.
    slack_ticks: u32,
    /// Displaceable support entries of the class's public ids.
    support: u32,
    /// Under the scope's single support profile, the counted entries the
    /// class's public id removes and the positions past them it holds; zero
    /// without one.
    removed: u8,
    held: u64,
    /// Attributes of the class's cards, one bit each.
    attrs: u8,
    power: u32,
    skill: u32,
    leader: u32,
    /// Largest joint value of a card of the class; zero without a joint
    /// ceiling.
    joint: i32,
    start: u32,
    end: u32,
}

impl Class {
    fn parts(&self) -> ClassParts {
        (
            self.key_ticks,
            self.limited_ticks,
            self.slack_ticks,
            self.support,
            self.removed,
            self.held,
        )
    }
}

/// The parts a class shares: key, limited bonus, slack and displaceable
/// entries in ticks, then the removed counted entries and held positions.
type ClassParts = (i32, u32, u32, u32, u8, u64);

/// A group restricted to one regime, with its cards ordered by class.
#[derive(Clone, Debug)]
struct RegimeGroup {
    fixed_role: bool,
    leader_role: bool,
    mandatory: bool,
    cards: Vec<CardIdx>,
    classes: Vec<Class>,
    /// Largest composition-aware skill ceiling of the group's cards.
    ceilings: CeilingSet,
}

/// Bonus parts of one card, in ticks.
#[derive(Clone, Copy, Debug, Default)]
struct CardBonus {
    /// Always counted: base bonus, or the whole card bonus under `All`.
    fixed: u32,
    /// Counted only within the first-`cap` rule.
    limited: u32,
    /// Lower support adjustment: a non-positive multiple of the unit.
    adjust: i32,
    /// Width of the support adjustment interval.
    slack: u32,
    /// Support entries of the card's public id that a deck can displace.
    support: u32,
}

/// Deck-level World Bloom terms, in ticks.
struct Extras {
    diversity: [u16; 6],
    /// Outward range of ten times the support base, for any leader.
    base: (i64, i64),
    /// Final Chapter: the same range per leader character.
    by_leader: Option<Vec<(i64, i64)>>,
    /// `excess[q]`: ten times the largest extra support loss of a deck that
    /// displaces `q` entries, rounded up; the last entry covers larger `q`.
    excess: Vec<i64>,
    /// Support entries are finite, non-negative and non-increasing; otherwise
    /// the deck-level terms are left unbounded.
    bounded: bool,
}

/// The support decks a scope can use and, in the Final Chapter, the deck of
/// each leader character.
struct SupportProfiles<'a> {
    decks: Vec<&'a SupportDeck>,
    /// Final Chapter: index into `decks` by leader character.
    leader_profile: Vec<usize>,
}

impl<'a> SupportProfiles<'a> {
    /// A Final Chapter scope of one leader character uses that character's
    /// deck alone; any other Final Chapter scope uses every leader's deck.
    fn new(ctx: &'a SearchContext, scope: Scope) -> Self {
        if let (true, Scope::Leader(character)) = (ctx.is_final_chapter, scope) {
            return Self {
                decks: vec![ctx.support_deck_for_leader(character)],
                leader_profile: vec![0; usize::from(u8::MAX) + 1],
            };
        }
        let mut decks = vec![&ctx.support_deck];
        let mut leader_profile = Vec::new();
        if ctx.is_final_chapter {
            for character in 0..=u8::MAX {
                let deck = ctx.support_deck_for_leader(character);
                if std::ptr::eq(deck, &ctx.support_deck) {
                    leader_profile.push(0);
                } else {
                    decks.push(deck);
                    leader_profile.push(decks.len() - 1);
                }
            }
        }
        Self {
            decks,
            leader_profile,
        }
    }
}

/// Support base, per-public-card loss and excess bounds of one profile.
///
/// With entries `s_1 ≥ s_2 ≥ …` (entries past the end count as zero), `W`
/// counted entries and at most `M` entries held by five main cards, a deck's
/// loss against the base `Σ_{i ≤ W} s_i` is `Σ ℓ_c + X`: `ℓ_c` sums
/// `s_i - s_{W+1}` over card `c`'s own entries with `i ≤ W`, and
/// `0 ≤ X ≤ Σ_{k ≤ q} (s_{W+1} - s_{W+k})`, where `q` counts the deck's
/// entries with `i ≤ W + M`.
struct SupportTerms {
    base: f64,
    /// Per public id: `ℓ_c`, its entries with `i ≤ W`, its entries with
    /// `W < i ≤ W + M`, and those positions as bits from `W + 1` (positions
    /// past the 64th are left out).
    loss: HashMap<u16, (f64, u32, u32, u64)>,
    /// `excess[q]` for `q = 0..=M`.
    excess: Vec<f64>,
    /// `s_{W+1} - s_{W+k}` for `k = 1..=2M`.
    steps: Vec<f64>,
}

impl SupportTerms {
    /// `None` unless every entry is finite, non-negative and not larger than
    /// the one before it.
    fn new(deck: &SupportDeck) -> Option<Self> {
        let entries = &deck.cards;
        let ordered = entries
            .iter()
            .all(|(_, bonus)| bonus.is_finite() && *bonus >= 0.0)
            && entries.windows(2).all(|pair| pair[0].1 >= pair[1].1);
        if !ordered {
            return None;
        }
        let counted = usize::from(deck.count);
        let mut ids = entries
            .iter()
            .map(|(game_id, _)| *game_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let mut multiplicities = ids
            .chunk_by(|left, right| left == right)
            .map(<[u16]>::len)
            .collect::<Vec<_>>();
        multiplicities.sort_unstable_by(|left, right| right.cmp(left));
        let removable = multiplicities.iter().take(DECK_SIZE).sum::<usize>();
        let value = |index: usize| entries.get(index).map_or(0.0, |entry| entry.1);
        let reference = value(counted);
        let mut loss: HashMap<u16, (f64, u32, u32, u64)> = HashMap::new();
        for (index, &(game_id, bonus)) in entries.iter().enumerate() {
            if index >= counted + removable {
                break;
            }
            let entry = loss.entry(game_id).or_default();
            if index < counted {
                entry.0 += bonus - reference;
                entry.1 += 1;
            } else {
                entry.2 += 1;
                if index - counted < 64 {
                    entry.3 |= 1 << (index - counted);
                }
            }
        }
        let steps = (0..2 * removable)
            .map(|index| reference - value(counted + index))
            .collect::<Vec<_>>();
        let mut excess = vec![0.0; removable + 1];
        for displaced in 1..=removable {
            excess[displaced] = excess[displaced - 1] + steps[displaced - 1];
        }
        Some(Self {
            base: entries.iter().take(counted).map(|entry| entry.1).sum(),
            loss,
            excess,
            steps,
        })
    }

    fn loss_of(&self, game_id: u16) -> (f64, u32) {
        let (loss, counted, past, _) = self.entries(game_id);
        (loss, counted + past)
    }

    fn entries(&self, game_id: u16) -> (f64, u32, u32, u64) {
        self.loss.get(&game_id).copied().unwrap_or_default()
    }

    /// `X` of a deck that removes `removed` counted entries and holds the
    /// positions `held` past them: the refill takes the first `removed`
    /// positions after `W` that the deck does not hold.
    fn refill(&self, removed: usize, held: u64) -> f64 {
        (0..self.steps.len())
            .filter(|&index| index >= 64 || held & (1 << index) == 0)
            .take(removed)
            .map(|index| self.steps[index])
            .sum()
    }

    /// Excess bound for `displaced` entries; monotone in `displaced`.
    fn excess_of(&self, displaced: usize) -> f64 {
        self.excess[displaced.min(self.excess.len() - 1)]
    }
}

/// Integer range of `value` percent in ticks, exact for values within
/// `ROUNDING` of a whole tick.
fn ticks(value: f64, scale: i64) -> (i64, i64) {
    let scaled = value * 10.0 * scale as f64;
    (
        (scaled + ROUNDING).floor() as i64,
        (scaled - ROUNDING).ceil() as i64,
    )
}

/// Ticks per tenth of a percent: the smallest candidate at which every entry
/// of `decks` is a whole number of ticks. Bases, losses and excess bounds
/// are sums and differences of entries and are then whole as well.
fn tick_scale(decks: &[&SupportDeck]) -> Option<i64> {
    TICK_SCALES.into_iter().find(|&scale| {
        decks.iter().all(|deck| {
            deck.cards.iter().all(|&(_, bonus)| {
                let (low, high) = ticks(bonus, scale);
                low == high
            })
        })
    })
}

/// Exact search of one scope: the whole pool under character uniqueness, or
/// one Challenge character.
struct Problem<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    objective: ObjectiveBound,
    targets: &'a [i32],
    counting: Counting,
    unique_characters: bool,
    /// Ticks per tenth of a percent.
    scale: i64,
    /// Every card's key plus slack is its exact contribution: the support
    /// entries are whole ticks and one profile is in use, or there is no
    /// support deck. The slack of the selected cards is then known exactly.
    exact_slack: bool,
    /// Common divisor of every key contribution, in ticks.
    unit: u32,
    /// Per-card shift that makes every key non-negative; a unit multiple.
    offset_ticks: i64,
    /// Largest shifted key sum the table represents, in units.
    max_units: usize,
    /// Largest shifted key of one card, in units.
    widest_units: usize,
    bonus: Vec<CardBonus>,
    groups: Vec<Group>,
    /// Class parts of every group's cards, in the group's card order; the
    /// key includes the leader-only bonus on the Final leader role.
    parts: Vec<Vec<ClassParts>>,
    extras: Option<Extras>,
    /// Composition-aware skill ceiling of every card, by pool index.
    ceilings: &'a [SkillCeiling],
    /// The live score as a product of power and rate, when it is one.
    product: Option<LiveProduct>,
    /// Nodes visited with one attribute set before its completions are
    /// searched in the view of that set: `RESTRICT_AFTER`.
    restrict_after: u64,
    /// The scope's single support profile, when its losses are whole ticks.
    terms: Option<SupportTerms>,
    /// Every position a deck can hold past the counted entries fits the
    /// profile's held mask, so a complete deck's refill is exact.
    refill_exact: bool,
    /// `(tier, diversity bonus)` pairs no deck of the scope reaches, from
    /// the root sums of the regime that admits every card.
    unreachable: Vec<(usize, i64)>,
    /// The scope's tier certificate, built by the first tier search that
    /// visits `certify_after` nodes; `None` inside when the scope has none.
    certificate: OnceCell<Option<TierCertificate>>,
    certify_after: u64,
}

impl<'a> Problem<'a> {
    fn new(
        pool: &'a CardPool,
        ctx: &'a SearchContext,
        targets: &'a [i32],
        ceilings: &'a [SkillCeiling],
        scope: Scope,
    ) -> Option<Self> {
        let in_scope = |card: CardIdx| scope.admits(pool, card);
        if let Scope::Character(character) = scope
            && ctx
                .forced_leader_character_id
                .is_some_and(|forced| forced != character)
        {
            return None;
        }
        let limit = ctx.card_bonus_count_limit;
        let limited_rule = limit < DECK_SIZE && (ctx.is_final_chapter || !ctx.is_world_bloom);
        let any_limited = pool
            .indices()
            .any(|card| in_scope(card) && pool.event_bonus_exact(card).limited_x10() > 0);
        let counting = if limited_rule && any_limited {
            Counting::FirstN(limit as u8)
        } else {
            Counting::All
        };

        let groups = Self::groups(pool, ctx, scope)?;
        let support = ctx.is_world_bloom.then(|| SupportProfiles::new(ctx, scope));
        let found_scale = support
            .as_ref()
            .map_or(Some(1), |support| tick_scale(&support.decks));
        let scale = found_scale.unwrap_or(1);
        let exact_slack = found_scale.is_some()
            && support
                .as_ref()
                .is_none_or(|support| support.decks.len() == 1);
        let per_tenth = scale as u32;
        let leader_extra = |card: CardIdx| {
            (ctx.leader_honor_bonus_x10_at(card.raw()) + ctx.leader_limit_bonus_x10_at(card.raw()))
                * per_tenth
        };
        let mut bonus = vec![CardBonus::default(); pool.count()];
        for card in pool.indices() {
            let exact = pool.event_bonus_exact(card);
            bonus[card.raw()] = if counting == Counting::All {
                CardBonus {
                    fixed: exact.total_x10() * per_tenth,
                    ..CardBonus::default()
                }
            } else {
                CardBonus {
                    fixed: exact.base_x10() * per_tenth,
                    limited: exact.limited_x10() * per_tenth,
                    ..CardBonus::default()
                }
            };
        }
        // One exact unit for every card contribution a transition can add.
        let mut unit = 0u32;
        for group in &groups {
            for &card in &group.cards {
                let parts = bonus[card.raw()];
                let fixed = parts.fixed
                    + if group.leader_role {
                        leader_extra(card)
                    } else {
                        0
                    };
                unit = gcd(gcd(unit, fixed), parts.limited);
            }
        }
        let unit = unit.max(1);

        let (extras, terms) = match support {
            Some(support) => {
                let (extras, terms) =
                    Self::fold_support(pool, ctx, &support, scale, unit, &mut bonus);
                (Some(extras), terms.filter(|_| exact_slack))
            }
            None => (None, None),
        };
        let mut lowest_key = 0i64;
        let mut group_max = Vec::with_capacity(groups.len());
        for group in &groups {
            let mut best = i64::MIN;
            for &card in &group.cards {
                let parts = bonus[card.raw()];
                let key = i64::from(parts.fixed)
                    + i64::from(parts.adjust)
                    + if group.leader_role {
                        i64::from(leader_extra(card))
                    } else {
                        0
                    };
                lowest_key = lowest_key.min(key);
                best = best.max(key + i64::from(parts.limited));
            }
            group_max.push(best);
        }
        let refill_parts = |card: CardIdx| {
            terms.as_ref().map_or((0, 0), |terms| {
                let (_, removed, _, held) = terms.entries(pool.game_id(card));
                (removed.min(u32::from(u8::MAX)) as u8, held)
            })
        };
        let parts = groups
            .iter()
            .map(|group| {
                group
                    .cards
                    .iter()
                    .map(|&card| {
                        let parts = bonus[card.raw()];
                        let leader = if group.leader_role {
                            leader_extra(card)
                        } else {
                            0
                        };
                        let (removed, held) = refill_parts(card);
                        (
                            parts.fixed as i32 + parts.adjust + leader as i32,
                            parts.limited,
                            parts.slack,
                            parts.support,
                            removed,
                            held,
                        )
                    })
                    .collect()
            })
            .collect();
        let unit_i = i64::from(unit);
        let offset_ticks = (-lowest_key + unit_i - 1) / unit_i * unit_i;
        group_max.sort_unstable_by(|left, right| right.cmp(left));
        let widest_units = group_max
            .first()
            .map_or(0, |best| ((best + offset_ticks).max(0) / unit_i) as usize);
        let reachable = group_max
            .iter()
            .take(DECK_SIZE)
            .map(|best| best + offset_ticks)
            .sum::<i64>();
        // Deck-level terms are at least -1 tenth minus the support excess
        // unless they are unbounded, so a hitting key sum never exceeds the
        // highest tier by more than that.
        let highest = i64::from(targets.iter().copied().max().unwrap_or(0).max(0)) * 10 * scale;
        let cap = match &extras {
            Some(extras) if !extras.bounded => reachable,
            Some(extras) => reachable.min(
                highest
                    + scale
                    + extras.excess.last().copied().unwrap_or(0)
                    + DECK_SIZE as i64 * offset_ticks,
            ),
            None => reachable.min(highest + scale + DECK_SIZE as i64 * offset_ticks),
        };
        let max_units = (cap.max(0) / unit_i) as usize;

        let objective = ObjectiveBound::from_context(ctx);
        let eager = SearchTuning::load().eager_bonus_tiers;
        Some(Self {
            pool,
            ctx,
            product: objective.live_product(),
            objective,
            targets,
            counting,
            unique_characters: !matches!(scope, Scope::Character(_)),
            scale,
            exact_slack,
            unit,
            offset_ticks,
            max_units,
            widest_units,
            bonus,
            groups,
            parts,
            extras,
            ceilings,
            restrict_after: if eager { 1 } else { RESTRICT_AFTER },
            refill_exact: terms.as_ref().is_some_and(|terms| terms.excess.len() <= 65),
            terms,
            unreachable: Vec::new(),
            certificate: OnceCell::new(),
            certify_after: if eager { 0 } else { CERTIFY_AFTER },
        })
    }

    /// Fixed roles in slot order, then the free groups of the scope.
    fn groups(pool: &CardPool, ctx: &SearchContext, scope: Scope) -> Option<Vec<Group>> {
        let in_scope = |card: CardIdx| scope.admits(pool, card);
        let unique = !matches!(scope, Scope::Character(_));
        let leader_role = |slot: usize| ctx.is_final_chapter && slot == 0;
        let fixed_slots = (ctx.fixed_card_ids.len() + ctx.fixed_character_ids.len()).min(DECK_SIZE);
        let role_slots = fixed_slots.max(usize::from(ctx.is_final_chapter));
        let mut groups = Vec::with_capacity(32);
        let mut excluded_characters = CharacterSet::default();
        let mut excluded_game_ids = Vec::new();
        for slot in 0..role_slots {
            let cards = pool
                .indices()
                .filter(|&card| {
                    in_scope(card)
                        && ctx.card_matches_slot(pool, slot, card)
                        && match scope {
                            Scope::Leader(character) if leader_role(slot) => {
                                pool.char_id(card) == character
                            }
                            _ => true,
                        }
                })
                .collect::<Vec<_>>();
            let first = *cards.first()?;
            if unique
                && cards
                    .iter()
                    .all(|&card| pool.char_id(card) == pool.char_id(first))
            {
                excluded_characters = excluded_characters.with(pool.char_id(first));
            }
            if cards
                .iter()
                .all(|&card| pool.game_id(card) == pool.game_id(first))
            {
                excluded_game_ids.push(pool.game_id(first));
            }
            groups.push(Group {
                fixed_role: true,
                leader_role: leader_role(slot),
                mandatory: true,
                cards,
            });
        }
        let forced_member = (!ctx.is_final_chapter)
            .then_some(ctx.forced_leader_character_id)
            .flatten();
        if let Some(forced) = forced_member
            && !excluded_characters.contains(forced)
            && !pool
                .indices()
                .any(|card| in_scope(card) && pool.char_id(card) == forced)
        {
            return None;
        }
        let mut free: Vec<(u32, Vec<CardIdx>)> = Vec::new();
        for card in pool.indices() {
            if !in_scope(card) || excluded_game_ids.contains(&pool.game_id(card)) {
                continue;
            }
            let key = if unique {
                if excluded_characters.contains(pool.char_id(card)) {
                    continue;
                }
                u32::from(pool.char_id(card))
            } else {
                u32::from(pool.game_id(card))
            };
            match free.iter_mut().find(|(group, _)| *group == key) {
                Some((_, cards)) => cards.push(card),
                None => free.push((key, vec![card])),
            }
        }
        free.sort_unstable_by_key(|(key, _)| *key);
        for (key, cards) in free {
            groups.push(Group {
                fixed_role: false,
                leader_role: false,
                mandatory: unique && forced_member.is_some_and(|forced| u32::from(forced) == key),
                cards,
            });
        }
        (groups.len() >= DECK_SIZE).then_some(groups)
    }

    /// Folds the support deck into each card's key, slack and displaced
    /// entry count and returns the deck-level World Bloom terms, with the
    /// terms of the profile when there is one. A Final Chapter scope of one
    /// leader character folds that character's profile; otherwise every
    /// leader profile is covered: card losses take their extremes over the
    /// profiles, counts and excess bounds their maxima.
    fn fold_support(
        pool: &CardPool,
        ctx: &SearchContext,
        support: &SupportProfiles<'_>,
        scale: i64,
        unit: u32,
        bonus: &mut [CardBonus],
    ) -> (Extras, Option<SupportTerms>) {
        let profiles = support
            .decks
            .iter()
            .map(|deck| SupportTerms::new(deck))
            .collect::<Vec<_>>();
        let leader_profile = &support.leader_profile;
        let bounded = profiles.iter().all(Option::is_some);
        let extras = |base: (i64, i64), by_leader, excess| Extras {
            diversity: ctx.diff_attr_bonus,
            base,
            by_leader,
            excess,
            bounded,
        };
        if !bounded {
            return (extras(UNBOUNDED, None, vec![0]), None);
        }
        let mut profiles = profiles.into_iter().flatten().collect::<Vec<_>>();
        let unit = i64::from(unit);
        for card in pool.indices() {
            let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut support = 0u32;
            for profile in &profiles {
                let (loss, displaced) = profile.loss_of(pool.game_id(card));
                low = low.min(loss);
                high = high.max(loss);
                support = support.max(displaced);
            }
            // The card adds −ℓ_c, which lies in [−high, −low].
            let floor = ticks(-high, scale).0;
            let ceil = ticks(-low, scale).1;
            let adjust = floor.div_euclid(unit) * unit;
            let parts = &mut bonus[card.raw()];
            parts.adjust = adjust as i32;
            parts.slack = (ceil - adjust) as u32;
            parts.support = support;
        }
        // A deck displaces at most `q` entries of every profile, and each
        // profile's excess bound is monotone, so the maximum over profiles
        // bounds every leader.
        let longest = profiles
            .iter()
            .map(|profile| profile.excess.len())
            .max()
            .unwrap_or(1);
        let excess = (0..longest)
            .map(|displaced| {
                profiles
                    .iter()
                    .map(|profile| ticks(profile.excess_of(displaced), scale).1.max(0))
                    .max()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>();
        let ranges = profiles
            .iter()
            .map(|profile| ticks(profile.base, scale))
            .collect::<Vec<_>>();
        let base = ranges.iter().fold((i64::MAX, i64::MIN), |acc, range| {
            (acc.0.min(range.0), acc.1.max(range.1))
        });
        let by_leader = ctx
            .is_final_chapter
            .then(|| leader_profile.iter().map(|&index| ranges[index]).collect());
        let single = if profiles.len() == 1 {
            profiles.pop()
        } else {
            None
        };
        (extras(base, by_leader, excess), single)
    }

    /// Whether the scope's certificate, once built, shows that no deck hits
    /// `tier` with the diversity bonus `extra_ticks`.
    fn excluded(&self, tier: usize, extra_ticks: i64) -> bool {
        self.unreachable.contains(&(tier, extra_ticks))
            || self
                .certificate
                .get()
                .and_then(Option::as_ref)
                .is_some_and(|certificate| certificate.excludes(tier, extra_ticks))
    }

    /// Debug builds build the regime's table and check that the root sums
    /// are exactly the sums it holds at its root.
    #[cfg(debug_assertions)]
    fn check_root_sums(&self, view: &RegimeView, sums: &SuffixTable, joined: bool) {
        let mut table = SuffixTable::default();
        table.fit(self);
        table.build(self, view, joined);
        let capacity = usize::from(self.counting.capacity());
        for sum in 0..=self.max_units {
            assert_eq!(
                sums.reaches_root(sum, sum),
                table.best(0, DECK_SIZE, capacity, 0, sum, sum).is_some(),
                "root sum {sum}"
            );
        }
    }

    /// Records the `(tier, diversity bonus)` pairs no deck of the scope
    /// reaches. The regime that admits every card, `Mixed`, holds every
    /// group and card of any other regime, so its root sums contain theirs;
    /// its suffix slack and displaced entries are the largest, so its needed
    /// ranges contain theirs. A pair its root misses is missed in every
    /// regime.
    fn find_unreachable(&mut self, power: &Rc<[u32]>, table: &mut SuffixTable) {
        let Some(bounds) = self.regime_bounds(Regime::Mixed, power) else {
            return;
        };
        let view = self.regime_view(&bounds, Rc::clone(power));
        table.fit(self);
        table.find_root_sums(self, &view);
        let mut extras = self
            .diversity_classes(false)
            .into_iter()
            .chain(self.diversity_classes(true))
            .map(|(extra_ticks, _)| extra_ticks)
            .collect::<Vec<_>>();
        extras.sort_unstable();
        extras.dedup();
        let root = State::root();
        let mut unreachable = Vec::new();
        for tier in 0..self.targets.len() {
            let target_ticks = i64::from(self.targets[tier]) * 10 * self.scale;
            for &extra_ticks in &extras {
                if !self
                    .needed(&view, target_ticks, extra_ticks, &root, None)
                    .is_some_and(|(low, high)| table.reaches_root(low, high))
                {
                    unreachable.push((tier, extra_ticks));
                }
            }
        }
        self.unreachable = unreachable;
    }

    /// The visited-node count at which a search starting at `visited`
    /// builds the certificate, while the scope can have one not yet built.
    fn certify_at(&self, visited: u64) -> Option<u64> {
        (self.terms.is_some() && self.certificate.get().is_none())
            .then(|| visited + self.certify_after)
    }

    fn certificate(&self) -> Option<&TierCertificate> {
        self.certificate
            .get_or_init(|| {
                self.terms
                    .as_ref()
                    .and_then(|terms| TierCertificate::new(self, terms))
            })
            .as_ref()
    }

    fn leader_extra_ticks(&self, card: CardIdx) -> u32 {
        (self.ctx.leader_honor_bonus_x10_at(card.raw())
            + self.ctx.leader_limit_bonus_x10_at(card.raw()))
            * self.scale as u32
    }

    /// The maxima and admissible ceiling of `regime` in the scope, with
    /// `power` its per-card power bounds, or `None` when the regime has no
    /// legal deck: every deck takes each role and at most `DECK_SIZE - roles`
    /// further groups.
    fn regime_bounds(&self, regime: Regime, power: &[u32]) -> Option<RegimeBounds> {
        let pool = self.pool;
        // Largest power and skill of each group's admitted cards.
        let mut roles: Vec<(u32, u32)> = Vec::new();
        let mut free: Vec<(u32, u32)> = Vec::new();
        for group in &self.groups {
            let best = group
                .cards
                .iter()
                .filter(|&&card| regime.admits(pool, card))
                .map(|&card| (power[card.raw()], u32::from(pool.skill_max(card))))
                .reduce(|left, right| (left.0.max(right.0), left.1.max(right.1)));
            match best {
                Some(best) if group.fixed_role => roles.push(best),
                Some(best) => free.push(best),
                None if group.mandatory => return None,
                None => {}
            }
        }
        if roles.len() + free.len() < DECK_SIZE {
            return None;
        }
        let free_slots = DECK_SIZE - roles.len();
        let top = |field: fn(&(u32, u32)) -> u32| {
            let mut values = free.iter().map(field).collect::<Vec<_>>();
            values.sort_unstable_by(|left, right| right.cmp(left));
            roles.iter().map(field).sum::<u32>() + values.iter().take(free_slots).sum::<u32>()
        };
        let (power, skill) = (top(|best| best.0), top(|best| best.1));
        let leader = roles
            .iter()
            .chain(&free)
            .map(|best| best.1)
            .max()
            .unwrap_or(0);
        Some(RegimeBounds {
            regime,
            power,
            skill,
            leader,
            ceiling: self.live_upper(power, skill, leader),
        })
    }

    /// The view of a regime with a legal deck: its groups, classes and
    /// joint ceiling, with `power` its per-card power bounds.
    fn regime_view(&self, bounds: &RegimeBounds, power: Rc<[u32]>) -> RegimeView {
        let pool = self.pool;
        let regime = bounds.regime;
        let mut roles = Vec::new();
        let mut free = Vec::new();
        for (group, parts) in self.groups.iter().zip(&self.parts) {
            // Admitted cards by class parts, then by falling power.
            let mut members = group
                .cards
                .iter()
                .zip(parts)
                .filter(|&(&card, _)| regime.admits(pool, card))
                .map(|(&card, &parts)| (parts, power[card.raw()], card))
                .collect::<Vec<_>>();
            if members.is_empty() {
                continue;
            }
            members.sort_unstable_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then(right.1.cmp(&left.1))
                    .then(left.2.cmp(&right.2))
            });
            let mut classes: Vec<Class> = Vec::new();
            for (index, &(parts, card_power, card)) in members.iter().enumerate() {
                let (key_ticks, limited_ticks, slack_ticks, support, removed, held) = parts;
                let skill = u32::from(pool.skill_max(card));
                match classes.last_mut() {
                    Some(class) if class.parts() == parts => {
                        class.attrs |= 1 << pool.attr(card);
                        class.power = class.power.max(card_power);
                        class.skill = class.skill.max(skill);
                        class.leader = class.leader.max(skill);
                        class.end = index as u32 + 1;
                    }
                    _ => classes.push(Class {
                        key_ticks,
                        limited_ticks,
                        slack_ticks,
                        support,
                        removed,
                        held,
                        attrs: 1 << pool.attr(card),
                        power: card_power,
                        skill,
                        leader: skill,
                        joint: 0,
                        start: index as u32,
                        end: index as u32 + 1,
                    }),
                }
            }
            let cards = members
                .into_iter()
                .map(|member| member.2)
                .collect::<Vec<_>>();
            let mut ceilings = CeilingSet::default();
            for card in &cards {
                ceilings.insert(self.ceilings[card.raw()]);
            }
            let view = RegimeGroup {
                fixed_role: group.fixed_role,
                leader_role: group.leader_role,
                mandatory: group.mandatory,
                cards,
                classes,
                ceilings,
            };
            if group.fixed_role {
                roles.push(view);
            } else {
                free.push(view);
            }
        }
        let best = |group: &RegimeGroup, field: fn(&Class) -> u32| {
            group.classes.iter().map(field).max().unwrap_or(0)
        };
        let joint = self.product.and_then(|product| {
            JointCeiling::new(product, bounds.power, bounds.skill, bounds.leader)
        });
        // The class maxima bound every card's joint value; below
        // `JointCeiling::LIMIT` five of them fit the table and keep an
        // unreachable entry negative.
        let joint = joint.filter(|joint| {
            roles.iter().chain(&free).all(|group| {
                group
                    .classes
                    .iter()
                    .all(|class| joint.value(class.power, class.skill) < JointCeiling::LIMIT)
            })
        });
        // Strong groups first: skipping one then lowers the ceiling at once.
        free.sort_by_key(|group| std::cmp::Reverse(best(group, |class| class.power)));
        let fixed_roles = roles.len();
        roles.extend(free);
        let mut view = RegimeView {
            diversity: self.diversity_classes(regime.shares_attr()),
            power,
            groups: roles,
            fixed_roles,
            suffix_slack: Vec::new(),
            suffix_support: Vec::new(),
            dynamic_from: Vec::new(),
            joint,
            joined: false,
        };
        view.order_groups();
        view
    }

    /// Prepares `view` for the joint column: every class's largest per-card
    /// joint value, and the free groups ordered by their largest joint
    /// value, which weighs skill against power as the joint ceiling does.
    fn join(&self, view: &mut RegimeView) {
        let Some(joint) = view.joint else {
            return;
        };
        if view.joined {
            return;
        }
        for group in &mut view.groups {
            let cards = &group.cards;
            for class in &mut group.classes {
                class.joint = cards[class.start as usize..class.end as usize]
                    .iter()
                    .map(|&card| {
                        joint.value(view.power[card.raw()], u32::from(self.pool.skill_max(card)))
                            as i32
                    })
                    .max()
                    .unwrap_or(0);
            }
        }
        view.groups[view.fixed_roles..].sort_by_key(|group| {
            std::cmp::Reverse(group.classes.iter().map(|class| class.joint).max())
        });
        view.order_groups();
        view.joined = true;
    }

    /// Diversity classes of the decks a regime must find: its attribute
    /// counts grouped by their diversity bonus, as `(bonus × 10, count mask)`.
    fn diversity_classes(&self, shares_attr: bool) -> Vec<(i64, u8)> {
        let Some(extras) = &self.extras else {
            return vec![(0, 0)];
        };
        let counts = if shares_attr { 1..=1 } else { 2..=DECK_SIZE };
        let mut classes: Vec<(i64, u8)> = Vec::new();
        for count in counts {
            let value = i64::from(extras.diversity[count]) * 10 * self.scale;
            match classes.iter_mut().find(|class| class.0 == value) {
                Some(class) => class.1 |= 1 << count,
                None => classes.push((value, 1 << count)),
            }
        }
        classes
    }

    fn live_upper(&self, power: u32, skill: u32, leader: u32) -> u32 {
        self.objective.ceiling(power, 0, skill, leader) as u32
    }

    /// Refill excess in ticks, rounded down and up, of a deck that removes
    /// `removed` counted entries and holds the positions `held` past them
    /// under the scope's single support profile; zero without one. The lower
    /// end is at most the excess of every deck that removes more and holds
    /// more.
    fn refill_ticks(&self, removed: u8, held: u64) -> (i32, i32) {
        self.terms.as_ref().map_or((0, 0), |terms| {
            let (low, high) = ticks(terms.refill(usize::from(removed), held), self.scale);
            (low.max(0) as i32, high.max(0) as i32)
        })
    }

    /// Deck-level terms of every completion of `state` in `view` with the
    /// diversity bonus `extra_ticks`, in ticks: the support excess is
    /// subtracted from the lower end and the selected cards' refill excess
    /// from the upper end. Once five cards are selected and the profile's
    /// held positions all fit the mask, their refill excess is the deck's,
    /// and it bounds the lower end too. `leader` is the card in the first
    /// slot once `state` holds one.
    #[inline(always)]
    fn extra_range(
        &self,
        view: &RegimeView,
        extra_ticks: i64,
        state: &State,
        leader: Option<CardIdx>,
    ) -> (i64, i64) {
        let Some(extras) = &self.extras else {
            return (0, 0);
        };
        if !extras.bounded {
            return UNBOUNDED;
        }
        let base = match (&extras.by_leader, leader) {
            (Some(by_leader), Some(leader)) => by_leader[usize::from(self.pool.char_id(leader))],
            _ => extras.base,
        };
        let remaining = DECK_SIZE - usize::from(state.picked);
        let displaced = state.support + view.suffix_support[usize::from(state.position)][remaining];
        let mut excess = extras.excess[(displaced as usize).min(extras.excess.len() - 1)];
        if remaining == 0 && self.refill_exact {
            excess = excess.min(i64::from(state.refill_high));
        }
        (
            extra_ticks + base.0 - excess,
            extra_ticks + base.1 - i64::from(state.refill_low),
        )
    }

    /// Inclusive range of the suffix's shifted key sum (in units) that can
    /// still complete `state` in `view` to `target_ticks` with the diversity
    /// bonus `extra_ticks`, or `None` when none can.
    #[inline(always)]
    fn needed(
        &self,
        view: &RegimeView,
        target_ticks: i64,
        extra_ticks: i64,
        state: &State,
        leader: Option<CardIdx>,
    ) -> Option<(usize, usize)> {
        let unit = i64::from(self.unit);
        let remaining = DECK_SIZE - usize::from(state.picked);
        let (extra_low, extra_high) = self.extra_range(view, extra_ticks, state, leader);
        // With exact slack the selected cards contribute exactly their key
        // plus slack sum; otherwise their slack only widens the interval.
        let (known, uncertain) = if self.exact_slack {
            (i64::from(state.slack_ticks), 0)
        } else {
            (0, i64::from(state.slack_ticks))
        };
        let rest = target_ticks - i64::from(state.key_ticks) - known;
        let shift = remaining as i64 * self.offset_ticks;
        let high = rest - extra_low + shift;
        if high < 0 {
            return None;
        }
        let slack =
            uncertain + i64::from(view.suffix_slack[usize::from(state.position)][remaining]);
        let low = rest - extra_high - slack + shift;
        let high_units = (high / unit).min(self.max_units as i64);
        let low_units = if low <= 0 { 0 } else { (low + unit - 1) / unit };
        (low_units <= high_units).then_some((low_units as usize, high_units as usize))
    }

    /// `view` limited to the cards with one of `attrs`, in the same group
    /// order, with its suffix table: each class keeps those cards and their
    /// maxima.
    fn restrict(&self, view: &RegimeView, attrs: u8) -> (RegimeView, SuffixTable) {
        let pool = self.pool;
        let groups = view
            .groups
            .iter()
            .map(|group| {
                let mut cards: Vec<CardIdx> = Vec::new();
                let mut classes = Vec::new();
                for class in group
                    .classes
                    .iter()
                    .filter(|class| class.attrs & attrs != 0)
                {
                    let start = cards.len();
                    cards.extend(
                        group.cards[class.start as usize..class.end as usize]
                            .iter()
                            .filter(|&&card| attrs & (1 << pool.attr(card)) != 0),
                    );
                    let mut kept = Class {
                        attrs: class.attrs & attrs,
                        power: 0,
                        skill: 0,
                        leader: 0,
                        joint: 0,
                        start: start as u32,
                        end: cards.len() as u32,
                        ..*class
                    };
                    for &card in &cards[start..] {
                        let (power, skill) =
                            (view.power[card.raw()], u32::from(pool.skill_max(card)));
                        kept.power = kept.power.max(power);
                        kept.skill = kept.skill.max(skill);
                        kept.leader = kept.leader.max(skill);
                        if let Some(joint) = view.joint.filter(|_| view.joined) {
                            kept.joint = kept.joint.max(joint.value(power, skill) as i32);
                        }
                    }
                    classes.push(kept);
                }
                RegimeGroup {
                    fixed_role: group.fixed_role,
                    leader_role: group.leader_role,
                    mandatory: group.mandatory,
                    cards,
                    classes,
                    ceilings: group.ceilings.clone(),
                }
            })
            .collect();
        let mut restricted = RegimeView {
            diversity: view.diversity.clone(),
            power: view.power.clone(),
            groups,
            suffix_slack: Vec::new(),
            suffix_support: Vec::new(),
            dynamic_from: Vec::new(),
            joint: view.joint,
            fixed_roles: view.fixed_roles,
            joined: view.joined,
        };
        restricted.order_groups();
        let mut table = SuffixTable::default();
        table.fit(self);
        table.build(self, &restricted, restricted.joined);
        (restricted, table)
    }

    /// Searches every tier in one regime of the scope, `power` its per-card
    /// power bounds.
    ///
    /// The joint column costs a table pass and tightens only bound tests;
    /// the scope keeps it (`joined`) from its first regime whose search
    /// passes a multiple of `JOIN_AFTER` visited nodes with at least one
    /// bound prune per `JOIN_RATIO` feasibility prunes, and that regime is
    /// searched again with it.
    #[allow(clippy::too_many_arguments)]
    fn solve(
        &self,
        bounds: &RegimeBounds,
        power: &Rc<[u32]>,
        joined: &mut bool,
        table: &mut SuffixTable,
        trackers: &mut [TopKTracker],
        budget: &mut SearchBudget,
        stats: &mut SearchStats,
    ) {
        let ceiling = bounds.ceiling;
        let open = |tracker: &TopKTracker| {
            tracker
                .cutoff()
                .is_none_or(|cutoff| ceiling >= cutoff as u32)
        };
        // A tier is wanted while its cutoff allows the regime and some
        // diversity class of the regime may still reach it.
        let diversity = self.diversity_classes(bounds.regime.shares_attr());
        let wanted = |tier: usize, tracker: &TopKTracker| {
            open(tracker)
                && diversity
                    .iter()
                    .any(|&(extra_ticks, _)| !self.excluded(tier, extra_ticks))
        };
        if !trackers
            .iter()
            .enumerate()
            .any(|(tier, tracker)| wanted(tier, tracker))
        {
            stats.diagnostics.regimes_pruned += 1;
            return;
        }
        stats.diagnostics.regimes_searched += 1;
        let mut view = self.regime_view(bounds, Rc::clone(power));
        table.fit(self);
        loop {
            if *joined {
                self.join(&mut view);
            }
            let view = &view;
            table.find_root_sums(self, view);
            #[cfg(debug_assertions)]
            self.check_root_sums(view, table, *joined);
            let mut built = false;
            // Only a diversity class whose largest attribute count is below
            // five limits the attributes of the completions.
            let limits = view
                .diversity
                .iter()
                .any(|&(_, counts)| counts != 0 && counts < 1 << DECK_SIZE);
            let restrictions = if limits {
                (0..1 << ATTR_COUNT)
                    .map(|_| Restriction::default())
                    .collect()
            } else {
                Vec::new()
            };
            let mut join = (!*joined && view.joint.is_some()).then(|| Join {
                after: stats.visited_nodes + JOIN_AFTER,
                bound_prunes: stats.ub_prunes,
                feasibility_prunes: stats.feasibility_prunes,
            });
            let mut stopped = false;
            'searches: for tier in 0..self.targets.len() {
                for &(extra_ticks, counts) in &view.diversity {
                    if budget.expired() {
                        return;
                    }
                    if !open(&trackers[tier]) {
                        stats.ub_prunes += 1;
                        continue;
                    }
                    let target_ticks = i64::from(self.targets[tier]) * 10 * self.scale;
                    let reaches = self
                        .needed(view, target_ticks, extra_ticks, &State::root(), None)
                        .is_some_and(|(low, high)| table.reaches_root(low, high));
                    if self.excluded(tier, extra_ticks) {
                        debug_assert!(
                            !reaches || !self.unreachable.contains(&(tier, extra_ticks)),
                            "a regime reaches a pair its scope excludes"
                        );
                        stats.feasibility_prunes += 1;
                        continue;
                    }
                    if !reaches {
                        stats.feasibility_prunes += 1;
                        continue;
                    }
                    if !built {
                        table.build(self, view, *joined);
                        built = true;
                    }
                    let certify_at = self.certify_at(stats.visited_nodes);
                    let mut search = TierSearch {
                        problem: self,
                        view,
                        table: &*table,
                        target_ticks,
                        extra_ticks,
                        counts,
                        tier,
                        trackers: &mut *trackers,
                        stats: &mut *stats,
                        budget: &mut *budget,
                        join,
                        stopped: false,
                        certify_at,
                        settled: false,
                        restrictions: &restrictions,
                        deck: [CardIdx::new(0); DECK_SIZE],
                        scratch: vec![Vec::new(); view.groups.len()],
                    };
                    search.run();
                    if search.stopped {
                        stopped = true;
                        break 'searches;
                    }
                    join = search.join;
                }
            }
            if !stopped {
                break;
            }
            *joined = true;
        }
    }
}

/// A regime of one scope with a legal deck, before its view is built.
#[derive(Clone, Copy, Debug)]
struct RegimeBounds {
    regime: Regime,
    /// Largest power sum, skill sum and single skill of the regime's decks
    /// from the per-group maxima.
    power: u32,
    skill: u32,
    leader: u32,
    /// Live-score ceiling of every deck of the regime.
    ceiling: u32,
}

/// Per-card power bounds of the composition regimes. A card's bound depends
/// on its regime only through the regime's member keys, so each key set has
/// one array, shared by every scope.
struct RegimePowers {
    kinds: Vec<(u8, Rc<[u32]>)>,
}

impl RegimePowers {
    fn new(pool: &CardPool) -> Self {
        let mut kinds: Vec<(u8, Rc<[u32]>)> = Vec::new();
        for keys in Regime::all().map(Regime::member_keys) {
            if kinds.iter().all(|kind| kind.0 != keys) {
                let power = pool
                    .indices()
                    .map(|card| power_over_keys(pool, card, keys))
                    .collect();
                kinds.push((keys, power));
            }
        }
        Self { kinds }
    }

    fn of(&self, regime: Regime) -> &Rc<[u32]> {
        let keys = regime.member_keys();
        &self
            .kinds
            .iter()
            .find(|kind| kind.0 == keys)
            .expect("every regime's member keys have an array")
            .1
    }
}

struct RegimeView {
    /// Diversity classes `(bonus × 10, attribute-count mask)`; one class with
    /// an empty mask when there is no diversity bonus.
    diversity: Vec<(i64, u8)>,
    /// Regime power bound per dense card, read for the admitted cards.
    power: Rc<[u32]>,
    /// Fixed roles in slot order, then the free groups in search order.
    groups: Vec<RegimeGroup>,
    /// `suffix_slack[position][count]`: largest slack `count` groups from
    /// `position` on can add.
    suffix_slack: Vec<[u32; COUNTS]>,
    /// `suffix_support[position][count]`: most support entries `count`
    /// groups from `position` on can displace.
    suffix_support: Vec<[u32; COUNTS]>,
    /// `dynamic_from[position]`: some group from `position` on has a card
    /// whose skill depends on the composition. Otherwise the frontier cannot
    /// improve on the suffix table, which already sums skill maxima.
    dynamic_from: Vec<bool>,
    joint: Option<JointCeiling>,
    /// Number of fixed roles leading `groups`.
    fixed_roles: usize,
    /// Whether the classes hold their joint values and the free groups are
    /// in joint order.
    joined: bool,
}

impl RegimeView {
    /// Sets the terms that depend on the group order.
    fn order_groups(&mut self) {
        // Largest slack and displaced support entries any `r` groups from
        // each position on can add.
        self.suffix_slack = suffix_largest(&self.groups, |class| class.slack_ticks);
        self.suffix_support = suffix_largest(&self.groups, |class| class.support);
        self.dynamic_from = vec![false; self.groups.len() + 1];
        for position in (0..self.groups.len()).rev() {
            self.dynamic_from[position] =
                self.dynamic_from[position + 1] || !self.groups[position].ceilings.is_fixed();
        }
    }
}

/// Joint power-skill ceiling of a regime. For a [`LiveProduct`] the live
/// score is increasing in `u = P + honor` and in the rate `w`, and every
/// completion's suffix satisfies `power * P + skill * S <= joint` besides
/// the separate maxima; the product `u * w` then peaks, over that box cut by
/// the half-plane, at a corner or at the half-plane's tangent point.
#[derive(Clone, Copy, Debug)]
struct JointCeiling {
    product: LiveProduct,
    /// Weights of the joint value, normal to the level curve of the product
    /// at the regime ceiling.
    power: i64,
    skill: i64,
}

impl JointCeiling {
    /// Power weight; the skill weight is rounded to a quarter of it.
    const POWER_WEIGHT: i64 = 4;
    const MAX_SKILL_WEIGHT: i128 = 1 << 12;
    /// Bound on one card's joint value.
    const LIMIT: i64 = 1 << 26;

    /// The ceiling whose half-plane is tangent to the product at a deck of
    /// `power`, `skill` and leader skill `leader`; `None` when the rate does
    /// not read skill.
    fn new(product: LiveProduct, power: u32, skill: u32, leader: u32) -> Option<Self> {
        let u = i128::from(power) + i128::from(product.honor);
        let w = product.rate(skill, leader);
        if product.skill <= 0 || u <= 0 || w <= 0 {
            return None;
        }
        // On `power * slope * u + skill * w = c` the product peaks where the
        // two terms are equal.
        let weight = (i128::from(Self::POWER_WEIGHT) * i128::from(product.skill) * u + w / 2) / w;
        Some(Self {
            product,
            power: Self::POWER_WEIGHT,
            skill: weight.clamp(1, Self::MAX_SKILL_WEIGHT) as i64,
        })
    }

    #[inline]
    fn value(&self, power: u32, skill: u32) -> i64 {
        self.power * i64::from(power) + self.skill * i64::from(skill)
    }

    /// Live-score ceiling of every completion of selected cards with power
    /// `power`, skill at most `skill` and leader skill at most `leader`
    /// whose further members stay within `rest`.
    #[inline]
    fn live(&self, power: u32, skill: u32, leader: u32, rest: &Maxima) -> u32 {
        let slope = i128::from(self.product.skill);
        let base_u = i128::from(power) + i128::from(self.product.honor);
        let base_w = self.product.rate(skill, leader);
        let u = base_u + i128::from(rest.power);
        let w = base_w + slope * i128::from(rest.skill);
        // `power * P + skill * S <= joint` as `alpha * u + beta * w <= c`.
        let alpha = i128::from(self.power) * slope;
        let beta = i128::from(self.skill);
        let c = slope * i128::from(rest.joint) + alpha * base_u + beta * base_w;
        // The peak as `numerator / denominator`.
        let (numerator, denominator) = if alpha * u + beta * w <= c {
            (u * w, 1)
        } else if c >= 2 * alpha * u {
            let Some(numerator) = u.checked_mul(c - alpha * u) else {
                return u32::MAX;
            };
            (numerator, beta)
        } else if c >= 2 * beta * w {
            let Some(numerator) = w.checked_mul(c - beta * w) else {
                return u32::MAX;
            };
            (numerator, alpha)
        } else {
            match c.checked_mul(c) {
                Some(square) => (square, 4 * alpha * beta),
                None => return u32::MAX,
            }
        };
        self.product.live(numerator, denominator)
    }
}

/// Componentwise maxima over suffix selections.
#[derive(Clone, Copy, Debug)]
struct Maxima {
    power: u32,
    skill: u32,
    leader: u32,
    /// Largest joint value; zero without a joint ceiling.
    joint: i32,
}

/// Suffix maxima indexed by position, remaining card count, counting state
/// and shifted key sum in units.
///
/// `capacity` is the limited-bonus capacity still open to the suffix. Mode 0
/// admits a suffix that counts at most `capacity` limited cards and leaves no
/// positive limited card uncounted, or counts exactly `capacity`; mode 1 (the
/// prefix already left a positive limited card uncounted) requires exactly
/// `capacity`.
///
/// A row of `count` remaining cards holds the key sums below
/// `widths[count]`: a card adds at most `widest_units`, and sums past the
/// table width cannot hit a tier. One position's rows form a block; the
/// storage is reused from regime to regime and from scope to scope.
#[derive(Default)]
struct SuffixTable {
    capacities: usize,
    modes: usize,
    widths: [usize; COUNTS],
    /// Offset of each count's rows inside a block.
    starts: [usize; COUNTS],
    block: usize,
    power: Vec<i32>,
    skill: Vec<i16>,
    leader: Vec<i16>,
    /// Largest joint value; kept only for a regime with a joint ceiling.
    joint: Vec<i32>,
    joined: bool,
    /// Raw layers of the counting states while a table is built.
    current: Layer,
    next: Layer,
    sums: RootSums,
}

/// Reachable key sums of five cards at a table's first position, in bit
/// rows of every counting state.
#[derive(Default)]
struct RootSums {
    /// Rows of the last and the current position; every row spans as many
    /// words as the widest one.
    current: Vec<u64>,
    next: Vec<u64>,
    /// The sums of the root query: the whole capacity open and no positive
    /// limited card left uncounted.
    root: Vec<u64>,
}

/// Raw suffix layer: `(count, counted, uncounted)` rows in block layout.
#[derive(Default)]
struct Layer {
    power: Vec<i32>,
    skill: Vec<i16>,
    leader: Vec<i16>,
    joint: Vec<i32>,
}

impl Layer {
    fn clear(&mut self, len: usize, joined: bool) {
        self.power.clear();
        self.power.resize(len, NO_POWER);
        self.skill.clear();
        self.skill.resize(len, NO_SKILL);
        self.leader.clear();
        self.leader.resize(len, NO_SKILL);
        self.joint.clear();
        self.joint.resize(if joined { len } else { 0 }, NO_JOINT);
    }

    fn copy_from(&mut self, other: &Self) {
        self.power.copy_from_slice(&other.power);
        self.skill.copy_from_slice(&other.skill);
        self.leader.copy_from_slice(&other.leader);
        self.joint.copy_from_slice(&other.joint);
    }
}

/// `destination[b] = max(destination[b], source[b - shift] + class)` over
/// one key row, componentwise and only from reachable source entries. The
/// rows may differ in width; sums past the destination row are dropped. An
/// empty joint row is a column the table does not keep.
#[inline]
fn relax(
    destination: (&mut [i32], &mut [i16], &mut [i16], &mut [i32]),
    source: (&[i32], &[i16], &[i16], &[i32]),
    shift: usize,
    class: &Class,
) {
    let (out_power, out_skill, out_leader, out_joint) = destination;
    let (in_power, in_skill, in_leader, in_joint) = source;
    if shift >= out_power.len() {
        return;
    }
    let len = in_power.len().min(out_power.len() - shift);
    let (power, skill, leader) = (class.power as i32, class.skill as i16, class.leader as i16);
    for (out, &value) in out_power[shift..shift + len]
        .iter_mut()
        .zip(&in_power[..len])
    {
        *out = (*out).max(value + power);
    }
    for (out, &value) in out_skill[shift..shift + len]
        .iter_mut()
        .zip(&in_skill[..len])
    {
        *out = (*out).max(value + skill);
    }
    for (out, &value) in out_leader[shift..shift + len]
        .iter_mut()
        .zip(&in_leader[..len])
    {
        // A negative (unreachable) source stays negative.
        *out = (*out).max(value.max(leader | (value >> 15)));
    }
    if !out_joint.is_empty() {
        for (out, &value) in out_joint[shift..shift + len]
            .iter_mut()
            .zip(&in_joint[..len])
        {
            *out = (*out).max(value + class.joint);
        }
    }
}

/// `column[range]`, or the empty row of a column the table does not keep.
#[inline]
fn row_of<T>(column: &[T], range: std::ops::Range<usize>) -> &[T] {
    if column.is_empty() {
        &[]
    } else {
        &column[range]
    }
}

#[inline]
fn row_of_mut<T>(column: &mut [T], range: std::ops::Range<usize>) -> &mut [T] {
    if column.is_empty() {
        &mut []
    } else {
        &mut column[range]
    }
}

fn max_into<T: Copy + Ord>(out: &mut [T], values: &[T]) {
    for (out, &value) in out.iter_mut().zip(values) {
        *out = (*out).max(value);
    }
}

impl SuffixTable {
    /// Adopts the row layout of `problem`, keeping the storage.
    fn fit(&mut self, problem: &Problem<'_>) {
        let width = problem.max_units + 1;
        (self.capacities, self.modes) = match problem.counting {
            Counting::All => (1, 1),
            Counting::FirstN(cap) => (usize::from(cap) + 1, 2),
        };
        let mut block = 0;
        for count in 0..COUNTS {
            self.widths[count] = (count * problem.widest_units + 1).min(width);
            self.starts[count] = block;
            block += self.capacities * self.modes * self.widths[count];
        }
        self.block = block;
    }

    /// Offset of a row inside a block.
    #[inline]
    fn row(&self, count: usize, capacity: usize, mode: usize) -> usize {
        self.starts[count] + (capacity * self.modes + mode) * self.widths[count]
    }

    /// Rebuilds the table for one regime, with the joint column when
    /// `joined` and the regime has a joint ceiling.
    fn build(&mut self, problem: &Problem<'_>, view: &RegimeView, joined: bool) {
        // Every entry of the regime's positions is written below.
        let len = (view.groups.len() + 1) * self.block;
        if self.power.len() < len {
            self.power.resize(len, NO_POWER);
            self.skill.resize(len, NO_SKILL);
            self.leader.resize(len, NO_SKILL);
        }
        self.joined = joined && view.joint.is_some();
        if !self.joined {
            self.joint.clear();
        } else if self.joint.len() < len {
            self.joint.resize(len, NO_JOINT);
        }
        if self.capacities == 1 && self.modes == 1 {
            self.build_in_place(problem, view);
        } else {
            self.build_counted(problem, view);
        }
    }

    /// Every card counts its whole bonus, so the only state is the card
    /// count and each position's block is built from the next one directly.
    fn build_in_place(&mut self, problem: &Problem<'_>, view: &RegimeView) {
        let block = self.block;
        let last = view.groups.len();
        let tail = last * block..(last + 1) * block;
        self.power[tail.clone()].fill(NO_POWER);
        self.skill[tail.clone()].fill(NO_SKILL);
        self.leader[tail.clone()].fill(NO_SKILL);
        row_of_mut(&mut self.joint, tail).fill(NO_JOINT);
        self.power[last * block] = 0;
        self.skill[last * block] = 0;
        self.leader[last * block] = 0;
        if self.joined {
            self.joint[last * block] = 0;
        }
        let unit = i64::from(problem.unit);
        let joined = self.joined;
        let joint_split = |split: usize| if joined { split } else { 0 };
        for (position, group) in view.groups.iter().enumerate().rev() {
            let split = (position + 1) * block;
            let (head_power, tail_power) = self.power.split_at_mut(split);
            let (head_skill, tail_skill) = self.skill.split_at_mut(split);
            let (head_leader, tail_leader) = self.leader.split_at_mut(split);
            let (head_joint, tail_joint) = self.joint.split_at_mut(joint_split(split));
            let current = (
                &mut head_power[position * block..],
                &mut head_skill[position * block..],
                &mut head_leader[position * block..],
                row_of_mut(
                    head_joint,
                    joint_split(position * block)..joint_split(split),
                ),
            );
            let next = (
                &tail_power[..block],
                &tail_skill[..block],
                &tail_leader[..block],
                row_of(tail_joint, 0..block),
            );
            if group.mandatory {
                current.0.fill(NO_POWER);
                current.1.fill(NO_SKILL);
                current.2.fill(NO_SKILL);
                current.3.fill(NO_JOINT);
            } else {
                current.0.copy_from_slice(next.0);
                current.1.copy_from_slice(next.1);
                current.2.copy_from_slice(next.2);
                current.3.copy_from_slice(next.3);
            }
            for class in &group.classes {
                let shift = ((i64::from(class.key_ticks) + problem.offset_ticks) / unit) as usize;
                for count in 1..COUNTS {
                    let target = self.starts[count]..self.starts[count] + self.widths[count];
                    let origin =
                        self.starts[count - 1]..self.starts[count - 1] + self.widths[count - 1];
                    relax(
                        (
                            &mut current.0[target.clone()],
                            &mut current.1[target.clone()],
                            &mut current.2[target.clone()],
                            row_of_mut(&mut *current.3, target),
                        ),
                        (
                            &next.0[origin.clone()],
                            &next.1[origin.clone()],
                            &next.2[origin.clone()],
                            row_of(next.3, origin),
                        ),
                        shift,
                        class,
                    );
                }
            }
        }
    }

    /// Limited bonuses count for the first `cap` cards: raw layers keep the
    /// counted and uncounted states, aggregated into the table per position.
    fn build_counted(&mut self, problem: &Problem<'_>, view: &RegimeView) {
        let mut current = std::mem::take(&mut self.current);
        let mut next = std::mem::take(&mut self.next);
        current.clear(self.block, self.joined);
        next.clear(self.block, self.joined);
        let zero = self.row(0, 0, 0);
        next.power[zero] = 0;
        next.skill[zero] = 0;
        next.leader[zero] = 0;
        if self.joined {
            next.joint[zero] = 0;
        }
        self.aggregate(view.groups.len(), &next);
        let unit = i64::from(problem.unit);
        for (position, group) in view.groups.iter().enumerate().rev() {
            if group.mandatory {
                current.clear(self.block, self.joined);
            } else {
                current.copy_from(&next);
            }
            for class in &group.classes {
                let fixed = ((i64::from(class.key_ticks) + problem.offset_ticks) / unit) as usize;
                let limited = (i64::from(class.limited_ticks) / unit) as usize;
                for count in 1..COUNTS {
                    for counted in 0..self.capacities {
                        for uncounted in 0..self.modes {
                            let target = self.row(count, counted, uncounted);
                            let mut apply = |origin: usize, shift: usize| {
                                let target = target..target + self.widths[count];
                                let origin = origin..origin + self.widths[count - 1];
                                relax(
                                    (
                                        &mut current.power[target.clone()],
                                        &mut current.skill[target.clone()],
                                        &mut current.leader[target.clone()],
                                        row_of_mut(&mut current.joint, target),
                                    ),
                                    (
                                        &next.power[origin.clone()],
                                        &next.skill[origin.clone()],
                                        &next.leader[origin.clone()],
                                        row_of(&next.joint, origin),
                                    ),
                                    shift,
                                    class,
                                );
                            };
                            if limited == 0 {
                                apply(self.row(count - 1, counted, uncounted), fixed);
                                continue;
                            }
                            if counted > 0 {
                                apply(self.row(count - 1, counted - 1, uncounted), fixed + limited);
                            }
                            if uncounted == 1 {
                                for from in 0..self.modes {
                                    apply(self.row(count - 1, counted, from), fixed);
                                }
                            }
                        }
                    }
                }
            }
            self.aggregate(position, &current);
            std::mem::swap(&mut current, &mut next);
        }
        self.current = current;
        self.next = next;
    }

    #[inline]
    fn index(&self, position: usize, count: usize, capacity: usize, mode: usize) -> usize {
        position * self.block + self.row(count, capacity, mode)
    }

    /// Finds the sums that [`Self::best`] reaches at the first position with
    /// five cards and the whole capacity open, without building the table:
    /// bit rows of `(count, counted, uncounted)` take the steps of
    /// [`Self::build_in_place`] and [`Self::build_counted`], so a sum is set
    /// exactly when the table holds its entry, and the root row joins the
    /// states [`Self::aggregate`] joins.
    fn find_root_sums(&mut self, problem: &Problem<'_>, view: &RegimeView) {
        let (capacities, modes) = (self.capacities, self.modes);
        let words = self.widths[DECK_SIZE].div_ceil(64);
        let row = move |count: usize, counted: usize, uncounted: usize| {
            ((count * capacities + counted) * modes + uncounted) * words
        };
        let sums = &mut self.sums;
        let len = row(COUNTS, 0, 0);
        sums.next.clear();
        sums.next.resize(len, 0);
        sums.current.clear();
        sums.current.resize(len, 0);
        sums.next[row(0, 0, 0)] = 1;
        let unit = i64::from(problem.unit);
        for group in view.groups.iter().rev() {
            if group.mandatory {
                sums.current.fill(0);
            } else {
                sums.current.copy_from_slice(&sums.next);
            }
            for class in &group.classes {
                let fixed = ((i64::from(class.key_ticks) + problem.offset_ticks) / unit) as usize;
                let limited = (i64::from(class.limited_ticks) / unit) as usize;
                for count in 1..COUNTS {
                    let width = self.widths[count];
                    for counted in 0..capacities {
                        for uncounted in 0..modes {
                            let target = row(count, counted, uncounted);
                            let mut apply = |origin: usize, shift: usize| {
                                or_shifted(
                                    &mut sums.current[target..target + words],
                                    &sums.next[origin..origin + words],
                                    shift,
                                    width,
                                );
                            };
                            if limited == 0 {
                                apply(row(count - 1, counted, uncounted), fixed);
                                continue;
                            }
                            if counted > 0 {
                                apply(row(count - 1, counted - 1, uncounted), fixed + limited);
                            }
                            if uncounted == 1 {
                                for from in 0..modes {
                                    apply(row(count - 1, counted, from), fixed);
                                }
                            }
                        }
                    }
                }
            }
            std::mem::swap(&mut sums.current, &mut sums.next);
        }
        let capacity = capacities - 1;
        let first = row(DECK_SIZE, capacity, 0);
        sums.root.clear();
        sums.root
            .extend_from_slice(&sums.next[first..first + words]);
        if modes > 1 {
            let others = (0..capacity)
                .map(|counted| row(DECK_SIZE, counted, 0))
                .chain([row(DECK_SIZE, capacity, 1)]);
            for other in others {
                for (out, &bits) in sums.root.iter_mut().zip(&sums.next[other..other + words]) {
                    *out |= bits;
                }
            }
        }
    }

    /// Whether some sum in `low..=high` is a root sum of
    /// [`Self::find_root_sums`], which is when [`Self::best`] finds an entry
    /// there at the root.
    fn reaches_root(&self, low: usize, high: usize) -> bool {
        let high = high.min(self.widths[DECK_SIZE] - 1);
        if low > high {
            return false;
        }
        let (first, last) = (low / 64, high / 64);
        (first..=last).any(|index| {
            let mut bits = self.sums.root[index];
            if index == first {
                bits &= u64::MAX << (low % 64);
            }
            if index == last {
                bits &= u64::MAX >> (63 - high % 64);
            }
            bits != 0
        })
    }

    fn aggregate(&mut self, position: usize, layer: &Layer) {
        let mut sources = Vec::with_capacity(self.capacities + 2);
        for count in 0..COUNTS {
            let width = self.widths[count];
            for capacity in 0..self.capacities {
                for mode in 0..self.modes {
                    let out = self.index(position, count, capacity, mode);
                    sources.clear();
                    if mode == 0 {
                        sources.extend((0..=capacity).map(|counted| self.row(count, counted, 0)));
                        sources.push(self.row(count, capacity, 1));
                    } else {
                        sources.push(self.row(count, capacity, 0));
                        sources.push(self.row(count, capacity, 1));
                    }
                    let (first, rest) = sources.split_first().expect("a state has a source");
                    self.power[out..out + width]
                        .copy_from_slice(&layer.power[*first..*first + width]);
                    self.skill[out..out + width]
                        .copy_from_slice(&layer.skill[*first..*first + width]);
                    self.leader[out..out + width]
                        .copy_from_slice(&layer.leader[*first..*first + width]);
                    row_of_mut(&mut self.joint, out..out + width)
                        .copy_from_slice(row_of(&layer.joint, *first..*first + width));
                    for &source in rest {
                        max_into(
                            &mut self.power[out..out + width],
                            &layer.power[source..source + width],
                        );
                        max_into(
                            &mut self.skill[out..out + width],
                            &layer.skill[source..source + width],
                        );
                        max_into(
                            &mut self.leader[out..out + width],
                            &layer.leader[source..source + width],
                        );
                        max_into(
                            row_of_mut(&mut self.joint, out..out + width),
                            row_of(&layer.joint, source..source + width),
                        );
                    }
                }
            }
        }
    }

    /// Componentwise maxima over the sums `low..=high`, or `None` when every
    /// one of them is unreachable.
    #[inline]
    fn best(
        &self,
        position: usize,
        count: usize,
        capacity: usize,
        mode: usize,
        low: usize,
        high: usize,
    ) -> Option<Maxima> {
        let high = high.min(self.widths[count] - 1);
        if low > high {
            return None;
        }
        let base = self.index(position, count, capacity, mode);
        let mut best: Option<Maxima> = None;
        for sum in low..=high {
            let power = self.power[base + sum];
            if power < 0 {
                continue;
            }
            let found = Maxima {
                power: power as u32,
                skill: self.skill[base + sum] as u32,
                leader: self.leader[base + sum] as u32,
                joint: if self.joined {
                    self.joint[base + sum]
                } else {
                    0
                },
            };
            best = Some(match best {
                None => found,
                Some(old) => Maxima {
                    power: old.power.max(found.power),
                    skill: old.skill.max(found.skill),
                    leader: old.leader.max(found.leader),
                    joint: old.joint.max(found.joint),
                },
            });
        }
        best
    }
}

/// The `(tier, diversity bonus)` pairs no deck of a scope hits, for a scope
/// with one support profile of whole ticks.
///
/// A deck's total in ticks is exactly `Σ e_c + E - X(a, S)`: `e_c` is card
/// `c`'s key plus slack plus its counted limited bonus, `E` the diversity
/// bonus plus the support base, and the refill excess `X` depends only on
/// the count `a` of counted entries the deck removes and the set `S` of
/// positions past them it holds. Only the cards of at most `M` public ids
/// hold such positions; the others enter a reachability table over the card
/// count, the counting state, `a` and the sum, and every subset of the
/// holding cards from distinct groups completes it. The table ignores
/// attributes and mandatory groups and lets a holding card share a group
/// with a table card, which only adds sums.
struct TierCertificate {
    excluded: Vec<(usize, i64)>,
}

impl TierCertificate {
    /// `None` when the positions past the counted entries do not fit the
    /// masks, too many cards hold them, or a refill is not whole ticks.
    fn new(problem: &Problem<'_>, terms: &SupportTerms) -> Option<Self> {
        let extras = problem.extras.as_ref()?;
        if extras.base.0 != extras.base.1 || terms.excess.len() > 65 {
            return None;
        }
        let pool = problem.pool;
        let capacity = usize::from(problem.counting.capacity());
        // (contribution without the limited bonus, limited bonus, counted
        // entries, held positions, group)
        let mut cards = Vec::new();
        for (index, group) in problem.groups.iter().enumerate() {
            for &card in &group.cards {
                let parts = problem.bonus[card.raw()];
                let leader = if group.leader_role {
                    problem.leader_extra_ticks(card)
                } else {
                    0
                };
                let (_, removed, _, held) = terms.entries(pool.game_id(card));
                cards.push((
                    i64::from(parts.fixed)
                        + i64::from(parts.adjust)
                        + i64::from(parts.slack)
                        + i64::from(leader),
                    i64::from(parts.limited),
                    removed as usize,
                    held,
                    index,
                ));
            }
        }
        cards.sort_unstable();
        cards.dedup();
        let lowest = cards.iter().map(|card| card.0).min()?;
        let step = cards
            .iter()
            .fold(0, |step, card| gcd64(gcd64(step, card.0 - lowest), card.1));
        let step = step.max(1);
        let widest = cards.iter().map(|card| card.0 + card.1 - lowest).max()?;
        let mut removals = cards.iter().map(|card| card.2).collect::<Vec<_>>();
        removals.sort_unstable_by(|left, right| right.cmp(left));
        let most = removals.iter().take(DECK_SIZE).sum::<usize>();
        let (holding, plain): (Vec<_>, Vec<_>) = cards.into_iter().partition(|card| card.3 != 0);
        if holding.len() > CERTIFY_HELD_CARDS {
            return None;
        }
        let table = Reachable::new(
            &plain,
            problem.groups.len(),
            lowest,
            step,
            (widest / step) as usize * DECK_SIZE + 1,
            capacity,
            most,
        );
        let scale = problem.scale;
        let diversity = (1..=DECK_SIZE)
            .map(|count| i64::from(extras.diversity[count]) * 10 * scale)
            .collect::<std::collections::BTreeSet<_>>();
        let mut reached = std::collections::BTreeSet::new();
        let mut subsets = vec![(Vec::new(), 0usize)];
        while let Some((chosen, from)) = subsets.pop() {
            for next in from..holding.len() {
                if chosen.len() < DECK_SIZE
                    && chosen
                        .iter()
                        .all(|&index: &usize| holding[index].4 != holding[next].4)
                {
                    let mut wider = chosen.clone();
                    wider.push(next);
                    subsets.push((wider, next + 1));
                }
            }
            let held = chosen
                .iter()
                .fold(0, |held, &index| held | holding[index].3);
            let removed = chosen.iter().map(|&index| holding[index].2).sum::<usize>();
            // Counting choices of the chosen cards: (sum, counted, some
            // limited bonus left uncounted).
            let mut choices = vec![(0i64, 0usize, false)];
            for &index in &chosen {
                let (exact, limited, ..) = holding[index];
                choices = choices
                    .into_iter()
                    .flat_map(|(sum, counted, uncounted)| {
                        let whole = (sum + exact + limited, counted + 1, uncounted);
                        let base = (sum + exact, counted, uncounted || limited > 0);
                        [Some(base), (limited > 0).then_some(whole)]
                    })
                    .flatten()
                    .filter(|choice| choice.1 <= capacity)
                    .collect();
            }
            for (tier, &target) in problem.targets.iter().enumerate() {
                let target_ticks = i64::from(target) * 10 * scale;
                for &extra in &diversity {
                    if reached.contains(&(tier, extra)) {
                        continue;
                    }
                    for rest in 0..=most.saturating_sub(removed) {
                        let (low, high) = ticks(terms.refill(removed + rest, held), scale);
                        if low != high {
                            return None;
                        }
                        let hit = choices.iter().any(|&(sum, counted, uncounted)| {
                            let need = target_ticks - extra - extras.base.0 + low - sum;
                            table.reaches(DECK_SIZE - chosen.len(), rest, counted, uncounted, need)
                        });
                        if hit {
                            reached.insert((tier, extra));
                            break;
                        }
                    }
                }
            }
        }
        let excluded = (0..problem.targets.len())
            .flat_map(|tier| diversity.iter().map(move |&extra| (tier, extra)))
            .filter(|pair| !reached.contains(pair))
            .collect();
        Some(Self { excluded })
    }

    fn excludes(&self, tier: usize, extra_ticks: i64) -> bool {
        self.excluded.contains(&(tier, extra_ticks))
    }
}

/// Sums reachable with one card from each of some groups, by card count,
/// counted entries removed, limited bonuses counted and whether one was
/// left uncounted; a sum is stored as `(Σ e_c - count·lowest) / step`.
struct Reachable {
    lowest: i64,
    step: i64,
    width: usize,
    words: usize,
    capacity: usize,
    most: usize,
    bits: Vec<u64>,
}

impl Reachable {
    fn new(
        cards: &[(i64, i64, usize, u64, usize)],
        groups: usize,
        lowest: i64,
        step: i64,
        width: usize,
        capacity: usize,
        most: usize,
    ) -> Self {
        let words = width.div_ceil(64);
        let mut table = Self {
            lowest,
            step,
            width,
            words,
            capacity,
            most,
            bits: Vec::new(),
        };
        let row =
            |count, removed, counted, uncounted| table.row(count, removed, counted, uncounted);
        let mut bits = vec![0; row(COUNTS, 0, 0, false)];
        bits[0] = 1;
        for group in 0..groups {
            let members = cards.iter().filter(|card| card.4 == group);
            let previous = bits.clone();
            for &(exact, limited, removed, _, _) in members {
                let shift = ((exact - lowest) / step) as usize;
                let extra = (limited / step) as usize;
                for count in 1..COUNTS {
                    for prior in 0..=most.saturating_sub(removed) {
                        for counted in 0..=capacity {
                            for uncounted in [false, true] {
                                let from = row(count - 1, prior, counted, uncounted);
                                let mut apply = |counted, uncounted, shift| {
                                    let to = row(count, prior + removed, counted, uncounted);
                                    or_shifted(
                                        &mut bits[to..to + words],
                                        &previous[from..from + words],
                                        shift,
                                        width,
                                    );
                                };
                                if limited == 0 {
                                    apply(counted, uncounted, shift);
                                    continue;
                                }
                                if counted < capacity {
                                    apply(counted + 1, uncounted, shift + extra);
                                }
                                apply(counted, true, shift);
                            }
                        }
                    }
                }
            }
        }
        table.bits = bits;
        table
    }

    #[inline]
    fn row(&self, count: usize, removed: usize, counted: usize, uncounted: bool) -> usize {
        (((count * (self.most + 1) + removed) * (self.capacity + 1) + counted) * 2
            + usize::from(uncounted))
            * self.words
    }

    /// Whether `count` cards removing `removed` counted entries reach the
    /// sum `need` with a counting state that, together with `counted` and
    /// `uncounted` of the other cards, some slot order realizes: a limited
    /// bonus is left uncounted only once the capacity is used.
    fn reaches(
        &self,
        count: usize,
        removed: usize,
        counted: usize,
        uncounted: bool,
        need: i64,
    ) -> bool {
        let shifted = need - count as i64 * self.lowest;
        if removed > self.most || shifted < 0 || shifted % self.step != 0 {
            return false;
        }
        let index = (shifted / self.step) as usize;
        if index >= self.width {
            return false;
        }
        (0..=self.capacity - counted).any(|own| {
            [false, true].into_iter().any(|left| {
                ((!uncounted && !left) || counted + own == self.capacity)
                    && self.bits[self.row(count, removed, own, left) + index / 64] >> (index % 64)
                        & 1
                        != 0
            })
        })
    }
}

/// `destination |= source << shift`, keeping the first `width` bits.
fn or_shifted(destination: &mut [u64], source: &[u64], shift: usize, width: usize) {
    if shift >= width {
        return;
    }
    let (words, bits) = (shift / 64, shift % 64);
    let used = width.div_ceil(64);
    for index in (words..used).rev() {
        let from = index - words;
        let mut value = source[from] << bits;
        if bits > 0 && from > 0 {
            value |= source[from - 1] >> (64 - bits);
        }
        destination[index] |= value;
    }
    if !width.is_multiple_of(64) {
        destination[used - 1] &= u64::MAX >> (64 - width % 64);
    }
}

/// The completions of one attribute set in a regime: nodes visited with
/// that set at the largest attribute count of a diversity class, and the
/// view and table of the set's cards, built once that count reaches
/// `RESTRICT_AFTER`.
#[derive(Default)]
struct Restriction {
    nodes: Cell<u64>,
    view: OnceCell<Box<(RegimeView, SuffixTable)>>,
}

/// Branch-and-bound state after deciding the groups before `position`.
#[derive(Clone, Copy, Debug)]
struct State {
    position: u8,
    picked: u8,
    /// Positive limited cards counted so far.
    counted: u8,
    /// Some positive limited card so far is not counted.
    uncounted: bool,
    attrs: u8,
    /// Key sum `Σκ` so far, counted limited bonuses included, in ticks.
    key_ticks: i32,
    /// Slack sum `Σσ` so far, in ticks.
    slack_ticks: u32,
    /// Support entries displaced so far.
    support: u32,
    /// Counted support entries removed and positions past them held so far,
    /// with their refill excess in ticks rounded down and up, under a single
    /// support profile.
    removed: u8,
    held: u64,
    refill_low: i32,
    refill_high: i32,
    power: u32,
    /// Skill-maximum sum and largest skill maximum of the selected cards.
    skill: u32,
    leader: u32,
    /// Largest skill maximum of the selected cards whose skill is the same
    /// in every deck.
    fixed_leader: u32,
    /// Units of the selected cards, read by composition-dependent skills.
    composition: Composition,
    /// Selected cards whose skill depends on the composition.
    dynamic: [CardIdx; DECK_SIZE],
    dynamic_len: u8,
}

impl State {
    /// No group decided and no card selected.
    fn root() -> Self {
        Self {
            position: 0,
            picked: 0,
            counted: 0,
            uncounted: false,
            attrs: 0,
            key_ticks: 0,
            slack_ticks: 0,
            support: 0,
            removed: 0,
            held: 0,
            refill_low: 0,
            refill_high: 0,
            power: 0,
            skill: 0,
            leader: 0,
            fixed_leader: 0,
            composition: Composition::default(),
            dynamic: [CardIdx::new(0); DECK_SIZE],
            dynamic_len: 0,
        }
    }
}

/// Search counters when a regime's search began, and the visited-node count
/// of the next test whether it gains the joint column.
#[derive(Clone, Copy, Debug)]
struct Join {
    after: u64,
    bound_prunes: u64,
    feasibility_prunes: u64,
}

#[derive(Clone, Copy, Debug)]
struct Child {
    /// Live-score ceiling of the child's completions.
    upper: u32,
    /// Ceiling from the separate suffix maxima, at least `upper`; siblings
    /// are explored in its order.
    order: u32,
    card: Option<CardIdx>,
    state: State,
}

/// A branch's [`Child::upper`] and [`Child::order`].
#[derive(Clone, Copy, Debug)]
struct Ceiling {
    upper: u32,
    order: u32,
}

struct TierSearch<'s, 'a> {
    problem: &'s Problem<'a>,
    view: &'s RegimeView,
    table: &'s SuffixTable,
    target_ticks: i64,
    /// Diversity bonus of the class, in ticks.
    extra_ticks: i64,
    /// Attribute counts of the class; zero when diversity does not apply.
    counts: u8,
    tier: usize,
    trackers: &'s mut [TopKTracker],
    stats: &'s mut SearchStats,
    budget: &'s mut SearchBudget,
    /// When the regime may still gain the joint column: the search stops
    /// at the next test that finds enough bound prunes since the regime
    /// began, and the caller searches the regime again.
    join: Option<Join>,
    stopped: bool,
    /// The visited-node count at which the search builds the scope's tier
    /// certificate; `None` once it is built or when the scope has none.
    certify_at: Option<u64>,
    /// The certificate shows that no deck of the search hits its tier; the
    /// search ends and its regime is not searched again.
    settled: bool,
    /// Per attribute set, the completions limited to its cards; empty when
    /// no diversity class limits the attributes.
    restrictions: &'s [Restriction],
    /// Slot assignment of the current prefix: roles, then free picks.
    deck: [CardIdx; DECK_SIZE],
    /// One child buffer per position; a path visits each position once.
    scratch: Vec<Vec<Child>>,
}

impl<'s> TierSearch<'s, '_> {
    fn run(&mut self) {
        let root = State::root();
        match self.bound(&root) {
            None => self.stats.feasibility_prunes += 1,
            Some(bound) if bound.upper < self.threshold() => self.stats.ub_prunes += 1,
            Some(_) => self.expand(root),
        }
    }

    /// The tier's K-th live score once K results are held, otherwise zero.
    #[inline]
    fn threshold(&self) -> u32 {
        self.trackers[self.tier]
            .cutoff()
            .map_or(0, |cutoff| cutoff as u32)
    }

    /// Inclusive range of the suffix's shifted key sum (in units) that can
    /// still complete `state` to the tier, or `None` when none can.
    #[inline(always)]
    fn needed(&self, state: &State) -> Option<(usize, usize)> {
        let leader = (state.picked > 0).then(|| self.deck[0]);
        self.problem.needed(
            self.view,
            self.target_ticks,
            self.extra_ticks,
            state,
            leader,
        )
    }

    /// Suffix maxima over every completion of `state` that can hit the tier.
    #[inline(always)]
    fn suffix(&self, state: &State) -> Option<Maxima> {
        let (low, high) = self.needed(state)?;
        let capacity = self.problem.counting.capacity() - state.counted;
        self.table.best(
            usize::from(state.position),
            DECK_SIZE - usize::from(state.picked),
            usize::from(capacity),
            usize::from(state.uncounted && self.table.modes > 1),
            low,
            high,
        )
    }

    /// Skill sum and largest skill of the selected cards in every completion
    /// of `state` with `remaining` further members: the composition-aware
    /// ceilings of Section 20.3 of the pruning proof.
    #[inline]
    fn selected_skill(&self, state: &State, remaining: usize) -> (u32, u32) {
        let (pool, ceilings) = (self.problem.pool, &self.problem.ceilings);
        state.dynamic[..usize::from(state.dynamic_len)].iter().fold(
            (state.skill, state.fixed_leader),
            |(sum, largest), &card| {
                let value = ceilings[card.raw()].selected(&state.composition, remaining);
                (
                    sum - u32::from(pool.skill_max(card)) + value,
                    largest.max(value),
                )
            },
        )
    }

    /// Skill sum and largest skill of the `picks` further cards of every
    /// completion that takes them from the groups at `position` on, among
    /// `free` unselected members around `composition`: at most one card per
    /// group, each within its group's largest candidate ceiling.
    #[inline]
    fn frontier(
        &self,
        position: usize,
        composition: &Composition,
        free: usize,
        picks: usize,
    ) -> (u32, u32) {
        if picks == 0 {
            return (0, 0);
        }
        if !self.view.dynamic_from[position] {
            return (u32::MAX, u32::MAX);
        }
        let mut top = [0u32; DECK_SIZE];
        for group in &self.view.groups[position..] {
            let mut value = group.ceilings.candidate(composition, free);
            for slot in &mut top[..picks] {
                if value > *slot {
                    std::mem::swap(&mut value, slot);
                }
            }
        }
        (top[..picks].iter().sum(), top[0])
    }

    /// The suffix skill terms of `state`'s completions, capped by the
    /// frontier of the groups left.
    #[inline]
    fn suffix_skill(&self, state: &State, skill: u32, leader: u32) -> (u32, u32) {
        let remaining = DECK_SIZE - usize::from(state.picked);
        let (sum, largest) = self.frontier(
            usize::from(state.position),
            &state.composition,
            remaining,
            remaining,
        );
        (skill.min(sum), leader.min(largest))
    }

    /// Live-score ceiling of every completion of selected cards with power
    /// `power`, skill at most `skill` and leader skill at most `leader`
    /// whose further members stay within `rest`, from the separate maxima
    /// and, when the regime has one, the joint ceiling: `(order, upper)`.
    #[inline]
    fn ceiling(&self, power: u32, skill: u32, leader: u32, rest: &Maxima) -> Ceiling {
        let order = self.problem.live_upper(
            power + rest.power,
            skill + rest.skill,
            leader.max(rest.leader),
        );
        let upper = match &self.view.joint {
            Some(joint) if self.table.joined => {
                order.min(joint.live(power, skill, leader.max(rest.leader), rest))
            }
            _ => order,
        };
        Ceiling { upper, order }
    }

    /// Live-score ceilings of every completion of `state` that can hit the
    /// tier. The composition-aware skill terms are computed only when the
    /// ceiling from skill maxima does not already fall below the cutoff.
    #[inline]
    fn bound(&self, state: &State) -> Option<Ceiling> {
        let rest = self.suffix(state)?;
        let coarse = self.problem.live_upper(
            state.power + rest.power,
            state.skill + rest.skill,
            state.leader.max(rest.leader),
        );
        if coarse < self.threshold() {
            return Some(Ceiling {
                upper: coarse,
                order: coarse,
            });
        }
        let (skill, leader) = self.suffix_skill(state, rest.skill, rest.leader);
        let remaining = DECK_SIZE - usize::from(state.picked);
        let (selected, largest) = self.selected_skill(state, remaining);
        Some(self.ceiling(
            state.power,
            selected,
            largest,
            &Maxima {
                skill,
                leader,
                ..rest
            },
        ))
    }

    /// Some attribute count of the diversity class stays reachable.
    #[inline]
    fn attrs_feasible(&self, state: &State) -> bool {
        if self.counts == 0 {
            return true;
        }
        let known = state.attrs.count_ones() as usize;
        let remaining = DECK_SIZE - usize::from(state.picked);
        let (fewest, most) = (known.max(1), (known + remaining).min(DECK_SIZE));
        let reachable = ((1u16 << (most + 1)) - (1u16 << fewest)) as u8;
        self.counts & reachable != 0
    }

    fn expand(&mut self, state: State) {
        if self.budget.expired_sampled() || self.stopped || self.settled {
            return;
        }
        if let Some(at) = self.certify_at
            && self.stats.visited_nodes >= at
        {
            self.certify_at = None;
            if self
                .problem
                .certificate()
                .is_some_and(|certificate| certificate.excludes(self.tier, self.extra_ticks))
            {
                self.settled = true;
                return;
            }
        }
        if let Some(join) = &mut self.join
            && self.stats.visited_nodes >= join.after
        {
            if JOIN_RATIO * (self.stats.ub_prunes - join.bound_prunes)
                >= self.stats.feasibility_prunes - join.feasibility_prunes
            {
                self.stopped = true;
                return;
            }
            join.after += JOIN_AFTER;
        }
        if let Some((view, table)) = self.restricted(&state) {
            self.expand_within(view, table, state);
            return;
        }
        self.stats.visited_nodes += 1;
        let position = usize::from(state.position);
        let mut children = std::mem::take(&mut self.scratch[position]);
        children.clear();
        self.collect_children(&state, &mut children);
        children.sort_unstable_by_key(|child| std::cmp::Reverse(child.order));
        for child in &children {
            if self.budget.hit {
                break;
            }
            let threshold = self.threshold();
            if child.order < threshold {
                self.stats.ub_prunes += 1;
                break;
            }
            if child.upper < threshold {
                self.stats.ub_prunes += 1;
                continue;
            }
            if let Some(card) = child.card {
                self.deck[usize::from(state.picked)] = card;
            }
            if usize::from(child.state.picked) == DECK_SIZE {
                self.leaf();
            } else {
                self.expand(child.state);
            }
        }
        self.scratch[position] = children;
    }

    /// The view and table of the attributes `state` holds, once the class
    /// allows no further attribute and the search has visited enough nodes
    /// with them.
    fn restricted(&self, state: &State) -> Option<&'s (RegimeView, SuffixTable)> {
        if self.most() != Some(state.attrs.count_ones()) {
            return None;
        }
        let restrictions: &'s [Restriction] = self.restrictions;
        let restriction = restrictions.get(usize::from(state.attrs))?;
        if restriction.view.get().is_none() {
            let nodes = restriction.nodes.get() + 1;
            restriction.nodes.set(nodes);
            if nodes < self.problem.restrict_after {
                return None;
            }
        }
        let built = restriction
            .view
            .get_or_init(|| Box::new(self.problem.restrict(self.view, state.attrs)));
        Some(&**built)
    }

    /// Largest attribute count of the class when the search may still
    /// narrow its completions to the attributes they hold.
    #[inline]
    fn most(&self) -> Option<u32> {
        (!self.restrictions.is_empty() && self.counts != 0)
            .then(|| u8::BITS - 1 - self.counts.leading_zeros())
    }

    /// Searches the completions of `state` in `view`, the view of the
    /// attributes it holds.
    fn expand_within(&mut self, view: &RegimeView, table: &SuffixTable, state: State) {
        let mut within = TierSearch {
            problem: self.problem,
            view,
            table,
            target_ticks: self.target_ticks,
            extra_ticks: self.extra_ticks,
            counts: self.counts,
            tier: self.tier,
            trackers: &mut *self.trackers,
            stats: &mut *self.stats,
            budget: &mut *self.budget,
            join: self.join,
            stopped: false,
            certify_at: self.certify_at,
            settled: false,
            restrictions: &[],
            deck: self.deck,
            scratch: std::mem::take(&mut self.scratch),
        };
        within.expand(state);
        self.join = within.join;
        self.stopped = within.stopped;
        self.certify_at = within.certify_at;
        self.settled = within.settled;
        self.scratch = within.scratch;
    }

    fn collect_children(&mut self, state: &State, children: &mut Vec<Child>) {
        let view = self.view;
        let problem = self.problem;
        let pool = problem.pool;
        let group = &view.groups[usize::from(state.position)];
        let threshold = self.threshold();
        let capacity = problem.counting.capacity();
        let slot = usize::from(state.picked);
        // Public ids and characters of the selected cards, in slot order.
        let mut taken_ids = [0u16; DECK_SIZE];
        let mut taken_characters = [0u8; DECK_SIZE];
        for ((id, character), &card) in taken_ids
            .iter_mut()
            .zip(&mut taken_characters)
            .zip(&self.deck[..slot])
        {
            *id = pool.game_id(card);
            *character = pool.char_id(card);
        }
        // The Final leader fixes the support profile, so its need depends on
        // the concrete card rather than on its class alone.
        let per_card = group.leader_role
            && problem
                .extras
                .as_ref()
                .is_some_and(|extras| extras.by_leader.is_some());
        // At the class's largest attribute count only the attributes held so
        // far remain.
        let full = self.most() == Some(state.attrs.count_ones());
        for class in &group.classes {
            if full && class.attrs & state.attrs == 0 {
                self.stats.feasibility_prunes += 1;
                continue;
            }
            // Counting choices of a class: the limited bonus of a fixed role
            // counts exactly when capacity remains; a free card may count it
            // or leave it uncounted.
            let (removed, held) = (
                state.removed.saturating_add(class.removed),
                state.held | class.held,
            );
            let (refill_low, refill_high) = if class.removed == 0 && class.held == 0 {
                (state.refill_low, state.refill_high)
            } else {
                problem.refill_ticks(removed, held)
            };
            let mut options = [None; 2];
            let key = class.key_ticks;
            let limited = class.limited_ticks as i32;
            if class.limited_ticks == 0 {
                options[0] = Some((key, 0u8, false));
            } else if group.fixed_role {
                options[0] = Some(if state.counted < capacity {
                    (key + limited, 1, false)
                } else {
                    (key, 0, true)
                });
            } else {
                if state.counted < capacity {
                    options[0] = Some((key + limited, 1, false));
                }
                options[1] = Some((key, 0, true));
            }
            for (key_ticks, counted, uncounted) in options.into_iter().flatten() {
                let next = State {
                    position: state.position + 1,
                    picked: state.picked + 1,
                    counted: state.counted + counted,
                    uncounted: state.uncounted || uncounted,
                    key_ticks: state.key_ticks + key_ticks,
                    slack_ticks: state.slack_ticks + class.slack_ticks,
                    support: state.support + class.support,
                    removed,
                    held,
                    refill_low,
                    refill_high,
                    ..*state
                };
                let shared = if per_card {
                    None
                } else {
                    let Some(suffix) = self.suffix(&next) else {
                        self.stats.feasibility_prunes += 1;
                        continue;
                    };
                    // The class maxima bound every card of the class, which
                    // is still one of the unknown members here.
                    let coarse = problem.live_upper(
                        state.power + class.power + suffix.power,
                        state.skill + class.skill + suffix.skill,
                        state.leader.max(class.leader).max(suffix.leader),
                    );
                    if coarse < threshold {
                        self.stats.ub_prunes += 1;
                        continue;
                    }
                    let remaining = DECK_SIZE - slot;
                    let (selected, largest) = self.selected_skill(state, remaining);
                    let (rest, rest_largest) = self.frontier(
                        usize::from(next.position),
                        &state.composition,
                        remaining,
                        remaining - 1,
                    );
                    let bound = self.ceiling(
                        state.power,
                        selected,
                        largest,
                        &Maxima {
                            power: class.power + suffix.power,
                            skill: class.skill + suffix.skill.min(rest),
                            leader: class.leader.max(suffix.leader.min(rest_largest)),
                            joint: class.joint + suffix.joint,
                        },
                    );
                    if bound.upper < threshold {
                        self.stats.ub_prunes += 1;
                        continue;
                    }
                    Some(suffix)
                };
                // The frontier after a card depends on the card only through
                // its unit mask and its reference value.
                let mut frontiers = [((0u8, 0u16), (0u32, 0u32)); 8];
                let mut cached = 0usize;
                for &card in &group.cards[class.start as usize..class.end as usize] {
                    let character = pool.char_id(card);
                    if (problem.unique_characters && taken_characters[..slot].contains(&character))
                        || taken_ids[..slot].contains(&pool.game_id(card))
                    {
                        self.stats.feasibility_prunes += 1;
                        continue;
                    }
                    let skill = u32::from(pool.skill_max(card));
                    let mut child = State {
                        attrs: next.attrs | (1 << pool.attr(card)),
                        power: next.power + view.power[card.raw()],
                        skill: next.skill + skill,
                        leader: next.leader.max(skill),
                        composition: next.composition.with(pool, card),
                        ..next
                    };
                    if problem.ceilings[card.raw()].is_fixed() {
                        child.fixed_leader = child.fixed_leader.max(skill);
                    } else {
                        child.dynamic[usize::from(child.dynamic_len)] = card;
                        child.dynamic_len += 1;
                    }
                    if !self.attrs_feasible(&child) {
                        self.stats.feasibility_prunes += 1;
                        continue;
                    }
                    let bound = match shared {
                        Some(suffix) => {
                            let coarse = problem.live_upper(
                                child.power + suffix.power,
                                child.skill + suffix.skill,
                                child.leader.max(suffix.leader),
                            );
                            if coarse < threshold {
                                self.stats.ub_prunes += 1;
                                continue;
                            }
                            let key = (pool.unit_mask_raw(card), pool.skill_reference(card));
                            let (skill, leader) =
                                match frontiers[..cached].iter().find(|entry| entry.0 == key) {
                                    Some(entry) => entry.1,
                                    None => {
                                        let value =
                                            self.suffix_skill(&child, suffix.skill, suffix.leader);
                                        if cached < frontiers.len() {
                                            frontiers[cached] = (key, value);
                                            cached += 1;
                                        }
                                        value
                                    }
                                };
                            let remaining = DECK_SIZE - usize::from(child.picked);
                            let (selected, largest) = self.selected_skill(&child, remaining);
                            self.ceiling(
                                child.power,
                                selected,
                                largest,
                                &Maxima {
                                    skill,
                                    leader,
                                    ..suffix
                                },
                            )
                        }
                        None => {
                            self.deck[slot] = card;
                            let Some(bound) = self.bound(&child) else {
                                self.stats.feasibility_prunes += 1;
                                continue;
                            };
                            bound
                        }
                    };
                    if bound.upper < threshold {
                        self.stats.ub_prunes += 1;
                        continue;
                    }
                    children.push(Child {
                        upper: bound.upper,
                        order: bound.order,
                        card: Some(card),
                        state: child,
                    });
                }
            }
        }
        if !group.mandatory {
            let skip = State {
                position: state.position + 1,
                ..*state
            };
            match self.bound(&skip) {
                None => self.stats.feasibility_prunes += 1,
                Some(bound) if bound.upper < threshold => self.stats.ub_prunes += 1,
                Some(bound) => children.push(Child {
                    upper: bound.upper,
                    order: bound.order,
                    card: None,
                    state: skip,
                }),
            }
        }
    }

    fn leaf(&mut self) {
        self.stats.leaf_nodes += 1;
        let problem = self.problem;
        let (pool, ctx) = (problem.pool, problem.ctx);
        let trackers = &mut *self.trackers;
        visit_bonus_candidates(pool, ctx, &self.deck, |candidate| {
            let total = resolve_total_bonus(pool, ctx, &candidate.cards);
            if let Some(tier) = problem
                .targets
                .iter()
                .position(|&target| total == f64::from(target))
            {
                trackers[tier].insert(pool, ctx, candidate);
            }
        });
    }
}

/// Characters by `u8` id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CharacterSet([u64; 4]);

impl CharacterSet {
    #[inline]
    fn contains(self, character: u8) -> bool {
        self.0[usize::from(character >> 6)] & (1 << (character & 63)) != 0
    }

    #[inline]
    fn with(mut self, character: u8) -> Self {
        self.0[usize::from(character >> 6)] |= 1 << (character & 63);
        self
    }
}

/// `result[position][count]`: the largest sum of `field` over `count` groups
/// from `position` on, each group contributing its largest class value.
fn suffix_largest(groups: &[RegimeGroup], field: fn(&Class) -> u32) -> Vec<[u32; COUNTS]> {
    let mut result = vec![[0u32; COUNTS]; groups.len() + 1];
    let mut largest = [0u32; DECK_SIZE];
    for (position, group) in groups.iter().enumerate().rev() {
        let mut value = group.classes.iter().map(field).max().unwrap_or(0);
        for slot in &mut largest {
            if value > *slot {
                std::mem::swap(&mut value, slot);
            }
        }
        for count in 1..COUNTS {
            result[position][count] = result[position][count - 1] + largest[count - 1];
        }
    }
    result
}

fn gcd64(mut left: i64, mut right: i64) -> i64 {
    (left, right) = (left.abs(), right.abs());
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The support bonus as the evaluator defines it: the first `count`
    /// entries whose public id is not in the main deck.
    fn support(deck: &SupportDeck, main: &[u16]) -> f64 {
        deck.cards
            .iter()
            .filter(|(game_id, _)| !main.contains(game_id))
            .take(usize::from(deck.count))
            .map(|(_, bonus)| bonus)
            .sum()
    }

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % u64::from(bound)) as u32
        }
    }

    #[test]
    fn support_loss_is_the_card_losses_plus_a_bounded_excess() {
        let mut rng = Lcg(0x5EED_2026);
        let mut checked = 0u64;
        for _ in 0..300 {
            let ids = 3 + rng.below(9) as u16;
            let len = rng.below(13) as usize;
            // Repeated public ids and non-binary fractions are both covered.
            let mut cards = (0..len)
                .map(|_| {
                    (
                        rng.below(u32::from(ids)) as u16,
                        f64::from(rng.below(40)) * 0.35,
                    )
                })
                .collect::<Vec<_>>();
            cards.sort_by(|left, right| right.1.total_cmp(&left.1));
            let deck = SupportDeck {
                cards,
                count: rng.below(7) as u8,
            };
            let terms = SupportTerms::new(&deck).expect("sorted non-negative profile");
            for mask in 0u32..(1 << ids) {
                if mask.count_ones() as usize > DECK_SIZE {
                    continue;
                }
                let main = (0..ids)
                    .filter(|id| mask & (1 << id) != 0)
                    .collect::<Vec<_>>();
                let loss = terms.base - support(&deck, &main);
                let (cards, displaced) = main.iter().fold((0.0, 0usize), |sum, &id| {
                    let (loss, displaced) = terms.loss_of(id);
                    (sum.0 + loss, sum.1 + displaced as usize)
                });
                let excess = loss - cards;
                let bound = terms.excess_of(displaced);
                let (removed, held) = main.iter().fold((0usize, 0u64), |sum, &id| {
                    let (_, removed, _, held) = terms.entries(id);
                    (sum.0 + removed as usize, sum.1 | held)
                });
                if terms.excess.len() <= 65 {
                    let refill = terms.refill(removed, held);
                    assert!(
                        (excess - refill).abs() < 1e-9,
                        "deck={deck:?} main={main:?} excess={excess} refill={refill}"
                    );
                }
                assert!(
                    displaced < terms.excess.len() && -1e-9 <= excess && excess <= bound + 1e-9,
                    "deck={deck:?} main={main:?} loss={loss} cards={cards} excess bound={bound}"
                );
                if displaced <= 1 {
                    assert!(excess.abs() < 1e-9, "deck={deck:?} main={main:?}");
                }
                checked += 1;
            }
        }
        assert!(checked > 50_000, "checked {checked} decks");
    }

    #[test]
    fn the_refill_of_cards_selected_first_bounds_the_deck_refill() {
        let mut rng = Lcg(0x5EED_2027);
        let mut checked = 0u64;
        for _ in 0..120 {
            let ids = 3 + rng.below(9) as u16;
            let len = rng.below(13) as usize;
            let mut cards = (0..len)
                .map(|_| {
                    (
                        rng.below(u32::from(ids)) as u16,
                        f64::from(rng.below(40)) * 0.35,
                    )
                })
                .collect::<Vec<_>>();
            cards.sort_by(|left, right| right.1.total_cmp(&left.1));
            let deck = SupportDeck {
                cards,
                count: rng.below(7) as u8,
            };
            let terms = SupportTerms::new(&deck).expect("sorted non-negative profile");
            let entries = |mask: u32| {
                (0..ids)
                    .filter(|id| mask & (1 << id) != 0)
                    .fold((0usize, 0u64), |sum, id| {
                        let (_, removed, _, held) = terms.entries(id);
                        (sum.0 + removed as usize, sum.1 | held)
                    })
            };
            for mask in 0u32..(1 << ids) {
                if mask.count_ones() as usize > DECK_SIZE {
                    continue;
                }
                let main = (0..ids)
                    .filter(|id| mask & (1 << id) != 0)
                    .collect::<Vec<_>>();
                let cards = main.iter().map(|&id| terms.loss_of(id).0).sum::<f64>();
                let excess = terms.base - support(&deck, &main) - cards;
                // Every subset of a deck's cards is selected first in some
                // slot order.
                let mut prefix = mask;
                loop {
                    let (removed, held) = entries(prefix);
                    let floor = terms.refill(removed, held);
                    assert!(
                        floor <= excess + 1e-9,
                        "deck={deck:?} main={main:?} prefix={prefix:b} floor={floor} excess={excess}"
                    );
                    checked += 1;
                    if prefix == 0 {
                        break;
                    }
                    prefix = (prefix - 1) & mask;
                }
            }
        }
        assert!(checked > 100_000, "checked {checked} prefixes");
    }

    #[test]
    fn unordered_or_negative_support_disables_the_fold() {
        let unordered = SupportDeck {
            cards: vec![(1, 1.0), (2, 2.0)],
            count: 1,
        };
        let negative = SupportDeck {
            cards: vec![(1, 1.0), (2, -0.5)],
            count: 2,
        };
        let infinite = SupportDeck {
            cards: vec![(1, f64::INFINITY)],
            count: 1,
        };
        for deck in [unordered, negative, infinite] {
            assert!(SupportTerms::new(&deck).is_none(), "{deck:?}");
        }
    }

    /// The joint ceiling bounds the product of every suffix selection within
    /// the box and the half-plane, and never exceeds the box corner.
    #[test]
    fn joint_ceiling_dominates_every_selection() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |bound: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        let mut checked = 0usize;
        for round in 0..300 {
            let product = LiveProduct {
                honor: next(5_000) as i64,
                intercept: 1_000_000 + next(8_000_000) as i64,
                leader: next(200_000) as i64,
                skill: 1 + next(50_000) as i64,
                constant: if round % 3 == 0 {
                    next(100_000_000_000) as i64
                } else {
                    0
                },
                divisor: if round % 2 == 0 { 1 } else { 500 },
                floor: if round % 4 == 0 {
                    next(1_000) as i64
                } else {
                    0
                },
            };
            let Some(joint) = JointCeiling::new(
                product,
                next(300_000) as u32,
                next(800) as u32,
                next(160) as u32,
            ) else {
                continue;
            };
            let selections = (0..1 + next(40))
                .map(|_| (next(250_000) as u32, next(750) as u32))
                .collect::<Vec<_>>();
            let cap = if round % 5 == 0 {
                next(750) as u32
            } else {
                u32::MAX
            };
            let rest = Maxima {
                power: selections.iter().map(|&(power, _)| power).max().unwrap(),
                skill: selections
                    .iter()
                    .map(|&(_, skill)| skill)
                    .max()
                    .unwrap()
                    .min(cap),
                leader: next(160) as u32,
                joint: selections
                    .iter()
                    .map(|&(power, skill)| joint.value(power, skill) as i32)
                    .max()
                    .unwrap(),
            };
            for _ in 0..20 {
                let power = next(100_000) as u32;
                let skill = next(300) as u32;
                let leader = rest.leader.max(next(160) as u32);
                let bound = joint.live(power, skill, leader, &rest);
                let base_u = i128::from(power) + i128::from(product.honor);
                let base_w = product.rate(skill, leader);
                let slope = i128::from(product.skill);
                let corner = product.live(
                    (base_u + i128::from(rest.power)) * (base_w + slope * i128::from(rest.skill)),
                    1,
                );
                assert!(bound <= corner, "round={round}");
                for &(extra_power, extra_skill) in &selections {
                    if extra_skill > rest.skill {
                        continue;
                    }
                    let exact = product.live(
                        (base_u + i128::from(extra_power))
                            * (base_w + slope * i128::from(extra_skill)),
                        1,
                    );
                    assert!(
                        bound >= exact,
                        "round={round} power={power} skill={skill} leader={leader}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 50_000, "{checked}");
    }
}
