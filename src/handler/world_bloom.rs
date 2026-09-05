//! World Bloom 域：支援卡评估、支援 deck 构建与 WL 模拟组卡。

use crate::search::SupportDeck;
use crate::types::{DefaultImage, FINAL_CHAPTER_EVENT_ID};

use super::build::enrich_master;
use super::gather::CardIntermediate;
use super::power::resolve_unit_mask;
use super::types::{
    self, GameData, WBSupportDeckBonus, default_image_kind, is_after_training, parse_unit_code,
    unit_to_pool_index,
};

use super::BuildError;
use super::event_bonus::EventContext;
use super::index;

pub struct WorldBloomSupportCard {
    pub card_id: i32,
    pub bonus: f64,
    pub skill_level: i32,
    pub master_rank: i32,
    pub level: i32,
    pub after_training: bool,
    pub default_image: DefaultImage,
}

/// Evaluate every owned card for a World Bloom support deck.
///
/// This operation deliberately does not build or mutate the DFS search pool.
pub fn world_bloom_support_cards(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
    support_master_max: bool,
    support_skill_max: bool,
    filter_other_unit: bool,
) -> Result<Vec<WorldBloomSupportCard>, BuildError> {
    let wb_event_id = resolve_wb_event_id(params)?;
    let event_id = wb_event_id.unwrap_or_else(|| params.event_id.unwrap_or_default());
    let turn = if wb_event_id.is_some() {
        Some(world_bloom_event_turn(event_id))
    } else {
        params.world_bloom_event_turn.or_else(|| {
            params.event_id.and_then(|event_id| {
                if event_id == FINAL_CHAPTER_EVENT_ID {
                    Some(2)
                } else if game
                    .world_blooms
                    .iter()
                    .any(|entry| entry.event_id == event_id)
                    || event_id > 1000
                {
                    // 真实 WL 章节按 id 区间覆盖 turn 1/2/3；模拟假活动 id
                    // （>1000）同样由统一函数换算。
                    Some(world_bloom_event_turn(event_id))
                } else {
                    None
                }
            })
        })
    };
    let Some(turn) = turn.filter(|turn| (1..=3).contains(turn)) else {
        return Err(BuildError::InvalidConfig(
            "world_bloom_event_turn or a World Bloom event_id is required".to_string(),
        ));
    };
    // 终章/模拟活动没有真实章节行，允许用 forced_leader_character_id 指定队长。
    let special_character_id = params
        .world_bloom_character_id
        .or_else(|| {
            if wb_event_id.is_some() {
                params.forced_leader_character_id
            } else {
                None
            }
        })
        .or_else(|| {
            params.event_id.and_then(|event_id| {
                game.world_blooms
                    .iter()
                    .find(|entry| entry.event_id == event_id)
                    .and_then(|entry| entry.game_character_id)
            })
        });
    let Some(special_character_id) = special_character_id.filter(|id| (1..=26).contains(id)) else {
        return Err(BuildError::InvalidConfig(
            "world_bloom_character_id is required".to_string(),
        ));
    };
    let synth = synthesize_wb_rows(game, event_id);
    let extra_limited = synth.support_limited_bonuses;

    let mut result = Vec::with_capacity(user.user_cards.len());
    for original in &user.user_cards {
        let Some(master) = game.cards.iter().find(|card| card.id == original.card_id) else {
            return Err(BuildError::InvalidConfig(format!(
                "support deck card not found for card_id={}",
                original.card_id
            )));
        };
        let master = enrich_master(master, game);
        let mut card = original.clone();
        if support_master_max {
            card.master_rank = master.max_master_rank.unwrap_or(card.master_rank);
        }
        if support_skill_max {
            card.skill_level = master.max_skill_level.unwrap_or(card.skill_level);
        }
        let unit_mask_raw = resolve_unit_mask(&master, game);
        let bonus = calc_wb_support_bonus(
            game,
            event_id,
            Some(turn),
            Some(special_character_id),
            master.id.clamp(0, u16::MAX as i32) as u16,
            master.card_rarity_type,
            master.character_id.clamp(0, u8::MAX as i32) as u8,
            unit_mask_raw,
            !filter_other_unit,
            card.master_rank,
            card.skill_level,
            &extra_limited,
        );
        result.push(WorldBloomSupportCard {
            card_id: card.card_id,
            bonus,
            skill_level: card.skill_level,
            master_rank: card.master_rank,
            level: card.level,
            after_training: is_after_training(&card.special_training_status),
            default_image: default_image_kind(&card.default_image),
        });
    }
    result.sort_by(|left, right| {
        right
            .bonus
            .total_cmp(&left.bonus)
            .then_with(|| left.card_id.cmp(&right.card_id))
    });
    Ok(result)
}

/// Slim per-card support-deck seed (deduped by card id).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SupportSeedSlim {
    pub(super) card_id: u16,
    pub(super) rarity: i32,
    pub(super) character_id: u8,
    pub(super) unit_mask: u8,
    pub(super) master_rank: i32,
    pub(super) skill_level: i32,
}

pub(crate) fn support_seed_from_intermediate(
    card: &CardIntermediate,
    indexes: &index::PoolIndexes,
    support_master_max: bool,
    support_skill_max: bool,
) -> SupportSeedSlim {
    let master = indexes
        .card_data(card.game_card_id)
        .map(|entry| &entry.master);
    let master_rank = if support_master_max {
        master
            .and_then(|master| master.max_master_rank)
            .unwrap_or(card.master_rank)
    } else {
        card.master_rank
    };
    let skill_level = if support_skill_max {
        master
            .and_then(|master| master.max_skill_level)
            .unwrap_or(card.skill_level)
    } else {
        card.skill_level
    };
    SupportSeedSlim {
        card_id: card.game_card_id.max(0).min(u16::MAX as i32) as u16,
        rarity: card.card_rarity_type,
        character_id: card.character_id,
        unit_mask: card.unit_mask_raw,
        master_rank,
        skill_level,
    }
}

