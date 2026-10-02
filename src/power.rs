//! Deck composition and area-item power selection.
use crate::pool::{CardIdx, CardPool};
use serde::{Deserialize, Serialize};

/// Activation policy for owned multi-unit area effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiUnitBonusMode {
    /// Activate only for a multi-unit deck.
    #[default]
    ByDeck,
    /// Activate irrespective of deck composition.
    ForceOn,
    /// Ignore multi-unit effects, retaining all other effects of the same item.
    ForceOff,
}

impl MultiUnitBonusMode {
    /// Whether the multi-unit table is active for this composition.
    pub fn active(self, is_multi_unit: bool) -> bool {
        match self {
            Self::ByDeck => is_multi_unit,
            Self::ForceOn => true,
            Self::ForceOff => false,
        }
    }
}

/// Original character unit, distinct from skill membership and support unit.
pub(crate) fn original_unit(mask: u8) -> u8 {
    if mask & (1 << 5) != 0 {
        1 << 5
    } else {
        mask & mask.wrapping_neg()
    }
}

/// Deck-wide facts shared by all effective-power consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeckComposition {
    /// Intersection of original-or-support memberships (not a single unit).
    pub shared_units: u8,
    /// Whether five members share their attribute.
    pub shared_attribute: bool,
    /// Multi-unit furniture activation predicate.
    pub is_multi_unit: bool,
    /// Union of original character units, used for finale shuffle bonuses.
    pub original_unit_mask: u8,
}

impl DeckComposition {
    /// Computes composition in deck order. Partial decks never have ALL_MATCH.
    pub fn from_cards(pool: &CardPool, cards: &[CardIdx]) -> Self {
        let mut shared_units = 0x3f;
        let mut original_unit_mask = 0;
        let mut attrs = 0u8;
        let mut multi_units = 0u8;
        for &card in cards {
            let mask = pool.unit_mask_raw(card);
            let original = original_unit(mask);
            shared_units &= mask;
            original_unit_mask |= original;
            attrs |= 1 << pool.attr(card);
            if original != 1 << 5 {
                multi_units |= original;
            }
        }
        let mut need_vs = false;
        for &card in cards {
            let mask = pool.unit_mask_raw(card);
            if original_unit(mask) != 1 << 5 {
                continue;
            }
            let support = mask & !(1 << 5);
            if support == 0 || multi_units & support != 0 {
                need_vs = true;
            } else {
                multi_units |= support;
            }
        }
        if need_vs {
            multi_units |= 1 << 5;
        }
        Self {
            shared_units: if cards.len() == 5 { shared_units } else { 0 },
            shared_attribute: cards.len() == 5 && attrs.count_ones() == 1,
            is_multi_unit: multi_units.count_ones() > 1,
            original_unit_mask,
        }
    }

    /// Index of `(original ALL_MATCH, support ALL_MATCH, attribute ALL_MATCH)`.
    pub fn multi_key(self, membership: u8) -> usize {
        let original = original_unit(membership);
        let support = membership & !original;
        usize::from(self.shared_units & original != 0) * 4
            + usize::from(self.shared_units & support != 0) * 2
            + usize::from(self.shared_attribute)
    }

    /// Finale shuffle bonus for three, four, or five original character units.
    pub fn shuffle_bonus(self) -> u32 {
        match self.original_unit_mask.count_ones() {
            3 => 10,
            4 => 30,
            5 => 50,
            _ => 0,
        }
    }
}

/// The sole selector between the legacy profile table and the optional multi table.
#[inline]
pub fn effective_power(pool: &CardPool, card: CardIdx, composition: DeckComposition) -> u32 {
    if pool
        .multi_unit_bonus_mode()
        .active(composition.is_multi_unit)
        && let Some(values) = pool.multi_power_values(card)
    {
        return values[composition.multi_key(pool.unit_mask_raw(card))];
    }
    legacy_power(
        pool,
        card,
        composition.shared_units,
        composition.shared_attribute,
    )
}

#[inline]
pub(crate) fn legacy_power(
    pool: &CardPool,
    card: CardIdx,
    shared_units: u8,
    attr_all: bool,
) -> u32 {
    let mask = pool.unit_mask_raw(card);
    let lut = pool.power_lut(card);
    (0..6)
        .filter(|unit| mask & (1 << unit) != 0)
        .map(|unit| {
            let profile = ((lut >> (16 + unit)) & 1) as usize;
            let key = usize::from(shared_units & (1 << unit) != 0) * 2 + usize::from(attr_all);
            crate::search::evaluate::decode_u18(pool.power_values(card), lut, profile * 4 + key)
        })
        .max()
        .unwrap_or(0)
}
