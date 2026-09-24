//! Ceilings of composition-dependent skills: the largest value a card's skill
//! can resolve to in any deck around a given selection.
use crate::pool::{CardIdx, CardPool, DiffSkill};
use crate::search::evaluate;
use crate::types::{DECK_SIZE, SkillReferenceStrategy};

/// Unit bits a card's unit mask can carry.
const UNIT_BITS: usize = 6;

/// The deck-wide facts that composition-dependent skills read, over the
/// selected cards.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Composition {
    /// Selected cards carrying each unit bit.
    unit_counts: [u8; UNIT_BITS],
    /// Union of the selected cards' different-unit units
    /// ([`evaluate::member_unit`]).
    member_units: u8,
    /// Static skill maxima that reference skills read
    /// ([`CardPool::skill_reference`]), of the selected cards.
    references: [u16; DECK_SIZE],
    selected: u8,
}

impl Composition {
    #[inline(always)]
    pub(crate) fn with(mut self, pool: &CardPool, card: CardIdx) -> Self {
        let mask = pool.unit_mask_raw(card);
        for (unit, count) in self.unit_counts.iter_mut().enumerate() {
            *count += (mask >> unit) & 1;
        }
        self.member_units |= evaluate::member_unit(mask);
        self.references[usize::from(self.selected)] = pool.skill_reference(card);
        self.selected += 1;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CeilingKind {
    /// Deck-independent: `steps[0]`.
    Fixed,
    /// Unit-count skill: `steps[k - 1]` bounds every deck in which at most
    /// `k` members carry the counted unit.
    UnitCount,
    /// Different-unit skill: `steps[d]` bounds every deck in which the card
    /// counts at most `d` units.
    DifferentUnit,
    /// Reference skill, bounded from the other members' static maxima and
    /// `steps[0]`.
    Reference(ReferenceRule),
}

/// A reference skill resolves to `base` plus the share its strategy takes
/// from `min(reference * rate / 100, cap)` over the other four members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReferenceRule {
    base: u8,
    rate: u8,
    cap: u8,
    /// The card's own reference value, which its selected ceiling skips once.
    own: u16,
    strategy: SkillReferenceStrategy,
}

impl ReferenceRule {
    /// Ceiling of the card's value when `composition` holds the card itself
    /// (`held`) or not, with `unknown` further members. Shares are integers
    /// in hundredths of a percent. The largest and the smallest share round
    /// up exactly, and so does the evaluator's rounded share; the mean takes
    /// one more than the floor of the exact mean, which also stays above the
    /// evaluator's floating-point mean.
    #[inline(always)]
    fn bound(&self, composition: &Composition, held: bool, unknown: usize) -> u32 {
        let cap = 100 * u32::from(self.cap);
        let mut skip = held;
        let (mut sum, mut high, mut low) = (0u32, 0u32, u32::MAX);
        for &reference in &composition.references[..usize::from(composition.selected)] {
            if skip && reference == self.own {
                skip = false;
                continue;
            }
            let share = (u32::from(reference) * u32::from(self.rate)).min(cap);
            sum += share;
            high = high.max(share);
            low = low.min(share);
        }
        if unknown > 0 {
            sum += unknown as u32 * cap;
            high = high.max(cap);
            low = low.min(cap);
        }
        let added = match self.strategy {
            SkillReferenceStrategy::Max => high.div_ceil(100),
            SkillReferenceStrategy::Min => low.div_ceil(100),
            SkillReferenceStrategy::Average => sum / (100 * (DECK_SIZE as u32 - 1)) + 1,
        };
        u32::from(self.base) + added
    }
}

/// Ceiling of one card's resolved skill as a function of the composition of
/// the deck around it. No step exceeds the card's `skill_max`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SkillCeiling {
    kind: CeilingKind,
    /// Unit bit index a unit-count skill counts, or the card's own member
    /// unit bit for a different-unit skill.
    unit: u8,
    /// Whether the card itself carries the unit its unit-count skill counts.
    carries: u8,
    steps: [u8; DECK_SIZE],
}