/// Precomputed per-(event, turn, special-character) support bonus rate tables.
pub(super) struct SupportRateTables {
    valid: bool,
    special_character_id: i32,
    special_unit_mask: u8,
    row_present: [bool; 6],
    char_specific: [f64; 6],
    char_others: [f64; 6],
    mr_bonus: [[f64; 8]; 6],
    sl_bonus: [[f64; 8]; 6],
    limited_by_card: std::collections::HashMap<i32, f64>,
}

impl SupportRateTables {
    fn new(
        game: &types::GameData<'_>,
        event_id: i32,
        turn: Option<i32>,
        special_character_id: Option<i32>,
        extra_limited: &[types::WBSupportDeckUnitEventLimitedBonus],
    ) -> Self {
        let mut tables = Self {
            valid: false,
            special_character_id: 0,
            special_unit_mask: 0,
            row_present: [false; 6],
            char_specific: [0.0; 6],
            char_others: [0.0; 6],
            mr_bonus: [[0.0; 8]; 6],
            sl_bonus: [[0.0; 8]; 6],
            limited_by_card: std::collections::HashMap::new(),
        };
        let Some(special_character_id) = special_character_id.filter(|id| *id > 0) else {
            return tables;
        };
        let Some(special_unit) = game
            .game_character_units
            .iter()
            .find(|entry| entry.game_character_id == special_character_id)
            .and_then(|entry| parse_unit_code(&entry.unit))
            .and_then(types::unit_to_pool_index)
        else {
            return tables;
        };
        tables.valid = true;
        tables.special_character_id = special_character_id;
        tables.special_unit_mask = 1u8 << special_unit;

        let table = match turn {
            Some(1) => game.wb_support_deck_bonuses_wl1,
            Some(2) => game.wb_support_deck_bonuses_wl2,
            Some(3) => game.wb_support_deck_bonuses_wl3,
            _ => &[],
        };
        for rarity in 1..6usize {
            let Some(row) = table
                .iter()
                .find(|entry| support_rarity_matches(&entry.card_rarity_type, rarity as i32))
            else {
                continue;
            };
            tables.row_present[rarity] = true;
            tables.char_specific[rarity] = support_char_bonus(row, "specific");
            tables.char_others[rarity] = support_char_bonus(row, "others");
            for mr in 0..8i32 {
                tables.mr_bonus[rarity][mr as usize] = row
                    .world_bloom_support_deck_master_rank_bonuses
                    .iter()
                    .find(|entry| entry.master_rank == mr)
                    .map(|entry| entry.bonus_rate)
                    .unwrap_or(0.0);
            }
            for sl in 0..8i32 {
                tables.sl_bonus[rarity][sl as usize] = row
                    .world_bloom_support_deck_skill_level_bonuses
                    .iter()
                    .find(|entry| entry.skill_level == sl)
                    .map(|entry| entry.bonus_rate)
                    .unwrap_or(0.0);
            }
        }
        for bonus in game.world_bloom_support_deck_unit_event_limited_bonuses {
            if bonus.event_id == event_id && bonus.game_character_id == special_character_id {
                *tables.limited_by_card.entry(bonus.card_id).or_insert(0.0) += bonus.bonus_rate;
            }
        }
        for bonus in extra_limited {
            if bonus.game_character_id == special_character_id {
                *tables.limited_by_card.entry(bonus.card_id).or_insert(0.0) += bonus.bonus_rate;
            }
        }
        tables
    }

    #[inline]
    fn bonus(&self, seed: &SupportSeedSlim) -> f64 {
        if !self.valid {
            return 0.0;
        }
        if seed.unit_mask & self.special_unit_mask == 0 {
            return 0.0;
        }
        let rarity = seed.rarity;
        if !(1..6).contains(&rarity) || !self.row_present[rarity as usize] {
            return 0.0;
        }
        let rarity = rarity as usize;
        let mut total = if seed.character_id as i32 == self.special_character_id {
            self.char_specific[rarity]
        } else {
            self.char_others[rarity]
        };
        if (0..8).contains(&seed.master_rank) {
            total += self.mr_bonus[rarity][seed.master_rank as usize];
        }
        if (0..8).contains(&seed.skill_level) {
            total += self.sl_bonus[rarity][seed.skill_level as usize];
        }
        if let Some(limited) = self.limited_by_card.get(&(seed.card_id as i32)) {
            total += *limited;
        }
        if !total.is_finite() || total <= 0.0 {
            0.0
        } else {
            total
        }
    }
}

pub(super) fn support_rarity_matches(code: &str, card_rarity_type: i32) -> bool {
    let trimmed = code.trim();
    let matches_ascii = |target: &str| trimmed.eq_ignore_ascii_case(target);
    match card_rarity_type {
        1 => matches_ascii("rarity_1") || matches_ascii("1"),
        2 => matches_ascii("rarity_2") || matches_ascii("2"),
        3 => matches_ascii("rarity_3") || matches_ascii("3"),
        4 => matches_ascii("rarity_4") || matches_ascii("4"),
        5 => matches_ascii("rarity_birthday") || matches_ascii("birthday") || matches_ascii("5"),
        _ => false,
    }
}

