use crate::types::{Attr, CardSpec, PowerDetail, Unit, ATTR_COUNT, DECK_SIZE, UNIT_COUNT};

#[derive(Debug, Copy, Clone)]
pub(crate) struct DeckPower {
    pub base: i32,
    pub area_item_bonus: i32,
    pub character_bonus: i32,
    pub fixture_bonus: i32,
    pub gate_bonus: i32,
    pub total: i32,
}

pub(crate) fn resolve_card_power(
    cards: &[&CardSpec; DECK_SIZE],
    attr_counts: &[i32; ATTR_COUNT],
    unit_counts: &[i32; UNIT_COUNT],
) -> [PowerDetail; DECK_SIZE] {
    let mut power = [PowerDetail::default(); DECK_SIZE];
    let mut card_index = 0;
    while card_index < DECK_SIZE {
        let card = cards[card_index];
        let attr_member = attr_counts[card.attr as usize];
        let mut best = PowerDetail::default();
        let mut best_set = false;
        let mut unit_index = 0;
        while unit_index < card.unit_count as usize {
            let unit = card.units[unit_index];
            // B 实例 L471-489；fallback 已由 handler 预解析到 PowerLookup，热路径只做数组索引。
            let current = resolve_power(card, unit, unit_counts[unit as usize], attr_member);
            if !best_set || current.total > best.total {
                best = current;
                best_set = true;
            }
            unit_index += 1;
        }
        power[card_index] = best;
        card_index += 1;
    }
    power
}

pub(crate) fn fold_deck_power(
    card_power: &[PowerDetail; DECK_SIZE],
    honor_bonus: i32,
) -> DeckPower {
    let mut power = DeckPower {
        base: 0,
        area_item_bonus: 0,
        character_bonus: 0,
        fixture_bonus: 0,
        gate_bonus: 0,
        total: honor_bonus,
    };
    let mut index = 0;
    while index < DECK_SIZE {
        let detail = card_power[index];
        power.base += detail.base;
        power.area_item_bonus += detail.area_item_bonus;
        power.character_bonus += detail.character_bonus;
        power.fixture_bonus += detail.fixture_bonus;
        power.gate_bonus += detail.gate_bonus;
        power.total += detail.total;
        index += 1;
    }
    power
}

fn resolve_power(card: &CardSpec, unit: Unit, unit_member: i32, attr_member: i32) -> PowerDetail {
    if matches!(unit, Unit::Diff) {
        let index = unit_member.clamp(0, 2) as usize;
        return card.power.diff[index];
    }
    card.power.resolved[unit as usize][member_key(unit_member, attr_member)]
}

/// 将 (unit_member, attr_member) 编码为 PowerLookup.resolved 的 0-3 索引。
/// unit_member == DECK_SIZE -> 全同组(1)，否则混组(0)
/// attr_member == DECK_SIZE -> 全同色(1)，否则混色(0)
/// key = unit_key * 2 + attr_key
fn member_key(unit_member: i32, attr_member: i32) -> usize {
    let unit_key = if unit_member == DECK_SIZE as i32 {
        1
    } else {
        0
    };
    let attr_key = if attr_member == DECK_SIZE as i32 {
        1
    } else {
        0
    };
    unit_key * 2 + attr_key
}

#[allow(dead_code)]
fn _keep_imports(_: Attr) {}
