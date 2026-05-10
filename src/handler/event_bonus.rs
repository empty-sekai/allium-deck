use crate::pool::EventBonusHot;
use crate::types::{Attr, EventType, FINAL_CHAPTER_EVENT_ID};

use super::types::{
    parse_attr_code, parse_unit_code, resolve_event_type, BuildParams, EventCard,
    EventCardBonusLimit, EventDeckBonus, EventHonorBonus, EventRarityBonusRate,
    EventSkillScoreUpLimit, GameData, MasterCard, UserCard, UserProfile, WorldBloomDiffAttrBonus,
};

/// Handler 构建阶段使用的活动上下文。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventContext {
    /// 活动 ID。
    pub event_id: i32,
    /// 活动类型。
    pub event_type: EventType,
    /// 当期卡表。
    pub event_cards: Vec<EventCard>,
    /// deck bonus 规则。
    pub deck_bonuses: Vec<EventDeckBonus>,
    /// 稀有度 bonus 规则。
    pub rarity_bonuses: Vec<EventRarityBonusRate>,
    /// leader honor bonus 规则。
    pub honor_bonuses: Vec<EventHonorBonus>,
    /// 技能上限。
    pub skill_score_up_limit: Option<u32>,
    /// limited bonus 生效张数上限。
    pub card_bonus_count_limit: usize,
    /// World Bloom 异色加成。
    pub diff_attr_bonus: [u16; 6],
    /// 支援 deck 取用数量。
    pub support_deck_count: u8,
    /// World Bloom 章节角色 ID。
    pub world_bloom_character_id: Option<i32>,
    /// World Bloom 回合。
    pub world_bloom_event_turn: Option<i32>,
    /// 自定义活动角色集合。
    pub custom_character_ids: Vec<i32>,
    /// 自定义活动属性。
    pub custom_attr: Option<Attr>,
}

fn load_diff_attr_bonus(table: &[WorldBloomDiffAttrBonus]) -> [u16; 6] {
    let mut result = [0u16; 6];
    for entry in table {
        if (1..=5).contains(&entry.attr_count) {
            result[entry.attr_count as usize] = entry.bonus_rate.max(0) as u16;
        }
    }
    result
}

fn load_card_bonus_limit(table: &[EventCardBonusLimit], event_id: i32) -> usize {
    table
        .iter()
        .find(|entry| entry.event_id == event_id)
        .map(|entry| entry.member_count_limit.max(1) as usize)
        .unwrap_or(5)
}

fn load_skill_limit(table: &[EventSkillScoreUpLimit], event_id: i32) -> Option<u32> {
    table
        .iter()
        .find(|entry| entry.event_id == event_id)
        .map(|entry| entry.score_up_limit.max(0) as u32)
}

fn resolve_skill_limit(game: &GameData<'_>, params: &BuildParams, event_id: i32) -> Option<u32> {
    if event_id == FINAL_CHAPTER_EVENT_ID
        && !matches!(
            params.live_type,
            crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
        )
    {
        // C++ moe base-deck-recommend.cpp hard-caps Final Chapter live skills at 140.
        return Some(140);
    }
    load_skill_limit(game.event_skill_score_up_limits, event_id)
}

fn load_support_deck_count(turn: Option<i32>, event_type: EventType) -> u8 {
    if !matches!(event_type, EventType::WorldBloom) {
        return 0;
    }
    match turn {
        Some(1) => 12,
        Some(2) => 20,
        Some(3) => 25,
        _ => 25,
    }
}

fn resolve_world_bloom_event_turn(game: &GameData<'_>, params: &BuildParams) -> Option<i32> {
    if params.world_bloom_event_turn.is_some() {
        return params.world_bloom_event_turn;
    }
    let event_id = params.event_id?;
    if event_id > 1000 {
        return Some((event_id / 100_000) % 10 + 1);
    }
    if event_id == FINAL_CHAPTER_EVENT_ID {
        return Some(2);
    }
    if game
        .world_blooms
        .iter()
        .any(|entry| entry.event_id == event_id)
    {
        return Some(if event_id <= 140 { 1 } else { 2 });
    }
    None
}

