use crate::types::{PowerDetail, Unit};

use super::index::PoolIndexes;
use super::types::{
    is_after_training, parse_attr_code, parse_unit_code, pool_index_to_unit, unit_to_pool_index,
    GameData, MasterCard, UserCard, UserProfile,
};

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

fn build_unit_list(master: &MasterCard, game: &GameData<'_>) -> Vec<Unit> {
    let primary = game
        .game_character_units
        .iter()
        .find(|entry| entry.game_character_id == master.character_id)
        .and_then(|entry| parse_unit_code(&entry.unit));
    let mut units = Vec::new();
    if let Some(primary) = primary {
        units.push(primary);
        if matches!(primary, Unit::Piapro) {
            if let Some(secondary) = master
                .support_unit
                .as_deref()
                .and_then(parse_unit_code)
                .filter(|unit| !matches!(unit, Unit::Piapro))
            {
                units.push(secondary);
            }
        }
    }
    units
}

fn base_power_dims(
    user_card: &UserCard,
    master: &MasterCard,
    user: &UserProfile,
    idx: &PoolIndexes<'_>,
) -> [i32; 3] {
    let level = user_card.level.max(1);
    let mut base = idx
        .card_parameters(master.id)
        .iter()
        .filter(|entry| entry.level <= level)
        .max_by_key(|entry| entry.level)
        .map(|entry| [entry.param1, entry.param2, entry.param3])
        .unwrap_or([0; 3]);

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
        .unwrap_or_else(|| user.user_mysekai_canvas_bonus_cards.contains(&master.id));
    if has_canvas_bonus {
        if let Some(canvas) = idx.canvas_bonus(master.card_rarity_type) {
            base[0] += canvas.power1_bonus_fixed;
            base[1] += canvas.power2_bonus_fixed;
            base[2] += canvas.power3_bonus_fixed;
        }
    }

    base
}

fn character_bonus_dims(
    master: &MasterCard,
    game: &GameData<'_>,
    user: &UserProfile,
    base: [i32; 3],
) -> [i32; 3] {
    let character_rank = user
        .user_characters
        .iter()
        .find(|entry| entry.character_id == master.character_id)
        .map(|entry| entry.character_rank)
        .unwrap_or(0);
    let rate = game
        .character_ranks
        .iter()
        .filter(|entry| entry.character_rank <= character_rank)
        .max_by_key(|entry| entry.character_rank)
        .map(|entry| entry.power_bonus_rate)
        .unwrap_or(0.0);
    [
        floor_mul_rate(base[0], rate),
        floor_mul_rate(base[1], rate),
        floor_mul_rate(base[2], rate),
    ]
}

