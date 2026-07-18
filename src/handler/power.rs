use crate::types::{PowerDetail, Unit};

use super::index::PoolIndexes;
use super::types::{
    is_after_training, parse_unit_code, unit_to_pool_index, GameData, MasterCard, UserCard,
    UserProfile,
};
use crate::simd::{PowerAreaItem, SimdBackend};

/// 卡牌综合力构建结果。
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PowerResult {
    /// 按 real unit × member_key 预计算的综合力。
    pub resolved: [[PowerDetail; 4]; 6],
    /// 精确最小综合力。
    pub power_min: i32,
    /// 精确最大综合力。
    pub power_max: i32,
}

#[derive(Clone, Copy)]
pub(crate) struct PowerInput<'a> {
    pub(crate) user_card: &'a UserCard,
    pub(crate) master: &'a MasterCard,
    pub(crate) unit_mask: u8,
    pub(crate) attr: u8,
}

pub(crate) struct PreparedPowerContext {
    character_rank: [i32; 27],
    character_bonus_rate: [f64; 27],
    fixture_rate: [i32; 27],
    canvas_cards: Vec<u64>,
    area_items: Vec<PowerAreaItem>,
    gate_rate_by_unit: [f64; 6],
    gate_rate_universal: f64,
    gate_rate_all: f64,
}

impl PreparedPowerContext {
    pub(crate) fn new(
        user: &UserProfile,
        game: &GameData<'_>,
        idx: &PoolIndexes,
        fixture_bonus_limit: Option<i32>,
    ) -> Self {
        let mut character_rank = [0; 27];
        for entry in &user.user_characters {
            if let Some(rank) = character_rank.get_mut(entry.character_id as usize) {
                *rank = entry.character_rank;
            }
        }
        let mut character_bonus_rate = [0.0; 27];
        for (character_id, &rank) in character_rank.iter().enumerate() {
            character_bonus_rate[character_id as usize] = game
                .character_ranks
                .iter()
                .filter(|entry| entry.character_rank <= rank)
                .max_by_key(|entry| entry.character_rank)
                .map(|entry| entry.power_bonus_rate)
                .unwrap_or(0.0);
        }

        let mut fixture_rate = [0; 27];
        let mut fixture_seen = [false; 27];
        for entry in &user.user_mysekai_fixture_bonuses {
            let Some(slot) = fixture_rate.get_mut(entry.character_id as usize) else {
                continue;
            };
            let seen = &mut fixture_seen[entry.character_id as usize];
            if !*seen {
                *slot = fixture_bonus_limit.map_or(entry.total_bonus_rate, |limit| {
                    entry.total_bonus_rate.min(limit)
                });
                *seen = true;
            }
        }

        let mut area_items = Vec::new();
        for user_item in &user.user_area_items {
            area_items.extend_from_slice(idx.area_items(user_item.area_item_id, user_item.level));
        }

        let mut gate_rate_by_unit = [0.0_f64; 6];
        let mut gate_rate_universal = 0.0_f64;
        let mut gate_rate_all = 0.0_f64;
        for entry in &user.user_mysekai_gate_bonuses {
            let Some((unit_code, rate)) = resolve_user_gate_bonus(entry, game) else {
                continue;
            };
            gate_rate_all = gate_rate_all.max(rate);
            if unit_code.trim().is_empty() {
                gate_rate_universal = gate_rate_universal.max(rate);
            } else if let Some(unit) = parse_unit_code(unit_code).and_then(unit_to_pool_index) {
                gate_rate_by_unit[unit as usize] = gate_rate_by_unit[unit as usize].max(rate);
            }
        }

        let max_canvas_id = game
            .cards
            .iter()
            .map(|card| card.id)
            .filter(|id| *id >= 0)
            .max()
            .unwrap_or(0) as usize;
        let mut canvas_cards = vec![0u64; (max_canvas_id >> 6) + 1];
        for &card_id in &user.user_mysekai_canvas_bonus_cards {
            if card_id >= 0 && card_id as usize <= max_canvas_id {
                canvas_cards[card_id as usize >> 6] |= 1u64 << (card_id as usize & 63);
            }
        }

        Self {
            character_rank,
            character_bonus_rate,
            fixture_rate,
            canvas_cards,
            area_items,
            gate_rate_by_unit,
            gate_rate_universal,
            gate_rate_all,
        }
    }