fn custom_character_ids(game: &GameData<'_>, unit_code: Option<&str>) -> Vec<i32> {
    let Some(unit) = unit_code.and_then(parse_unit_code) else {
        return Vec::new();
    };
    game.game_character_units
        .iter()
        .filter(|entry| parse_unit_code(&entry.unit) == Some(unit))
        .map(|entry| entry.game_character_id)
        .collect()
}

/// 构建活动上下文。
pub(crate) fn build_event_context(
    game: &GameData<'_>,
    params: &BuildParams,
) -> Option<EventContext> {
    let event_type = resolve_event_type(game, params).or_else(|| {
        if params.event_unit.is_some() || params.event_attr.is_some() {
            Some(EventType::Marathon)
        } else {
            None
        }
    })?;
    let event_id = params.event_id.unwrap_or_default();
    let world_bloom_event_turn = if matches!(event_type, EventType::WorldBloom) {
        resolve_world_bloom_event_turn(game, params)
    } else {
        params.world_bloom_event_turn
    };

    Some(EventContext {
        event_id,
        event_type,
        event_cards: game
            .event_cards
            .iter()
            .filter(|entry| entry.event_id == event_id)
            .cloned()
            .collect(),
        deck_bonuses: game
            .event_deck_bonuses
            .iter()
            .filter(|entry| entry.event_id == event_id)
            .cloned()
            .collect(),
        rarity_bonuses: game
            .event_rarity_bonus_rates
            .iter()
            .filter(|entry| entry.event_id == event_id)
            .cloned()
            .collect(),
        honor_bonuses: game
            .event_honor_bonuses
            .iter()
            .filter(|entry| entry.event_id == event_id)
            .cloned()
            .collect(),
        skill_score_up_limit: resolve_skill_limit(game, params, event_id),
        card_bonus_count_limit: load_card_bonus_limit(game.event_card_bonus_limits, event_id),
        diff_attr_bonus: if matches!(event_type, EventType::WorldBloom) {
            load_diff_attr_bonus(game.world_bloom_different_attribute_bonuses)
        } else {
            [0; 6]
        },
        support_deck_count: load_support_deck_count(world_bloom_event_turn, event_type),
        world_bloom_character_id: params.world_bloom_character_id,
        world_bloom_event_turn,
        custom_character_ids: custom_character_ids(game, params.event_unit.as_deref()),
        custom_attr: params.event_attr.as_deref().and_then(parse_attr_code),
    })
}

fn card_matches_rule(master: &MasterCard, rule: &EventDeckBonus, game: &GameData<'_>) -> bool {
    let character_ok = rule
        .character_id
        .is_none_or(|character_id| character_id == master.character_id);
    let attr_ok = rule
        .attr
        .as_deref()
        .and_then(parse_attr_code)
        .is_none_or(|attr| parse_attr_code(&master.attr) == Some(attr));
    let unit_ok = match rule.unit.as_deref().and_then(parse_unit_code) {
        Some(unit) => game
            .game_character_units
            .iter()
            .find(|entry| entry.game_character_id == master.character_id)
            .and_then(|entry| parse_unit_code(&entry.unit))
            .is_some_and(|card_unit| {
                card_unit == unit
                    || (matches!(card_unit, crate::types::Unit::Piapro)
                        && master
                            .support_unit
                            .as_deref()
                            .and_then(parse_unit_code)
                            .is_none_or(|support_unit| support_unit == unit))
            }),
        None => true,
    };
    character_ok && attr_ok && unit_ok
}

