use super::gather::FullPrecisionCard;
use super::types::{parse_unit_code, unit_to_pool_index, GameData, WBSupportDeckBonus};

/// 计算单张卡的 World Bloom 支援加成。
pub(crate) fn calc_wb_support_bonus(
    card: &FullPrecisionCard,
    game: &GameData<'_>,
    event_id: i32,
    turn: Option<i32>,
    special_character_id: Option<i32>,
) -> u16 {
    let Some(special_character_id) = special_character_id.filter(|id| *id > 0) else {
        return 0;
    };
    let Some(special_unit) = game
        .game_character_units
        .iter()
        .find(|entry| entry.game_character_id == special_character_id)
        .and_then(|entry| parse_unit_code(&entry.unit))
        .and_then(unit_to_pool_index)
    else {
        return 0;
    };
    if card.unit_mask_raw & (1u8 << special_unit) == 0 {
        return 0;
    }

    let Some(bonus_table) = support_bonus_table(game, turn)
        .iter()
        .find(|entry| rarity_matches(&entry.card_rarity_type, card.card_rarity_type))
    else {
        return 0;
    };

    let mut total = 0.0_f64;
    let character_type = if card.character_id as i32 == special_character_id {
        "specific"
    } else {
        "others"
    };
    total += find_character_bonus(bonus_table, character_type);
    total += bonus_table
        .world_bloom_support_deck_master_rank_bonuses
        .iter()
        .find(|entry| entry.master_rank == card.master_rank)
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0);
    total += bonus_table
        .world_bloom_support_deck_skill_level_bonuses
        .iter()
        .find(|entry| entry.skill_level == card.skill_level)
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0);

    for bonus in game.world_bloom_support_deck_unit_event_limited_bonuses {
        if bonus.event_id == event_id
            && bonus.game_character_id == special_character_id
            && bonus.card_id == card.game_card_id as i32
        {
            total += bonus.bonus_rate;
        }
    }

    rate_to_u16(total)
}

fn support_bonus_table<'a>(game: &'a GameData<'_>, turn: Option<i32>) -> &'a [WBSupportDeckBonus] {
    match turn {
        Some(1) => game.wb_support_deck_bonuses_wl1,
        Some(2) => game.wb_support_deck_bonuses_wl2,
        Some(3) => game.wb_support_deck_bonuses_wl3,
        _ => &[],
    }
}

fn find_character_bonus(table: &WBSupportDeckBonus, character_type: &str) -> f64 {
    table
        .world_bloom_support_deck_character_bonuses
        .iter()
        .find(|entry| {
            entry
                .world_bloom_support_deck_character_type
                .eq_ignore_ascii_case(character_type)
        })
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0)
}

fn rarity_matches(code: &str, card_rarity_type: i32) -> bool {
    match code.trim().to_ascii_lowercase().as_str() {
        "rarity_1" | "1" => card_rarity_type == 1,
        "rarity_2" | "2" => card_rarity_type == 2,
        "rarity_3" | "3" => card_rarity_type == 3,
        "rarity_4" | "4" => card_rarity_type == 4,
        "rarity_birthday" | "birthday" | "5" => card_rarity_type == 5,
        _ => false,
    }
}

fn rate_to_u16(value: f64) -> u16 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        value.round().min(u16::MAX as f64) as u16
    }
}