    #[inline(always)]
    pub(crate) fn character_rank(&self, character_id: i32) -> i32 {
        self.character_rank
            .get(character_id as usize)
            .copied()
            .unwrap_or(0)
    }

    #[inline(always)]
    fn character_rate(&self, character_id: i32) -> f64 {
        self.character_bonus_rate
            .get(character_id as usize)
            .copied()
            .unwrap_or(0.0)
    }

    #[inline(always)]
    fn has_canvas(&self, card_id: i32) -> bool {
        if card_id < 0 {
            return false;
        }
        let card_id = card_id as usize;
        self.canvas_cards
            .get(card_id >> 6)
            .is_some_and(|word| word & (1u64 << (card_id & 63)) != 0)
    }

    #[inline(always)]
    fn fixture_rate(&self, character_id: i32) -> f64 {
        self.fixture_rate
            .get(character_id as usize)
            .copied()
            .unwrap_or(0) as f64
    }

    #[inline(always)]
    fn gate_rate(&self, unit_mask: u8) -> f64 {
        let piapro_mask = 1u8 << unit_to_pool_index(Unit::Piapro).unwrap_or(5);
        let mut rate = if unit_mask == piapro_mask {
            self.gate_rate_all
        } else {
            self.gate_rate_universal
        };
        if unit_mask != piapro_mask {
            let mut units = unit_mask;
            while units != 0 {
                let unit = units.trailing_zeros() as usize;
                units &= units - 1;
                rate = rate.max(self.gate_rate_by_unit[unit]);
            }
        }
        rate
    }

    #[inline(always)]
    #[cfg(test)]
    fn fixture_bonus(&self, sum_power: i32, character_id: i32) -> i32 {
        let rate = self
            .fixture_rate
            .get(character_id as usize)
            .copied()
            .unwrap_or(0);
        ((sum_power as f64) * (rate as f64) * 0.001_f64).floor() as i32
    }

    #[inline(always)]
    #[cfg(test)]
    fn gate_bonus(&self, sum_power: i32, unit_mask: u8) -> i32 {
        let rate = self.gate_rate(unit_mask);
        ((sum_power as f64) * rate * 0.01_f64).floor() as i32
    }
}

fn base_power_dims(
    user_card: &UserCard,
    master: &MasterCard,
    ctx: &PreparedPowerContext,
    idx: &PoolIndexes,
) -> [i32; 3] {
    let level = user_card.level.max(1);
    let mut base = idx.base_power(master.id, level);

    if is_after_training(&user_card.special_training_status) {
        base[0] += master.special_training_power1_bonus_fixed;
        base[1] += master.special_training_power2_bonus_fixed;
        base[2] += master.special_training_power3_bonus_fixed;
    }

    for episode in idx.card_episodes(master.id) {
        if user_card.episodes_read.contains(&episode.episode_no) {
            base[0] += episode.power1_bonus_fixed;
            base[1] += episode.power2_bonus_fixed;
            base[2] += episode.power3_bonus_fixed;
        }
    }

    for lesson in idx.master_lessons(master.card_rarity_type) {
        if lesson.master_rank <= user_card.master_rank {
            base[0] += lesson.power1_bonus_fixed;
            base[1] += lesson.power2_bonus_fixed;
            base[2] += lesson.power3_bonus_fixed;
        }
    }

    let has_canvas_bonus = user_card
        .has_canvas_bonus_override
        .unwrap_or_else(|| ctx.has_canvas(master.id));
    if has_canvas_bonus {
        if let Some(canvas) = idx.canvas_bonus(master.card_rarity_type) {
            base[0] += canvas.power1_bonus_fixed;
            base[1] += canvas.power2_bonus_fixed;
            base[2] += canvas.power3_bonus_fixed;
        }
    }

    base
}