pub(super) fn support_char_bonus(table: &types::WBSupportDeckBonus, character_type: &str) -> f64 {
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

pub(super) fn build_support_deck_fast(
    seeds: &[SupportSeedSlim],
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
    special_character_id: Option<i32>,
) -> SupportDeck {
    let Some(event_ctx) = event_ctx else {
        return SupportDeck::default();
    };
    if event_ctx.support_deck_count == 0 {
        return SupportDeck::default();
    }
    let special_character_id = special_character_id.or(event_ctx.world_bloom_character_id);
    let tables = SupportRateTables::new(
        game,
        event_ctx.event_id,
        event_ctx.world_bloom_event_turn,
        special_character_id,
        &event_ctx.support_limited_bonuses,
    );
    let mut cards: Vec<(u16, f64)> = Vec::with_capacity(seeds.len());
    for seed in seeds {
        cards.push((seed.card_id, tables.bonus(seed)));
    }
    cards.sort_by(|left, right| right.1.total_cmp(&left.1));
    SupportDeck {
        cards,
        count: event_ctx.support_deck_count,
    }
}

pub(super) fn build_final_chapter_support_decks_fast(
    seeds: &[SupportSeedSlim],
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> Vec<SupportDeck> {
    let mut decks = vec![SupportDeck::default(); 27];
    let Some(event_ctx) = event_ctx else {
        return decks;
    };
    // 终章为每个可能队长各建一个支援卡桶，搜索期按 deck leader 取用
    // 语义与游戏一致：终章支援不绑定章节角色，而是跟随卡组队长。
    for character_id in 1..=26 {
        decks[character_id as usize] =
            build_support_deck_fast(seeds, game, Some(event_ctx), Some(character_id));
    }
    decks
}

/// 计算单张卡的 World Bloom 支援加成。
///
/// `extra_limited`：模拟终章合成的 limited 加成行（真实活动传空切片）。
pub(crate) fn calc_wb_support_bonus(
    game: &GameData<'_>,
    event_id: i32,
    turn: Option<i32>,
    special_character_id: Option<i32>,
    game_card_id: u16,
    card_rarity_type: i32,
    character_id: u8,
    unit_mask_raw: u8,
    require_special_unit_match: bool,
    master_rank: i32,
    skill_level: i32,
    extra_limited: &[types::WBSupportDeckUnitEventLimitedBonus],
) -> f64 {
    let Some(special_character_id) = special_character_id.filter(|id| *id > 0) else {
        return 0.0;
    };
    let Some(special_unit) = game
        .game_character_units
        .iter()
        .find(|entry| entry.game_character_id == special_character_id)
        .and_then(|entry| parse_unit_code(&entry.unit))
        .and_then(unit_to_pool_index)
    else {
        return 0.0;
    };
    if require_special_unit_match && unit_mask_raw & (1u8 << special_unit) == 0 {
        return 0.0;
    }

    let Some(bonus_table) = support_bonus_table(game, turn)
        .iter()
        .find(|entry| rarity_matches(&entry.card_rarity_type, card_rarity_type))
    else {
        return 0.0;
    };

    let mut total = 0.0_f64;
    let character_type = if character_id as i32 == special_character_id {
        "specific"
    } else {
        "others"
    };
    total += find_character_bonus(bonus_table, character_type);
    total += bonus_table
        .world_bloom_support_deck_master_rank_bonuses
        .iter()
        .find(|entry| entry.master_rank == master_rank)
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0);
    total += bonus_table
        .world_bloom_support_deck_skill_level_bonuses
        .iter()
        .find(|entry| entry.skill_level == skill_level)
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0);

    for bonus in game.world_bloom_support_deck_unit_event_limited_bonuses {
        if bonus.event_id == event_id
            && bonus.game_character_id == special_character_id
            && bonus.card_id == game_card_id as i32
        {
            total += bonus.bonus_rate;
        }
    }
    for bonus in extra_limited {
        if bonus.game_character_id == special_character_id && bonus.card_id == game_card_id as i32 {
            total += bonus.bonus_rate;
        }
    }

    rate_to_f64(total)
}