impl SkillCeiling {
    /// Reads the card's skill exactly as `evaluate::resolve_skills` does. A
    /// skill that resolves to a deck-independent value keeps `skill_max`.
    pub(crate) fn new(pool: &CardPool, card: CardIdx, strategy: SkillReferenceStrategy) -> Self {
        let max = pool.skill_max(card);
        let slot = pool.skill(card);
        let index = usize::from(slot.value.saturating_sub(1));
        let fixed = Self {
            kind: CeilingKind::Fixed,
            unit: 0,
            carries: 0,
            steps: [max; DECK_SIZE],
        };
        match slot.skill_type {
            1 => {
                let Some(entry) = pool.special().unit_count().get(index) else {
                    return fixed;
                };
                if usize::from(entry.unit) >= UNIT_BITS {
                    return fixed;
                }
                // Tables need not be monotone: step `k - 1` is the largest
                // entry for one through `k` members.
                let mut steps = [0; DECK_SIZE];
                let mut best = 0;
                for (step, &score_up) in steps.iter_mut().zip(&entry.score_up) {
                    best = best.max(score_up);
                    *step = best.min(max);
                }
                Self {
                    kind: CeilingKind::UnitCount,
                    unit: entry.unit,
                    carries: (pool.unit_mask_raw(card) >> entry.unit) & 1,
                    steps,
                }
            }
            2 => {
                let Some(entry) = pool.special().diff().get(index) else {
                    return fixed;
                };
                let mut steps = [max; DECK_SIZE];
                for (counted, step) in steps
                    .iter_mut()
                    .enumerate()
                    .take(usize::from(DiffSkill::MAX_COUNTED_UNITS) + 1)
                {
                    let value = u32::from(entry.base) + u32::from(entry.increment) * counted as u32;
                    *step = value.min(u32::from(max)) as u8;
                }
                Self {
                    kind: CeilingKind::DifferentUnit,
                    unit: evaluate::member_unit(pool.unit_mask_raw(card)),
                    carries: 0,
                    steps,
                }
            }
            3 => {
                let Some(entry) = pool.special().ref_skills().get(index) else {
                    return fixed;
                };
                if entry.rate == 0 || entry.max == 0 {
                    return fixed;
                }
                Self {
                    kind: CeilingKind::Reference(ReferenceRule {
                        base: pool.skill_min(card),
                        rate: entry.rate,
                        cap: entry.max,
                        own: pool.skill_reference(card),
                        strategy,
                    }),
                    ..fixed
                }
            }
            _ => fixed,
        }
    }

    /// Whether the ceiling is `skill_max` in every deck.
    #[inline(always)]
    pub(crate) fn is_fixed(&self) -> bool {
        matches!(self.kind, CeilingKind::Fixed)
    }

    /// Ceiling in every deck made of the cards counted in `composition`, the
    /// card itself (`held` when `composition` counts it already), and
    /// `unknown` further members.
    #[inline(always)]
    fn bound(&self, composition: &Composition, held: bool, unknown: usize) -> u32 {
        let step = match self.kind {
            CeilingKind::Fixed => 0,
            CeilingKind::UnitCount => {
                let own = if held { 0 } else { self.carries };
                let known = composition.unit_counts[usize::from(self.unit)] + own;
                (usize::from(known) + unknown).clamp(1, DECK_SIZE) - 1
            }
            CeilingKind::DifferentUnit => {
                let counted = (composition.member_units & !self.unit).count_ones() as usize;
                (counted + unknown).min(usize::from(DiffSkill::MAX_COUNTED_UNITS))
            }
            CeilingKind::Reference(rule) => {
                return rule
                    .bound(composition, held, unknown)
                    .min(u32::from(self.steps[0]));
            }
        };
        u32::from(self.steps[step])
    }

    /// Ceiling of a selected card, held in `composition`, with `free` slots
    /// left.
    #[inline(always)]
    pub(crate) fn selected(&self, composition: &Composition, free: usize) -> u32 {
        self.bound(composition, true, free)
    }

    /// Ceiling of an unselected card taken as one of the `free` remaining
    /// picks.
    #[inline(always)]
    pub(crate) fn candidate(&self, composition: &Composition, free: usize) -> u32 {
        self.bound(composition, false, free - 1)
    }
}

/// The largest candidate ceiling over a set of cards: the ceilings that read
/// the same composition facts share one entry with their stepwise maxima.
#[derive(Clone, Debug, Default)]
pub(crate) struct CeilingSet {
    entries: Vec<SkillCeiling>,
}

impl CeilingSet {
    pub(crate) fn insert(&mut self, ceiling: SkillCeiling) {
        let same = |entry: &&mut SkillCeiling| match (entry.kind, ceiling.kind) {
            (CeilingKind::Reference(left), CeilingKind::Reference(right)) => {
                left.strategy == right.strategy
            }
            (left, right) => {
                left == right && entry.unit == ceiling.unit && entry.carries == ceiling.carries
            }
        };
        match self.entries.iter_mut().find(same) {
            Some(entry) => {
                for (step, &other) in entry.steps.iter_mut().zip(&ceiling.steps) {
                    *step = (*step).max(other);
                }
                // A candidate reference ceiling is non-decreasing in the
                // base, the rate and the cap, and never reads `own`.
                if let (CeilingKind::Reference(left), CeilingKind::Reference(right)) =
                    (&mut entry.kind, ceiling.kind)
                {
                    left.base = left.base.max(right.base);
                    left.rate = left.rate.max(right.rate);
                    left.cap = left.cap.max(right.cap);
                }
            }
            None => self.entries.push(ceiling),
        }
    }

    /// Whether every inserted ceiling is the card's `skill_max`.
    pub(crate) fn is_fixed(&self) -> bool {
        self.entries.iter().all(SkillCeiling::is_fixed)
    }

    /// At least [`SkillCeiling::candidate`] of every inserted card.
    #[inline(always)]
    pub(crate) fn candidate(&self, composition: &Composition, free: usize) -> u32 {
        self.entries
            .iter()
            .map(|entry| entry.candidate(composition, free))
            .max()
            .unwrap_or(0)
    }
}