#[cfg(test)]
fn character_bonus_dims(
    master: &MasterCard,
    ctx: &PreparedPowerContext,
    base: [i32; 3],
) -> [i32; 3] {
    let rate = ctx.character_rate(master.character_id);
    [
        floor_mul_rate(base[0], rate),
        floor_mul_rate(base[1], rate),
        floor_mul_rate(base[2], rate),
    ]
}

#[allow(clippy::too_many_arguments)]
fn area_item_bonus_dims(
    master: &MasterCard,
    ctx: &PreparedPowerContext,
    unit_mask: u8,
    card_attr: u8,
    base: [i32; 3],
    same_unit: bool,
    same_attr: bool,
    target_unit: u8,
) -> [i32; 3] {
    let mut acc = [0.0_f64; 3];

    for item in &ctx.area_items {
        let unit_ok = item.unit == PowerAreaItem::ANY
            || (item.unit == target_unit && unit_mask & (1u8 << item.unit) != 0);
        let attr_ok = item.attr == PowerAreaItem::ANY || card_attr == item.attr;
        let character_ok = item.character_id == PowerAreaItem::ANY_CHARACTER
            || item.character_id == master.character_id;
        if !(unit_ok && attr_ok && character_ok) {
            continue;
        }

        let all_match = (item.unit != PowerAreaItem::ANY && same_unit)
            || (item.attr != PowerAreaItem::ANY && same_attr);
        // 适配层约定：三维 area item 倍率已验证相等并折叠为单一倍率；
        // 若 masterdata 出现三维不等，需扩展 AreaItemLevel 类型而不是继续复用该字段。
        let power_rate = if all_match {
            item.power_all_match_rate
        } else {
            item.power_rate
        };
        acc[0] += power_rate * 0.01_f64 * base[0] as f64;
        acc[1] += power_rate * 0.01_f64 * base[1] as f64;
        acc[2] += power_rate * 0.01_f64 * base[2] as f64;
    }

    [
        acc[0].floor() as i32,
        acc[1].floor() as i32,
        acc[2].floor() as i32,
    ]
}

fn resolve_user_gate_bonus<'a>(
    entry: &'a super::types::UserGateBonus,
    game: &'a GameData<'_>,
) -> Option<(&'a str, f64)> {
    if let (Some(gate_id), Some(level)) = (entry.mysekai_gate_id, entry.mysekai_gate_level) {
        let gate = game.mysekai_gates.iter().find(|gate| gate.id == gate_id)?;
        let level = game
            .mysekai_gate_levels
            .iter()
            .find(|row| row.mysekai_gate_id == gate_id && row.level == level)?;
        return Some((gate.unit.as_str(), level.power_bonus_rate));
    }
    if entry.bonus_rate > 0.0 {
        Some((entry.unit.as_str(), entry.bonus_rate))
    } else {
        None
    }
}

#[cfg(test)]
fn floor_mul_rate(base: i32, rate: f64) -> i32 {
    (((rate as f32) * 0.01_f32) * (base as f32)).floor() as i32
}