fn load_rarity_bonus_x2(
    user_card: &UserCard,
    master: &MasterCard,
    event_ctx: &EventContext,
) -> i32 {
    event_ctx
        .rarity_bonuses
        .iter()
        .filter(|entry| entry.card_rarity_type == master.card_rarity_type)
        .filter(|entry| entry.master_rank <= user_card.master_rank)
        .max_by_key(|entry| entry.master_rank)
        .map(|entry| entry.bonus_rate_x2)
        .unwrap_or(0)
}

fn load_custom_bonus_x2(master: &MasterCard, event_ctx: &EventContext) -> i32 {
    if event_ctx.custom_character_ids.is_empty() && event_ctx.custom_attr.is_none() {
        return 0;
    }
    let char_match = event_ctx
        .custom_character_ids
        .contains(&master.character_id);
    let attr_match = event_ctx
        .custom_attr
        .is_some_and(|attr| parse_attr_code(&master.attr) == Some(attr));
    if char_match && attr_match {
        100
    } else if char_match || attr_match {
        50
    } else {
        0
    }
}

/// 构建单卡的热路径活动 bonus，同时返回角色/属性轴命中标记。
pub(crate) fn build_card_event_bonus(
    user_card: &UserCard,
    master: &MasterCard,
    game: &GameData<'_>,
    event_ctx: &EventContext,
) -> (EventBonusHot, bool, bool) {
    let base_bonus_x2 = load_rarity_bonus_x2(user_card, master, event_ctx)
        + load_custom_bonus_x2(master, event_ctx);

    let custom_char = event_ctx
        .custom_character_ids
        .contains(&master.character_id);
    let custom_attr = event_ctx
        .custom_attr
        .is_some_and(|attr| parse_attr_code(&master.attr) == Some(attr));

    // 活动 deck bonus：多条规则命中时取最大值（与 C++/TS 一致）。
    let mut deck_bonus_x2 = 0i32;
    let mut deck_char = false;
    let mut deck_attr = false;
    for rule in &event_ctx.deck_bonuses {
        if card_matches_rule(master, rule, game) {
            deck_bonus_x2 = deck_bonus_x2.max(rule.bonus_rate.saturating_mul(2));
            if rule.character_id.is_some() {
                deck_char = true;
            }
            if rule.attr.is_some() {
                deck_attr = true;
            }
        }
    }
    let base_bonus_x2 = base_bonus_x2 + deck_bonus_x2;
    let limited_bonus_x2 = event_ctx
        .event_cards
        .iter()
        .find(|entry| entry.card_id == master.id)
        .map(|entry| entry.bonus_rate.saturating_mul(2))
        .unwrap_or(0);

    (
        EventBonusHot::from_x2(
            base_bonus_x2.clamp(0, u8::MAX as i32) as u8,
            limited_bonus_x2.clamp(0, u8::MAX as i32) as u8,
        ),
        custom_char || deck_char,
        custom_attr || deck_attr,
    )
}

/// 构建终章 leader honor bonus。
pub(crate) fn build_leader_honor_bonus(
    user: &UserProfile,
    master: &MasterCard,
    event_ctx: &EventContext,
) -> u16 {
    if event_ctx.event_id != FINAL_CHAPTER_EVENT_ID {
        return 0;
    }
    let total = event_ctx
        .honor_bonuses
        .iter()
        .filter(|entry| entry.leader_game_character_id == master.character_id)
        .filter(|entry| {
            user.user_honors
                .iter()
                .any(|honor| honor.honor_id == entry.honor_id)
        })
        .map(|entry| entry.bonus_rate.max(0) as u16)
        .sum::<u16>();
    total
}

/// 构建终章 leader 限定卡 bonus。
pub(crate) fn build_leader_limit_bonus(master: &MasterCard, event_ctx: &EventContext) -> u16 {
    event_ctx
        .event_cards
        .iter()
        .find(|entry| entry.card_id == master.id)
        .map(|entry| entry.leader_bonus_rate.max(0) as u16)
        .unwrap_or(0)
}
