use crate::types::{
    CardId, CardSpec, CustomBonusParams, EventContext, EventType, Unit, DECK_SIZE,
    FINAL_CHAPTER_EVENT_ID,
};

pub(crate) fn resolve_event_bonus(
    cards: &[&CardSpec; DECK_SIZE],
    event: Option<&EventContext>,
) -> ([f64; DECK_SIZE], f64, f64) {
    let Some(event) = event else {
        return ([0.0; DECK_SIZE], 0.0, 0.0);
    };

    let mut card_bonus = [0.0_f64; DECK_SIZE];
    let mut index = 0;
    while index < DECK_SIZE {
        let card = cards[index];
        let custom = custom_bonus_value(card, event.custom_bonus.as_ref());
        let base = card.event_bonus.base_bonus + custom;
        card_bonus[index] = if event.event_id == FINAL_CHAPTER_EVENT_ID {
            // moe deck-information/deck-calculator.cpp:28-33 adds leader bonuses then removes them from non-leaders.
            base + card.event_bonus.limited_bonus
                + card.event_bonus.leader_honor_bonus
                + card.event_bonus.leader_limit_bonus
        } else {
            base + card.event_bonus.limited_bonus
        };
        index += 1;
    }

    if event.event_id == FINAL_CHAPTER_EVENT_ID {
        let mut non_leader = 1;
        while non_leader < DECK_SIZE {
            card_bonus[non_leader] -= cards[non_leader].event_bonus.leader_honor_bonus;
            card_bonus[non_leader] -= cards[non_leader].event_bonus.leader_limit_bonus;
            non_leader += 1;
        }
        let mut limited_count = 0usize;
        let mut card_index = 0;
        while card_index < DECK_SIZE {
            if cards[card_index].event_bonus.limited_bonus > 0.0 {
                if limited_count >= event.card_bonus_count_limit {
                    card_bonus[card_index] -= cards[card_index].event_bonus.limited_bonus;
                } else {
                    limited_count += 1;
                }
            }
            card_index += 1;
        }
    }

    let diff_attr_bonus = if matches!(event.event_type, EventType::WorldBloom) {
        let world_bloom = event
            .world_bloom
            .as_ref()
            .expect("validated world bloom context");
        let mut seen = 0_u32;
        let mut attr_index = 0;
        while attr_index < DECK_SIZE {
            // B 实例 L562-573；diff attr bonus 直接按属性种类数查表，单独输出。
            seen |= 1 << cards[attr_index].attr as usize;
            attr_index += 1;
        }
        world_bloom.diff_attr_bonus_table[seen.count_ones() as usize]
    } else {
        0.0
    };

    let mut total = diff_attr_bonus;
    let mut total_index = 0;
    while total_index < DECK_SIZE {
        total += card_bonus[total_index];
        total_index += 1;
    }
    (card_bonus, diff_attr_bonus, total)
}

pub(crate) fn resolve_support_deck_bonus(
    cards: &[CardId; DECK_SIZE],
    selected: &[&CardSpec; DECK_SIZE],
    event: Option<&EventContext>,
) -> f64 {
    let Some(event) = event else {
        return 0.0;
    };
    if !matches!(event.event_type, EventType::WorldBloom) {
        return 0.0;
    }
    let world_bloom = event
        .world_bloom
        .as_ref()
        .expect("validated world bloom context");
    let mut total = 0.0;
    let mut count = 0usize;

    if event.event_id == FINAL_CHAPTER_EVENT_ID {
        // B 实例 L602-612：终章支援卡组按 leader character_id 选择。
        let leader_character_id = selected[0].character_id;
        let mut group_index = 0;
        while group_index < world_bloom.final_chapter_support.len() {
            let group = &world_bloom.final_chapter_support[group_index];
            if group.leader_character_id == leader_character_id {
                let mut support_index = 0;
                while support_index < group.cards.len() {
                    let support = group.cards[support_index];
                    if !deck_contains(cards, support.card_id) {
                        total += support.bonus;
                        count += 1;
                        if count >= world_bloom.support_deck_count {
                            return total;
                        }
                    }
                    support_index += 1;
                }
                return total;
            }
            group_index += 1;
        }
        return 0.0;
    }

    let mut support_index = 0;
    while support_index < world_bloom.support_cards.len() {
        let support = world_bloom.support_cards[support_index];
        if !deck_contains(cards, support.card_id) {
            // moe deck-information/deck-calculator.cpp:70-86 uses caller-provided supportDeckCount.
            total += support.bonus;
            count += 1;
            if count >= world_bloom.support_deck_count {
                return total;
            }
        }
        support_index += 1;
    }
    total
}

pub(crate) fn custom_bonus_value(card: &CardSpec, custom: Option<&CustomBonusParams>) -> f64 {
    let Some(custom) = custom else {
        return 0.0;
    };
    let character_id = card.character_id as usize;
    let character_matched = character_id < 32 && ((custom.character_mask >> character_id) & 1) == 1;
    let support_unit_ok = card.character_id < 21
        || character_id >= custom.support_unit_by_char.len()
        || matches!(custom.support_unit_by_char[character_id], Unit::None)
        || custom.support_unit_by_char[character_id] == card.support_unit
        || matches!(card.support_unit, Unit::None);
    // moe event-point/card-event-calculator.cpp:49-75 is the only reference with custom mixed bonus.
    let in_character = character_matched && support_unit_ok;
    let attr_matched = custom.attr.is_some_and(|attr| attr == card.attr);
    if in_character && attr_matched {
        50.0
    } else if in_character || attr_matched {
        25.0
    } else {
        0.0
    }
}

fn deck_contains(cards: &[CardId; DECK_SIZE], card_id: CardId) -> bool {
    let mut index = 0;
    while index < DECK_SIZE {
        if cards[index] == card_id {
            return true;
        }
        index += 1;
    }
    false
}