/// 构建单卡的综合力预计算表。
#[cfg(test)]
pub(crate) fn build_power(
    user_card: &UserCard,
    master: &MasterCard,
    ctx: &PreparedPowerContext,
    idx: &PoolIndexes,
    unit_mask: u8,
    card_attr: u8,
) -> PowerResult {
    let input = PowerInput {
        user_card,
        master,
        unit_mask,
        attr: card_attr,
    };
    build_power_batch(std::slice::from_ref(&input), ctx, idx)
        .pop()
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn build_power_batch(
    inputs: &[PowerInput<'_>],
    ctx: &PreparedPowerContext,
    idx: &PoolIndexes,
) -> Vec<PowerResult> {
    let mut results = Vec::with_capacity(inputs.len());
    build_power_batch_into(inputs, ctx, idx, &mut results);
    results
}

pub(crate) fn build_power_batch_into(
    inputs: &[PowerInput<'_>],
    ctx: &PreparedPowerContext,
    idx: &PoolIndexes,
    results: &mut Vec<PowerResult>,
) {
    results.clear();
    results.reserve(inputs.len());
    let backend = SimdBackend::detect();
    for block in inputs.chunks(16) {
        let mut base_dims = [[0i32; 16]; 3];
        let mut base_sum = [0i32; 16];
        let mut character_rates = [0f32; 16];
        let mut fixture_rates = [0f64; 16];
        let mut gate_rates = [0f64; 16];
        let mut target_units = [0u8; 16];
        let mut secondary_units = [0u8; 16];
        let mut attrs = [0u8; 16];
        let mut character_ids = [0i32; 16];
        let mut primary_unit_lanes = 0u16;
        let mut secondary_unit_lanes = 0u16;
        let mut lane = 0usize;
        while lane < block.len() {
            let input = block[lane];
            let base = base_power_dims(input.user_card, input.master, ctx, idx);
            base_dims[0][lane] = base[0];
            base_dims[1][lane] = base[1];
            base_dims[2][lane] = base[2];
            base_sum[lane] = base[0] + base[1] + base[2];
            character_rates[lane] = ctx.character_rate(input.master.character_id) as f32;
            fixture_rates[lane] = ctx.fixture_rate(input.master.character_id);
            gate_rates[lane] = ctx.gate_rate(input.unit_mask);
            target_units[lane] = input.unit_mask.trailing_zeros() as u8;
            attrs[lane] = input.attr;
            character_ids[lane] = input.master.character_id;
            let unit_count = input.unit_mask.count_ones();
            if (1..=2).contains(&unit_count) {
                primary_unit_lanes |= 1u16 << lane;
            }
            if unit_count == 2 {
                let remaining = input.unit_mask & !(1u8 << target_units[lane]);
                secondary_units[lane] = remaining.trailing_zeros() as u8;
                secondary_unit_lanes |= 1u16 << lane;
            }
            lane += 1;
        }
        let common = unsafe {
            backend.power_common_16(
                &base_dims,
                &base_sum,
                &character_rates,
                &fixture_rates,
                &gate_rates,
                block.len(),
            )
        };
        let mut primary_area_sums = [[0i32; 16]; 4];
        let mut secondary_area_sums = [[0i32; 16]; 4];
        let mut member_key = 0usize;
        while member_key < 4 {
            primary_area_sums[member_key] = unsafe {
                backend.power_area_single_unit_16(
                    &base_dims,
                    &target_units,
                    &attrs,
                    &character_ids,
                    &ctx.area_items,
                    member_key,
                    primary_unit_lanes,
                )
            };
            if secondary_unit_lanes != 0 {
                secondary_area_sums[member_key] = unsafe {
                    backend.power_area_single_unit_16(
                        &base_dims,
                        &secondary_units,
                        &attrs,
                        &character_ids,
                        &ctx.area_items,
                        member_key,
                        secondary_unit_lanes,
                    )
                };
            }
            member_key += 1;
        }
        lane = 0;
        while lane < block.len() {
            let input = block[lane];
            let character_sum = common.character_bonus[lane];
            let fixture = common.fixture_bonus[lane];
            let gate = common.gate_bonus[lane];
            let mut result = PowerResult::default();
            let mut min_value = i32::MAX;
            let mut max_value = i32::MIN;
            let unit_count = input.unit_mask.count_ones();
            if (1..=2).contains(&unit_count) {
                let pool_index = target_units[lane] as usize;
                let mut member_key = 0usize;
                while member_key < 4 {
                    let area_sum = primary_area_sums[member_key][lane];
                    let total = base_sum[lane] + character_sum + area_sum + fixture + gate;
                    result.resolved[pool_index][member_key] = PowerDetail {
                        base: base_sum[lane],
                        area_item_bonus: area_sum,
                        character_bonus: character_sum,
                        fixture_bonus: fixture,
                        gate_bonus: gate,
                        total,
                    };
                    min_value = min_value.min(total);
                    max_value = max_value.max(total);
                    member_key += 1;
                }
            }
            if unit_count == 2 {
                let pool_index = secondary_units[lane] as usize;
                let mut member_key = 0usize;
                while member_key < 4 {
                    let area_sum = secondary_area_sums[member_key][lane];
                    let total = base_sum[lane] + character_sum + area_sum + fixture + gate;
                    result.resolved[pool_index][member_key] = PowerDetail {
                        base: base_sum[lane],
                        area_item_bonus: area_sum,
                        character_bonus: character_sum,
                        fixture_bonus: fixture,
                        gate_bonus: gate,
                        total,
                    };
                    min_value = min_value.min(total);
                    max_value = max_value.max(total);
                    member_key += 1;
                }
            }
            let mut pool_index = 0u8;
            while unit_count > 2 && pool_index < 6 {
                if input.unit_mask & (1u8 << pool_index) != 0 {
                    let base = [base_dims[0][lane], base_dims[1][lane], base_dims[2][lane]];
                    let mut member_key = 0usize;
                    while member_key < 4 {
                        let area_bonus = area_item_bonus_dims(
                            input.master,
                            ctx,
                            input.unit_mask,
                            input.attr,
                            base,
                            member_key >= 2,
                            member_key % 2 == 1,
                            pool_index,
                        );
                        let area_sum = area_bonus[0] + area_bonus[1] + area_bonus[2];
                        let total = base_sum[lane] + character_sum + area_sum + fixture + gate;
                        result.resolved[pool_index as usize][member_key] = PowerDetail {
                            base: base_sum[lane],
                            area_item_bonus: area_sum,
                            character_bonus: character_sum,
                            fixture_bonus: fixture,
                            gate_bonus: gate,
                            total,
                        };
                        min_value = min_value.min(total);
                        max_value = max_value.max(total);
                        member_key += 1;
                    }
                }
                pool_index += 1;
            }
            if min_value == i32::MAX {
                min_value = 0;
                max_value = 0;
            }
            result.power_min = min_value;
            result.power_max = max_value;
            results.push(result);
            lane += 1;
        }
    }
}

#[cfg(test)]
pub(crate) fn build_power_scalar_reference(
    user_card: &UserCard,
    master: &MasterCard,
    ctx: &PreparedPowerContext,
    idx: &PoolIndexes,
    unit_mask: u8,
    card_attr: u8,
) -> PowerResult {
    let base = base_power_dims(user_card, master, ctx, idx);
    let character_bonus = character_bonus_dims(master, ctx, base);
    let base_sum = base[0] + base[1] + base[2];
    let character_sum = character_bonus[0] + character_bonus[1] + character_bonus[2];
    let mut result = PowerResult::default();
    let mut min_value = i32::MAX;
    let mut max_value = i32::MIN;
    let fixture = ctx.fixture_bonus(base_sum, master.character_id);
    let gate = ctx.gate_bonus(base_sum, unit_mask);

    for pool_index in 0..6u8 {
        if unit_mask & (1u8 << pool_index) == 0 {
            continue;
        }

        for member_key in 0..4usize {
            let same_unit = member_key >= 2;
            let same_attr = member_key % 2 == 1;
            let area_bonus = area_item_bonus_dims(
                master, ctx, unit_mask, card_attr, base, same_unit, same_attr, pool_index,
            );
            let area_sum = area_bonus[0] + area_bonus[1] + area_bonus[2];
            let total = base_sum + character_sum + area_sum + fixture + gate;

            let detail = PowerDetail {
                base: base_sum,
                area_item_bonus: area_sum,
                character_bonus: character_sum,
                fixture_bonus: fixture,
                gate_bonus: gate,
                total,
            };
            result.resolved[pool_index as usize][member_key] = detail;
            min_value = min_value.min(total);
            max_value = max_value.max(total);
        }
    }

    if min_value == i32::MAX {
        min_value = 0;
        max_value = 0;
    }
    result.power_min = min_value;
    result.power_max = max_value;
    result
}

#[cfg(test)]
mod tests {
    use super::{PoolIndexes, PreparedPowerContext};
    use crate::handler::types::{
        GameData, MysekaiGate, MysekaiGateLevel, UserFixtureBonus, UserGateBonus,
    };
    use crate::handler::UserProfile;

    #[test]
    fn fixture_bonus_only_clamps_when_limit_is_present() {
        let user = UserProfile {
            user_mysekai_fixture_bonuses: vec![UserFixtureBonus {
                character_id: 20,
                event_id: None,
                total_bonus_rate: 30,
            }],
            ..UserProfile::default()
        };

        let game = GameData {
            cards: &[],
            card_parameters: &[],
            card_rarities: &[],
            card_episodes: &[],
            master_lessons: &[],
            skills: &[],
            skill_effects: &[],
            area_item_levels: &[],
            game_character_units: &[],
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &[],
            mysekai_gate_levels: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let idx = PoolIndexes::build(&game);
        assert_eq!(
            PreparedPowerContext::new(&user, &game, &idx, None).fixture_bonus(38_792, 20),
            1_163
        );
        assert_eq!(
            PreparedPowerContext::new(&user, &game, &idx, Some(20)).fixture_bonus(38_792, 20),
            775
        );
    }

    #[test]
    fn gate_bonus_uses_masterdata_gate_level_rate() {
        let gates = [MysekaiGate {
            id: 1,
            unit: "light_sound".to_string(),
        }];
        let levels = [MysekaiGateLevel {
            mysekai_gate_id: 1,
            level: 2,
            power_bonus_rate: 1.5,
        }];
        let game = GameData {
            cards: &[],
            card_parameters: &[],
            card_rarities: &[],
            card_episodes: &[],
            master_lessons: &[],
            skills: &[],
            skill_effects: &[],
            area_item_levels: &[],
            game_character_units: &[],
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &gates,
            mysekai_gate_levels: &levels,
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_mysekai_gate_bonuses: vec![UserGateBonus {
                mysekai_gate_id: Some(1),
                mysekai_gate_level: Some(2),
                unit: "school_refusal".to_string(),
                bonus_rate: 40.0,
            }],
            ..UserProfile::default()
        };

        let idx = PoolIndexes::build(&game);
        let ctx = PreparedPowerContext::new(&user, &game, &idx, None);
        assert_eq!(ctx.gate_bonus(10_000, 1 << 0), 150);
        assert_eq!(ctx.gate_bonus(10_000, 1 << 1), 0);
    }
}

/// 解析卡的 unit bitmask。
pub(crate) fn resolve_unit_mask(master: &MasterCard, game: &GameData<'_>) -> u8 {
    let primary = game
        .game_character_units
        .iter()
        .find(|entry| entry.game_character_id == master.character_id)
        .and_then(|entry| parse_unit_code(&entry.unit));
    let mut mask = primary
        .and_then(unit_to_pool_index)
        .map_or(0, |unit| 1u8 << unit);
    if matches!(primary, Some(Unit::Piapro)) {
        if let Some(secondary) = master
            .support_unit
            .as_deref()
            .and_then(parse_unit_code)
            .filter(|unit| !matches!(unit, Unit::Piapro))
            .and_then(unit_to_pool_index)
        {
            mask |= 1u8 << secondary;
        }
    }
    mask
}
