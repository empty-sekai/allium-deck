//! Exact-safe card-pool filtering.
//!
//! Pool construction applies only hard user constraints here.  Objective- or
//! quality-based candidate reduction belongs in the search layer, where exact
//! dominance can record inverse alternatives for Top-K recovery.

use super::build::PreparedCardSeed;
use super::event_bonus::EventContext;
use super::gather::CardIntermediate;
use super::types::{self, attr_to_pool_index, parse_attr_code, parse_unit_code};

/// Hard filters available before power/skill have been computed.
pub(super) fn prepared_keep_card(card: &PreparedCardSeed<'_>, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.master.id);
    if params.excluded_cards.contains(&card.master.id) {
        return false;
    }
    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
            && card.unit_mask & (1u8 << unit) == 0
        {
            return false;
        }
        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
            && card.attr != attr
        {
            return false;
        }
        if params.filter_other_unit
            && let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
            && card.unit_mask & (1u8 << unit) == 0
        {
            return false;
        }
    }
    params
        .challenge_live_character_id
        .is_none_or(|character_id| card.master.character_id == character_id)
}

/// Applies the resolved event-unit hard filter after the event context exists.
pub(super) fn prepared_post_event_unit_filter(
    card: &PreparedCardSeed<'_>,
    params: &types::BuildParams,
    event_ctx: Option<&EventContext>,
) -> bool {
    if !params.filter_other_unit {
        return true;
    }
    let Some(unit) = event_ctx.and_then(|ctx| ctx.filter_unit) else {
        return true;
    };
    let Some(unit_index) = types::unit_to_pool_index(unit) else {
        return true;
    };
    let wanted = 1u8 << unit_index;
    let piapro = types::unit_to_pool_index(crate::types::Unit::Piapro)
        .map(|index| 1u8 << index)
        .unwrap_or(0);
    card.unit_mask & wanted != 0 || card.unit_mask == piapro
}

/// Hard filters after the full per-card state has been computed.
pub(super) fn keep_card(card: &CardIntermediate, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.game_card_id);
    if params.excluded_cards.contains(&card.game_card_id) {
        return false;
    }

    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
        {
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & wanted == 0 {
                return false;
            }
        }

        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
            && card.attr != attr
        {
            return false;
        }

        if params.filter_other_unit
            && let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
        {
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & wanted == 0 {
                return false;
            }
        }
    }

    if let Some(challenge_char_id) = params.challenge_live_character_id
        && card.character_id != challenge_char_id as u8
    {
        return false;
    }

    true
}