#[allow(clippy::too_many_arguments)]
fn area_item_bonus_dims(
    master: &MasterCard,
    idx: &PoolIndexes<'_>,
    user: &UserProfile,
    card_units: &[Unit],
    base: [i32; 3],
    same_unit: bool,
    same_attr: bool,
    target_unit: Unit,
) -> [i32; 3] {
    let card_attr = parse_attr_code(&master.attr);
    let mut acc = [0.0_f64; 3];

    for user_item in &user.user_area_items {
        for item in idx.area_items(user_item.area_item_id, user_item.level) {
            let unit_ok = match item.unit.as_deref().and_then(parse_unit_code) {
                Some(unit) => unit == target_unit && card_units.contains(&unit),
                None => true,
            };
            let attr_ok = match item.attr.as_deref().and_then(parse_attr_code) {
                Some(attr) => card_attr == Some(attr),
                None => true,
            };
            let character_ok = item
                .character_id
                .is_none_or(|character_id| character_id == master.character_id);
            if !(unit_ok && attr_ok && character_ok) {
                continue;
            }

            let all_match =
                (item.unit.is_some() && same_unit) || (item.attr.is_some() && same_attr);
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
    }

    [
        acc[0].floor() as i32,
        acc[1].floor() as i32,
        acc[2].floor() as i32,
    ]
}

fn fixture_bonus(
    sum_power: i32,
    character_id: i32,
    bonus_limit: Option<i32>,
    user: &UserProfile,
) -> i32 {
    let bonus_rate = user
        .user_mysekai_fixture_bonuses
        .iter()
        .find(|entry| entry.character_id == character_id)
        .map(|entry| entry.total_bonus_rate)
        .unwrap_or(0);
    let clamped = bonus_limit.map_or(bonus_rate, |limit| bonus_rate.min(limit));
    ((sum_power as f64) * (clamped as f64) * 0.001_f64).floor() as i32
}

fn gate_bonus(sum_power: i32, card_units: &[Unit], game: &GameData<'_>, user: &UserProfile) -> i32 {
    let is_only_piapro = card_units.len() == 1 && matches!(card_units[0], Unit::Piapro);
    let max_rate = user
        .user_mysekai_gate_bonuses
        .iter()
        .filter_map(|entry| resolve_user_gate_bonus(entry, game))
        .filter(|(unit_code, _)| {
            is_only_piapro
                || unit_code.trim().is_empty()
                || parse_unit_code(unit_code.trim()).is_some_and(|unit| card_units.contains(&unit))
        })
        .map(|(_, rate)| rate)
        .fold(0.0_f64, f64::max);
    ((sum_power as f64) * max_rate * 0.01_f64).floor() as i32
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

fn floor_mul_rate(base: i32, rate: f64) -> i32 {
    (((rate as f32) * 0.01_f32) * (base as f32)).floor() as i32
}

/// 构建单卡的综合力预计算表。
pub(crate) fn build_power(
    user_card: &UserCard,
    master: &MasterCard,
    game: &GameData<'_>,
    user: &UserProfile,
    idx: &PoolIndexes<'_>,
    fixture_bonus_limit: Option<i32>,
) -> PowerResult {
    let card_units = build_unit_list(master, game);
    let base = base_power_dims(user_card, master, user, idx);
    let character_bonus = character_bonus_dims(master, game, user, base);
    let base_sum = base[0] + base[1] + base[2];
    let character_sum = character_bonus[0] + character_bonus[1] + character_bonus[2];
    let mut result = PowerResult::default();
    let mut min_value = i32::MAX;
    let mut max_value = i32::MIN;

    for pool_index in 0..6u8 {
        let Some(target_unit) = pool_index_to_unit(pool_index) else {
            continue;
        };
        if !card_units.contains(&target_unit) {
            continue;
        }

        for member_key in 0..4usize {
            let same_unit = member_key >= 2;
            let same_attr = member_key % 2 == 1;
            let area_bonus = area_item_bonus_dims(
                master,
                idx,
                user,
                &card_units,
                base,
                same_unit,
                same_attr,
                target_unit,
            );
            let area_sum = area_bonus[0] + area_bonus[1] + area_bonus[2];
            let fixture = fixture_bonus(base_sum, master.character_id, fixture_bonus_limit, user);
            let gate = gate_bonus(base_sum, &card_units, game, user);
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
    use super::{fixture_bonus, gate_bonus};
    use crate::handler::types::{
        GameData, MysekaiGate, MysekaiGateLevel, UserFixtureBonus, UserGateBonus,
    };
    use crate::handler::UserProfile;
    use crate::types::Unit;

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

        assert_eq!(fixture_bonus(38_792, 20, None, &user), 1_163);
        assert_eq!(fixture_bonus(38_792, 20, Some(20), &user), 775);
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

        assert_eq!(gate_bonus(10_000, &[Unit::LightSound], &game, &user), 150);
        assert_eq!(gate_bonus(10_000, &[Unit::Idol], &game, &user), 0);
    }
}

/// 解析卡的 unit bitmask。
pub(crate) fn resolve_unit_mask(master: &MasterCard, game: &GameData<'_>) -> u8 {
    let mut mask = 0u8;
    for unit in build_unit_list(master, game) {
        if let Some(index) = unit_to_pool_index(unit) {
            mask |= 1u8 << index;
        }
    }
    mask
}
