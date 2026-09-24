//! Ceilings of composition-dependent skills: the largest value a card's skill
//! can resolve to in any deck around a given selection.
use crate::pool::{CardIdx, CardPool, DiffSkill};
use crate::search::evaluate;
use crate::types::DECK_SIZE;

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
}

impl Composition {
    #[inline(always)]
    pub(crate) fn with(mut self, pool: &CardPool, card: CardIdx) -> Self {
        let mask = pool.unit_mask_raw(card);
        for (unit, count) in self.unit_counts.iter_mut().enumerate() {
            *count += (mask >> unit) & 1;
        }
        self.member_units |= evaluate::member_unit(mask);
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
    pub(crate) fn new(pool: &CardPool, card: CardIdx) -> Self {
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
            _ => fixed,
        }
    }

    /// Whether the ceiling is `skill_max` in every deck.
    #[inline(always)]
    pub(crate) fn is_fixed(&self) -> bool {
        matches!(self.kind, CeilingKind::Fixed)
    }

    /// Ceiling in every deck made of the cards counted in `composition`, the
    /// card itself, and `unknown` further members. `own` is the card's
    /// contribution to its counted unit when `composition` does not hold it.
    #[inline(always)]
    fn bound(&self, composition: &Composition, own: u8, unknown: usize) -> u32 {
        let step = match self.kind {
            CeilingKind::Fixed => 0,
            CeilingKind::UnitCount => {
                let known = composition.unit_counts[usize::from(self.unit)] + own;
                (usize::from(known) + unknown).clamp(1, DECK_SIZE) - 1
            }
            CeilingKind::DifferentUnit => {
                let counted = (composition.member_units & !self.unit).count_ones() as usize;
                (counted + unknown).min(usize::from(DiffSkill::MAX_COUNTED_UNITS))
            }
        };
        u32::from(self.steps[step])
    }

    /// Ceiling of a selected card, held in `composition`, with `free` slots
    /// left.
    #[inline(always)]
    pub(crate) fn selected(&self, composition: &Composition, free: usize) -> u32 {
        self.bound(composition, 0, free)
    }

    /// Ceiling of an unselected card taken as one of the `free` remaining
    /// picks.
    #[inline(always)]
    pub(crate) fn candidate(&self, composition: &Composition, free: usize) -> u32 {
        self.bound(composition, self.carries, free - 1)
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
        let same = |entry: &&mut SkillCeiling| {
            entry.kind == ceiling.kind
                && entry.unit == ceiling.unit
                && entry.carries == ceiling.carries
        };
        match self.entries.iter_mut().find(same) {
            Some(entry) => {
                for (step, &other) in entry.steps.iter_mut().zip(&ceiling.steps) {
                    *step = (*step).max(other);
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