fn support_bonus_table<'a>(game: &'a GameData<'_>, turn: Option<i32>) -> &'a [WBSupportDeckBonus] {
    match turn {
        Some(1) => game.wb_support_deck_bonuses_wl1,
        Some(2) => game.wb_support_deck_bonuses_wl2,
        Some(3) => game.wb_support_deck_bonuses_wl3,
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// World Bloom 模拟组卡
//
// WL 活动尚无 masterdata 行时（含模拟终章），在此合成 handler 构建所需的
// event / eventCards / eventDeckBonuses / worldBlooms / 支援 limited 加成行。
// 合成仅发生在 handler 构建阶段（每请求一次，O(表行数)），搜索热路径不感知。
// ---------------------------------------------------------------------------

/// WL3 分组角色表（每组 VS 打头）。
const WL3_PART_CHARACTER_IDS: [&[i32]; 5] = [
    &[21, 1, 6, 14, 17],
    &[22, 23, 4, 5, 10, 13],
    &[24, 3, 8, 9, 18],
    &[26, 2, 12, 16, 20],
    &[25, 7, 11, 15, 19],
];

/// 假 WL 活动 ID：`3_000_000 + (turn-1)*100_000 + group`。
///
/// turn 1/2 的 group 为团（Unit 编码 1-6），turn 3 的 group 为分组（1-5）；
/// group=0 的 3_100_000/3_200_000 分别对应 legacy/模拟终章。
const fn fake_wb_event_id(turn: i32, group: i32) -> i32 {
    3_000_000 + (turn - 1) * 100_000 + group
}

/// 活动所属 WL 回合。
pub(crate) const fn world_bloom_event_turn(event_id: i32) -> i32 {
    if event_id > 1000 {
        (event_id / 100_000) % 10 + 1
    } else if event_id <= 140 {
        1 // 事件 140 之前均为第一轮
    } else if event_id <= 180 {
        2
    } else {
        3
    }
}

/// 角色 → WL3 分组（1-5）；非 WL3 角色返回 `None`。
fn world_bloom_3_part_by_character_id(character_id: i32) -> Option<i32> {
    WL3_PART_CHARACTER_IDS
        .iter()
        .position(|part| part.contains(&character_id))
        .map(|index| index as i32 + 1)
}

/// 解析模拟 WL 参数为假活动 ID；真实 `event_id` 优先。
///
/// - `world_bloom_finale_turn`：2 → legacy 终章 180；3 → 模拟终章 3_200_000；
/// - `world_bloom_event_turn=3`：要求 `world_bloom_character_id`，按分组出卡；
/// - `world_bloom_event_turn=1/2`：要求 `event_unit`，按团出卡。
pub(super) fn resolve_wb_event_id(params: &types::BuildParams) -> Result<Option<i32>, BuildError> {
    if params.event_id.is_some() {
        return Ok(None); // 真实活动优先，模拟参数只作辅助
    }
    if matches!(
        params.live_type,
        crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
    ) {
        return Ok(None); // 挑战 live 不参与模拟
    }
    if let Some(turn) = params.world_bloom_finale_turn {
        return match turn {
            2 => Ok(Some(crate::types::FINAL_CHAPTER_EVENT_ID)),
            3 => Ok(Some(crate::types::WL3_FAKE_FINALE_EVENT_ID)),
            other => Err(BuildError::InvalidConfig(format!(
                "world_bloom_finale_turn 仅支持 2 或 3，当前 {other}"
            ))),
        };
    }
    if let Some(turn) = params.world_bloom_event_turn {
        return match turn {
            3 => {
                let character_id = params.world_bloom_character_id.ok_or_else(|| {
                    BuildError::InvalidConfig(
                        "world_bloom_event_turn=3 需要 world_bloom_character_id（按 WL3 分组模拟）"
                            .to_string(),
                    )
                })?;
                if !(1..=26).contains(&character_id) {
                    return Err(BuildError::InvalidConfig(format!(
                        "world_bloom_character_id 非法: {character_id}"
                    )));
                }
                let part = world_bloom_3_part_by_character_id(character_id).ok_or_else(|| {
                    BuildError::InvalidConfig(format!("角色 {character_id} 不属于任何 WL3 分组"))
                })?;
                Ok(Some(fake_wb_event_id(3, part)))
            }
            1 | 2 => {
                let unit = params
                    .event_unit
                    .as_deref()
                    .and_then(parse_unit_code)
                    .ok_or_else(|| {
                        BuildError::InvalidConfig(
                            "world_bloom_event_turn=1/2 需要 event_unit（按团模拟）".to_string(),
                        )
                    })?;
                let group = unit as i32;
                if !(1..=6).contains(&group) {
                    return Err(BuildError::InvalidConfig(format!(
                        "event_unit 非法: {unit:?}"
                    )));
                }
                Ok(Some(fake_wb_event_id(turn, group)))
            }
            other => Err(BuildError::InvalidConfig(format!(
                "world_bloom_event_turn 仅支持 1-3，当前 {other}"
            ))),
        };
    }
    Ok(None)
}

/// 模拟活动合成出的 masterdata 行集合（等价于把同名行插入主数据；
/// 本引擎保持 `GameData` 只读，改为随 `EventContext` 透传）。
#[derive(Debug, Default, Clone)]
pub(crate) struct SynthWbRows {
    pub event_cards: Vec<types::EventCard>,
    pub deck_bonuses: Vec<types::EventDeckBonus>,
    pub world_blooms: Vec<types::WorldBloom>,
    pub honor_bonuses: Vec<types::EventHonorBonus>,
    pub support_limited_bonuses: Vec<types::WBSupportDeckUnitEventLimitedBonus>,
}

/// 按假活动 ID 合成模拟行；真实活动（masterdata 已有活动行）返回空集合。
pub(crate) fn synthesize_wb_rows(game: &types::GameData<'_>, event_id: i32) -> SynthWbRows {
    if game.events.iter().any(|event| event.id == event_id) {
        return SynthWbRows::default(); // 真实活动：不合成
    }
    if event_id == crate::types::WL3_FAKE_FINALE_EVENT_ID {
        return synthesize_wb_finale_rows(game);
    }
    if event_id == crate::types::FINAL_CHAPTER_EVENT_ID {
        return synthesize_wb_legacy_rows(game);
    }
    let turn = world_bloom_event_turn(event_id);
    let group = event_id % 100_000;
    let valid_group = match turn {
        1 | 2 => (1..=6).contains(&group),
        3 => (1..=5).contains(&group),
        _ => false,
    };
    if !valid_group {
        return SynthWbRows::default();
    }
    synthesize_wb_turn_rows(game, turn, group, event_id)
}

/// WL1/2 按团、WL3 按分组的 25% 模拟活动。
fn synthesize_wb_turn_rows(
    game: &types::GameData<'_>,
    turn: i32,
    group: i32,
    event_id: i32,
) -> SynthWbRows {
    let mut rows = SynthWbRows::default();

    // 成员集合：turn1/2 按团（VS 只归 piapro 团），turn3 按分组表。
    let mut members: Vec<i32> = game
        .game_character_units
        .iter()
        .filter(|entry| {
            if turn == 3 {
                WL3_PART_CHARACTER_IDS[(group - 1) as usize].contains(&entry.game_character_id)
            } else {
                let unit = parse_unit_code(&entry.unit);
                let in_group_unit =
                    unit.is_some_and(|unit| unit as i32 == group) && entry.game_character_id <= 20;
                let in_piapro = group == crate::types::Unit::Piapro as i32
                    && unit.is_some_and(|unit| unit == crate::types::Unit::Piapro)
                    && entry.game_character_id > 20;
                in_group_unit || in_piapro
            }
        })
        .map(|entry| entry.game_character_id)
        .filter(|id| (1..=26).contains(id))
        .collect();
    members.sort_unstable();
    members.dedup();

    // 同团角色加成 25%。
    for character_id in &members {
        rows.deck_bonuses.push(types::EventDeckBonus {
            event_id,
            character_id: Some(*character_id),
            unit: None,
            attr: None,
            bonus_rate_x10: 250,
        });
    }

    // WL 章节行（按角色 ID 升序分配 chapterNo，从 1 起）。
    for (index, character_id) in members.iter().enumerate() {
        rows.world_blooms.push(types::WorldBloom {
            event_id,
            game_character_id: Some(*character_id),
            chapter_no: index as i32 + 1,
            world_bloom_chapter_type: None,
        });
    }

    // turn>=2：把前几轮 WL 的限定卡作为支援额外加成卡。
    if turn >= 2 {
        rows.support_limited_bonuses = synth_support_limited(game, turn, event_id, &members);
    }

    rows
}

/// legacy WL2 终章（180）兜底合成（仅在 masterdata 缺 180 时生效）。
fn synthesize_wb_legacy_rows(game: &types::GameData<'_>) -> SynthWbRows {
    let mut rows = SynthWbRows::default();

    // 全角色 5% deck bonus。
    push_all_character_deck_bonuses(game, crate::types::FINAL_CHAPTER_EVENT_ID, 50, &mut rows);

    // WL2 六场活动的限定卡按 25% 挂到终章。
    const WL2_EVENT_IDS: [i32; 6] = [163, 167, 170, 171, 176, 179];
    let mut card_ids: Vec<i32> = game
        .event_cards
        .iter()
        .filter(|entry| WL2_EVENT_IDS.contains(&entry.event_id))
        .map(|entry| entry.card_id)
        .collect();
    card_ids.sort_unstable();
    card_ids.dedup();
    for card_id in card_ids {
        rows.event_cards.push(types::EventCard {
            event_id: crate::types::FINAL_CHAPTER_EVENT_ID,
            card_id,
            bonus_rate_x10: 250,
            leader_bonus_rate_x10: 0,
        });
    }

    // 支援 limited 加成整表复制。
    rows.support_limited_bonuses = game
        .world_bloom_support_deck_unit_event_limited_bonuses
        .iter()
        .map(|bonus| types::WBSupportDeckUnitEventLimitedBonus {
            event_id: crate::types::FINAL_CHAPTER_EVENT_ID,
            ..bonus.clone()
        })
        .collect();

    // 终章章节行。
    rows.world_blooms.push(types::WorldBloom {
        event_id: crate::types::FINAL_CHAPTER_EVENT_ID,
        game_character_id: None,
        chapter_no: 1,
        world_bloom_chapter_type: Some("finale".to_string()),
    });

    rows
}

/// 模拟 WL3 终章（3_200_000）
/// （WL3 终章尚未有真实 masterdata 数据）。
///
/// 规则：全角色 5% deck bonus；WL3 各组章节的限定卡按 25%（队长额外 20%）
/// 挂到终章；WL1/2 限定卡按 20% 进入支援 limited 加成；
/// `wl_3rd` 排行称号（top-1000）按 50% 合成队长荣誉加成。
fn synthesize_wb_finale_rows(game: &types::GameData<'_>) -> SynthWbRows {
    let mut rows = SynthWbRows::default();
    let event_id = crate::types::WL3_FAKE_FINALE_EVENT_ID;

    // 源活动：尚未有终章章节的真实 WL3 活动。
    let source_event_ids: Vec<i32> = game
        .events
        .iter()
        .filter(|event| {
            event.id < 1000
                && types::parse_event_type(&event.event_type)
                    == Some(crate::types::EventType::WorldBloom)
                && world_bloom_event_turn(event.id) == 3
                && !game.world_blooms.iter().any(|world_bloom| {
                    world_bloom.event_id == event.id
                        && world_bloom.world_bloom_chapter_type.as_deref() == Some("finale")
                })
        })
        .map(|event| event.id)
        .collect();
    if source_event_ids.is_empty() {
        return rows; // 无源活动时只落地空终章事件
    }

    // 全角色 5% deck bonus（含 VS）。
    push_all_character_deck_bonuses(game, event_id, 50, &mut rows);

    // 源活动限定卡 → 25%（队长 20%），按卡去重。
    let mut seen_cards: std::collections::BTreeSet<i32> = Default::default();
    for entry in game.event_cards {
        if !source_event_ids.contains(&entry.event_id)
            || entry.bonus_rate_x10 <= 0
            || !seen_cards.insert(entry.card_id)
        {
            continue;
        }
        rows.event_cards.push(types::EventCard {
            event_id,
            card_id: entry.card_id,
            bonus_rate_x10: 250,
            leader_bonus_rate_x10: 200,
        });
    }

    // 支援 limited：WL1/2 全部限定卡按 20%。
    let all_characters: Vec<i32> = game
        .game_character_units
        .iter()
        .map(|entry| entry.game_character_id)
        .filter(|id| (1..=26).contains(id))
        .collect();
    rows.support_limited_bonuses = synth_support_limited(game, 3, event_id, &all_characters);

    // WL3 排行称号 → 50% 队长荣誉加成（每枚称号命中一个 leader 角色）。
    for honor in game.honors {
        let Some(asset_bundle_name) = honor.asset_bundle_name.as_deref() else {
            continue;
        };
        let Some((part, chapter)) = parse_wl3_rank_honor(asset_bundle_name) else {
            continue;
        };
        let leader = game
            .world_blooms
            .iter()
            .find(|world_bloom| {
                source_event_ids.contains(&world_bloom.event_id)
                    && world_bloom.chapter_no == chapter
                    && world_bloom_3_part_by_character_id(
                        world_bloom.game_character_id.unwrap_or(0),
                    ) == Some(part)
            })
            .and_then(|world_bloom| world_bloom.game_character_id);
        let Some(leader) = leader else {
            continue;
        };
        rows.honor_bonuses.push(types::EventHonorBonus {
            event_id,
            honor_id: honor.id,
            leader_game_character_id: leader,
            bonus_rate: 50,
        });
    }

    // 终章章节行。
    rows.world_blooms.push(types::WorldBloom {
        event_id,
        game_character_id: None,
        chapter_no: 1,
        world_bloom_chapter_type: Some("finale".to_string()),
    });

    rows
}

/// 全角色统一 deck bonus（终章 5% 规则）。
fn push_all_character_deck_bonuses(
    game: &types::GameData<'_>,
    event_id: i32,
    bonus_rate_x10: i32,
    rows: &mut SynthWbRows,
) {
    for entry in game.game_character_units {
        rows.deck_bonuses.push(types::EventDeckBonus {
            event_id,
            character_id: Some(entry.game_character_id),
            unit: None,
            attr: None,
            bonus_rate_x10,
        });
    }
}

/// 支援 limited 加成：
/// - turn 2：复制真实 WL2 活动的 limited 行（保留原始加成率）；
/// - turn 3：WL1/2 全部限定卡按 20% 合成，按（角色, 卡）去重。
fn synth_support_limited(
    game: &types::GameData<'_>,
    turn: i32,
    fake_event_id: i32,
    characters: &[i32],
) -> Vec<types::WBSupportDeckUnitEventLimitedBonus> {
    if turn == 2 {
        return game
            .world_bloom_support_deck_unit_event_limited_bonuses
            .iter()
            .filter(|bonus| {
                bonus.event_id != crate::types::FINAL_CHAPTER_EVENT_ID
                    && world_bloom_event_turn(bonus.event_id) == 2
                    && characters.contains(&bonus.game_character_id)
            })
            .map(|bonus| types::WBSupportDeckUnitEventLimitedBonus {
                event_id: fake_event_id,
                ..bonus.clone()
            })
            .collect();
    }
    if turn != 3 {
        return Vec::new();
    }

    let mut used: std::collections::BTreeSet<(i32, i32)> = Default::default();
    let mut bonuses = Vec::new();
    for event_card in game.event_cards {
        if event_card.event_id == crate::types::FINAL_CHAPTER_EVENT_ID
            || world_bloom_event_turn(event_card.event_id) > 2
            || event_card.bonus_rate_x10 <= 0
        {
            continue;
        }
        // 仅真实 WL 活动（同时校验 eventType == world_bloom）。
        let is_world_bloom_event = game.events.iter().any(|event| {
            event.id == event_card.event_id
                && types::parse_event_type(&event.event_type)
                    == Some(crate::types::EventType::WorldBloom)
        });
        if !is_world_bloom_event {
            continue;
        }
        let Some(character_id) = game
            .cards
            .iter()
            .find(|card| card.id == event_card.card_id)
            .map(|card| card.character_id)
        else {
            continue;
        };
        if !characters.contains(&character_id) || !used.insert((character_id, event_card.card_id)) {
            continue;
        }
        bonuses.push(types::WBSupportDeckUnitEventLimitedBonus {
            event_id: fake_event_id,
            game_character_id: character_id,
            card_id: event_card.card_id,
            bonus_rate: 20.0,
        });
    }
    bonuses
}

/// 解析 WL3 排行称号 `honor_top_{rank}_event_wl_3rd_part{part}_cp{chapter}...`，
/// 返回 `(part, chapter)`；rank 需在 1-1000 内
/// （名称形如 `honor_top_{rank}_event_wl_3rd_part{part}_cp{chapter}`）。
fn parse_wl3_rank_honor(asset_bundle_name: &str) -> Option<(i32, i32)> {
    let rest = asset_bundle_name.strip_prefix("honor_top_")?;
    let rank_text = rest.split("_event_wl_").next()?;
    let rank: i32 = rank_text.parse().ok()?;
    if !(1..=1000).contains(&rank) {
        return None;
    }
    let part_start = asset_bundle_name.find("wl_3rd_part")? + "wl_3rd_part".len();
    let chapter_marker = asset_bundle_name[part_start..].find("_cp")? + part_start;
    let part: i32 = asset_bundle_name[part_start..chapter_marker].parse().ok()?;
    let chapter_text: String = asset_bundle_name[chapter_marker + 3..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let chapter: i32 = chapter_text.parse().ok()?;
    Some((part, chapter))
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

fn rate_to_f64(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FINAL_CHAPTER_EVENT_ID, LiveType, WL3_FAKE_FINALE_EVENT_ID};

    /// 构造只含 WL 模拟所需行的 GameData（其余字段为空切片）。
    struct WbFixture {
        game_character_units: Vec<types::GameCharacterUnit>,
        events: Vec<types::Event>,
        event_cards: Vec<types::EventCard>,
        world_blooms: Vec<types::WorldBloom>,
        honors: Vec<types::Honor>,
        limited: Vec<types::WBSupportDeckUnitEventLimitedBonus>,
        cards: Vec<types::MasterCard>,
    }

    impl WbFixture {
        fn game(&self) -> types::GameData<'_> {
            types::GameData {
                cards: &self.cards,
                card_parameters: &[],
                card_rarities: &[],
                card_episodes: &[],
                master_lessons: &[],
                skills: &[],
                skill_effects: &[],
                area_item_levels: &[],
                game_character_units: &self.game_character_units,
                character_ranks: &[],
                card_mysekai_canvas_bonuses: &[],
                mysekai_gates: &[],
                mysekai_gate_levels: &[],
                events: &self.events,
                event_cards: &self.event_cards,
                event_deck_bonuses: &[],
                event_rarity_bonus_rates: &[],
                event_honor_bonuses: &[],
                event_card_bonus_limits: &[],
                world_bloom_different_attribute_bonuses: &[],
                world_blooms: &self.world_blooms,
                wb_support_deck_bonuses_wl1: &[],
                wb_support_deck_bonuses_wl2: &[],
                wb_support_deck_bonuses_wl3: &[],
                world_bloom_support_deck_unit_event_limited_bonuses: &self.limited,
                event_mysekai_fixture_performance_bonus_limits: &[],
                event_skill_score_up_limits: &[],
                music_metas: &[],
                music_difficulties: &[],
                honors: &self.honors,
                bonds_honors: &[],
            }
        }
    }

    fn unit(code: &str, character_id: i32) -> types::GameCharacterUnit {
        types::GameCharacterUnit {
            game_character_id: character_id,
            unit: code.to_string(),
        }
    }

    fn event_card(card_id: i32, event_id: i32, bonus: i32) -> types::EventCard {
        types::EventCard {
            event_id,
            card_id,
            bonus_rate_x10: bonus * 10,
            leader_bonus_rate_x10: 0,
        }
    }

    fn master_card(card_id: i32, character_id: i32) -> types::MasterCard {
        types::MasterCard {
            id: card_id,
            character_id,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: format!("card_{card_id:06}_normal"),
            skill_id: 1,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: None,
            max_skill_level: None,
            max_master_rank: None,
        }
    }

    fn honor(id: i32, name: &str) -> types::Honor {
        types::Honor {
            id,
            levels: Vec::new(),
            asset_bundle_name: Some(name.to_string()),
        }
    }

    fn limited(
        event_id: i32,
        character_id: i32,
        card_id: i32,
        rate: f64,
    ) -> types::WBSupportDeckUnitEventLimitedBonus {
        types::WBSupportDeckUnitEventLimitedBonus {
            event_id,
            game_character_id: character_id,
            card_id,
            bonus_rate: rate,
        }
    }

    /// JP 形态的 WL3 数据：三场 WL3 源活动 + WL1/2 限定卡 + 排行称号。
    fn jp_like_fixture() -> WbFixture {
        WbFixture {
            game_character_units: vec![
                unit("light_sound", 1),
                unit("idol", 2),
                unit("street", 3),
                unit("theme_park", 4),
                unit("school_refusal", 5),
                unit("piapro", 21),
            ],
            events: vec![
                types::Event {
                    id: 100,
                    event_type: "marathon".to_string(),
                },
                types::Event {
                    id: 118,
                    event_type: "world_bloom".to_string(),
                },
                types::Event {
                    id: 170,
                    event_type: "world_bloom".to_string(),
                },
                types::Event {
                    id: 179,
                    event_type: "world_bloom".to_string(),
                },
                types::Event {
                    id: 202,
                    event_type: "world_bloom".to_string(),
                },
                types::Event {
                    id: 205,
                    event_type: "world_bloom".to_string(),
                },
                types::Event {
                    id: 207,
                    event_type: "world_bloom".to_string(),
                },
            ],
            event_cards: vec![
                event_card(1, 118, 20), // WL1 限定卡
                event_card(2, 170, 25), // WL2 限定卡
                event_card(3, 202, 25), // WL3 限定卡（源）
                event_card(3, 205, 30), // 同卡重复出现在另一场 WL3 → 去重
                event_card(4, 207, 0),  // bonus 0 → 不收
                event_card(5, 100, 25), // 非 WL 活动 → 不收
            ],
            world_blooms: vec![
                chapter(202, 1, 1), // char1, cp1, part1
                chapter(202, 4, 2),
                chapter(205, 3, 1),
                chapter(170, 4, 1),
            ],
            honors: vec![
                honor(9001, "honor_top_000100_event_wl_3rd_part1_cp1"),
                honor(9002, "honor_top_001000_event_wl_3rd_part2_cp2"),
                honor(9003, "honor_top_002000_event_wl_3rd_part1_cp1"), // rank>1000
                honor(9004, "honor_top_000500_event_wl_2nd_part1_cp1"), // 非 wl_3rd
            ],
            limited: vec![limited(170, 2, 2, 15.0)],
            cards: vec![
                master_card(1, 1),
                master_card(2, 2),
                master_card(3, 1),
                master_card(4, 21),
                master_card(5, 3),
            ],
        }
    }

    fn chapter(event_id: i32, character_id: i32, chapter_no: i32) -> types::WorldBloom {
        types::WorldBloom {
            event_id,
            game_character_id: Some(character_id),
            chapter_no,
            world_bloom_chapter_type: Some("game_character".to_string()),
        }
    }

    fn finale_params() -> types::BuildParams {
        types::BuildParams {
            world_bloom_finale_turn: Some(3),
            world_bloom_character_id: Some(1),
            live_type: LiveType::Solo,
            ..Default::default()
        }
    }

    #[test]
    fn world_bloom_event_turn_follows_upstream_id_scheme() {
        assert_eq!(world_bloom_event_turn(112), 1);
        assert_eq!(world_bloom_event_turn(140), 1);
        assert_eq!(world_bloom_event_turn(163), 2);
        assert_eq!(world_bloom_event_turn(179), 2);
        assert_eq!(world_bloom_event_turn(180), 2);
        assert_eq!(world_bloom_event_turn(202), 3);
        assert_eq!(world_bloom_event_turn(3_000_001), 1);
        assert_eq!(world_bloom_event_turn(3_100_002), 2);
        assert_eq!(world_bloom_event_turn(WL3_FAKE_FINALE_EVENT_ID), 3);
    }

    #[test]
    fn wl3_part_table_matches_upstream() {
        assert_eq!(world_bloom_3_part_by_character_id(21), Some(1));
        assert_eq!(world_bloom_3_part_by_character_id(1), Some(1));
        assert_eq!(world_bloom_3_part_by_character_id(23), Some(2));
        assert_eq!(world_bloom_3_part_by_character_id(26), Some(4));
        assert_eq!(world_bloom_3_part_by_character_id(25), Some(5));
        assert_eq!(world_bloom_3_part_by_character_id(27), None);
    }

    #[test]
    fn resolve_maps_finale_turn_to_legacy_and_fake_events() {
        let mut params = finale_params();
        params.world_bloom_finale_turn = Some(2);
        assert_eq!(
            resolve_wb_event_id(&params).unwrap(),
            Some(FINAL_CHAPTER_EVENT_ID)
        );
        params.world_bloom_finale_turn = Some(3);
        assert_eq!(
            resolve_wb_event_id(&params).unwrap(),
            Some(WL3_FAKE_FINALE_EVENT_ID)
        );
        params.world_bloom_finale_turn = Some(4);
        assert!(resolve_wb_event_id(&params).is_err());
    }

    #[test]
    fn resolve_requires_unit_for_low_turns_and_character_for_turn_three() {
        let mut params = finale_params();
        params.world_bloom_finale_turn = None;
        params.world_bloom_event_turn = Some(2);
        assert!(resolve_wb_event_id(&params).is_err());
        params.event_unit = Some("piapro".to_string());
        assert_eq!(resolve_wb_event_id(&params).unwrap(), Some(3_100_006));

        params.event_unit = None;
        params.world_bloom_character_id = None;
        params.world_bloom_event_turn = Some(3);
        assert!(resolve_wb_event_id(&params).is_err());
        params.world_bloom_character_id = Some(1); // 第 1 组
        assert_eq!(resolve_wb_event_id(&params).unwrap(), Some(3_200_001));
    }

    #[test]
    fn real_event_id_takes_priority_over_simulation() {
        let mut params = finale_params();
        params.event_id = Some(180);
        assert_eq!(resolve_wb_event_id(&params).unwrap(), None);
    }

    #[test]
    fn wl3_rank_honor_parsing_matches_upstream_names() {
        assert_eq!(
            parse_wl3_rank_honor("honor_top_000100_event_wl_3rd_part1_cp1"),
            Some((1, 1))
        );
        assert_eq!(
            parse_wl3_rank_honor("honor_top_001000_event_wl_3rd_part3_cp2"),
            Some((3, 2))
        );
        // rank > 1000 不参与
        assert_eq!(
            parse_wl3_rank_honor("honor_top_002000_event_wl_3rd_part1_cp1"),
            None
        );
        // 非 WL3 称号
        assert_eq!(
            parse_wl3_rank_honor("honor_top_000500_event_wl_2nd_part1_cp1"),
            None
        );
        assert_eq!(parse_wl3_rank_honor("honor_world_link_part1"), None);
    }

    #[test]
    fn finale_synthesis_follows_upstream_rules() {
        let fixture = jp_like_fixture();
        let rows = synthesize_wb_rows(&fixture.game(), WL3_FAKE_FINALE_EVENT_ID);

        // 源活动 = 202/205/207（170/179 是 WL2、118 是 WL1）。
        // 事件卡：card3 去重后 1 张、card4 bonus=0 剔除、card5 非 WL 剔除；
        // 25% 加成 + 队长 20%。
        assert_eq!(rows.event_cards.len(), 1);
        assert_eq!(rows.event_cards[0].card_id, 3);
        assert_eq!(rows.event_cards[0].bonus_rate_x10, 250);
        assert_eq!(rows.event_cards[0].leader_bonus_rate_x10, 200);

        // 全角色（fixture 的 6 个 unit 行）5% deck bonus。
        assert_eq!(rows.deck_bonuses.len(), 6);
        assert!(
            rows.deck_bonuses
                .iter()
                .all(|bonus| bonus.bonus_rate_x10 == 50 && bonus.attr.is_none())
        );

        // 支援 limited：WL1/2 的 card2（WL2）20%；card1 属 WL1 也应 20%。
        // fixture 中 WL1 的 card1 在 eventCards 里 bonus=20>0，故 1/2 两张。
        assert_eq!(rows.support_limited_bonuses.len(), 2);
        assert!(
            rows.support_limited_bonuses
                .iter()
                .all(|bonus| bonus.bonus_rate == 20.0)
        );

        // 荣誉：9001 → (part1, cp1) → 角色 1；9002 → (part2, cp2) → 角色 4（part2 成员）；
        // 9003 rank 超限、9004 非 wl_3rd。
        assert_eq!(rows.honor_bonuses.len(), 2);
        assert_eq!(rows.honor_bonuses[0].honor_id, 9001);
        assert_eq!(rows.honor_bonuses[0].leader_game_character_id, 1);
        assert_eq!(rows.honor_bonuses[0].bonus_rate, 50);
        assert_eq!(rows.honor_bonuses[1].leader_game_character_id, 4);

        // 终章章节行。
        assert_eq!(rows.world_blooms.len(), 1);
        assert_eq!(
            rows.world_blooms[0].world_bloom_chapter_type.as_deref(),
            Some("finale")
        );
    }

    #[test]
    fn real_events_are_never_synthesized() {
        let fixture = jp_like_fixture();
        assert!(
            synthesize_wb_rows(&fixture.game(), 202)
                .support_limited_bonuses
                .is_empty()
        );
        assert!(
            synthesize_wb_rows(&fixture.game(), 202)
                .event_cards
                .is_empty()
        );
    }

    #[test]
    fn legacy_finale_synthesis_copies_wl2_limited_rows() {
        let fixture = jp_like_fixture();
        // 去掉真实 180 活动行（fixture 本来就没有），确保 legacy 合成生效。
        let game = fixture.game();
        let rows = synthesize_wb_rows(&game, FINAL_CHAPTER_EVENT_ID);
        assert_eq!(rows.deck_bonuses.len(), 6);
        assert!(
            rows.deck_bonuses
                .iter()
                .all(|bonus| bonus.bonus_rate_x10 == 50)
        );
        // WL2 集合 {163,167,170,171,176,179} 中 fixture 只有 170 的 card2。
        assert_eq!(
            rows.event_cards,
            vec![types::EventCard {
                event_id: FINAL_CHAPTER_EVENT_ID,
                card_id: 2,
                bonus_rate_x10: 250,
                leader_bonus_rate_x10: 0,
            }]
        );
        // 支援 limited 整表复制且改写 event_id。
        assert_eq!(rows.support_limited_bonuses.len(), fixture.limited.len());
        assert!(
            rows.support_limited_bonuses
                .iter()
                .all(|bonus| bonus.event_id == FINAL_CHAPTER_EVENT_ID)
        );
    }
}
