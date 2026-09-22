//! Checked boundaries of the compact search representation.
//! These errors occur before lossy casts, saturation, or arena construction.
use super::{BuildError, gather::CardIntermediate};
use crate::pool::EventBonusHot;

pub(super) fn ensure(field: &'static str, value: u64, max: u64) -> Result<(), BuildError> {
    if value > max {
        Err(BuildError::CapacityExceeded { field, value, max })
    } else {
        Ok(())
    }
}

pub(super) fn score(value: i64, limit: Option<u32>, field: &'static str) -> Result<u8, BuildError> {
    let value = value.max(0) as u64;
    let value = limit.map_or(value, |limit| value.min(u64::from(limit)));
    ensure(field, value, u64::from(u8::MAX))?;
    Ok(value as u8)
}

pub(super) fn validate_cards(cards: &[CardIntermediate]) -> Result<(), BuildError> {
    let capacity = crate::pool::MASK_WORDS * 64;
    if cards.len() > capacity {
        return Err(BuildError::TooManyCards(cards.len()));
    }
    let mut limited_values = Vec::new();
    for card in cards {
        if card.game_card_id < 0 {
            return Err(BuildError::InvalidConfig(
                "card identity must be nonnegative".to_string(),
            ));
        }
        ensure(
            "public card id",
            card.game_card_id as u64,
            u64::from(u16::MAX),
        )?;
        ensure("character id", u64::from(card.character_id), 26)?;
        ensure("attribute id", u64::from(card.attr), 5)?;
        ensure("unit mask", u64::from(card.unit_mask_raw), 63)?;
        ensure(
            "per-card power unit profiles",
            u64::from(card.unit_mask_raw.count_ones()),
            2,
        )?;
        for unit in 0..6 {
            for members in 0..4 {
                ensure(
                    "card power",
                    card.power.detail(unit, members).total.max(0) as u64,
                    (1 << 18) - 1,
                )?;
            }
        }
        let total =
            u64::from(card.event_bonus.base_x10()) + u64::from(card.event_bonus.limited_x10());
        ensure(
            "card event bonus (tenths)",
            total,
            u64::from(EventBonusHot::MAX_TOTAL_X10),
        )?;
        let limited = card.event_bonus.limited_x10();
        if limited != 0 && !limited_values.contains(&limited) {
            limited_values.push(limited);
        }
        if let Some(reference) = card.skill.ref_skill {
            let upper = u64::from(card.skill.skill_min) + u64::from(reference.max);
            ensure(
                "reference skill upper bound",
                upper,
                u64::from(card.skill.skill_max),
            )?;
        }
    }
    ensure(
        "distinct limited bonus values",
        limited_values.len() as u64,
        15,
    )
}

/// One-based indices retain the existing zero sentinel. Identical content never
/// consumes another slot, even when many public cards share the same skill.
pub(super) fn intern<T: Copy + PartialEq>(
    values: &mut Vec<T>,
    value: T,
    field: &'static str,
) -> Result<(u8, bool), BuildError> {
    if let Some(index) = values.iter().position(|old| *old == value) {
        return Ok(((index + 1) as u8, false));
    }
    ensure(field, (values.len() + 1) as u64, u64::from(u8::MAX))?;
    values.push(value);
    Ok((values.len() as u8, true))
}
