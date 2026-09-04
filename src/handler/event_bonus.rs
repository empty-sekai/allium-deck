use crate::pool::EventBonusExact;
use crate::types::{Attr, EventType, FINAL_CHAPTER_EVENT_ID, Unit};

use super::BuildError;
use super::types::{
    BuildParams, EventCard, EventCardBonusLimit, EventDeckBonus, EventHonorBonus,
    EventRarityBonusRate, EventSkillScoreUpLimit, GameData, MasterCard, UserCard,
    WorldBloomDiffAttrBonus, attr_to_pool_index, parse_attr_code, parse_unit_code,
    resolve_event_type,
};

#[derive(Debug, Clone, PartialEq)]
struct PreparedEventDeckBonus {
    character_id: Option<i32>,
    attr: Option<u8>,
    unit: Option<Unit>,
    bonus_rate: i32,
    has_attr_rule: bool,
}

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
    deck_bonuses: Vec<PreparedEventDeckBonus>,
    /// 稀有度 bonus 规则。
    pub rarity_bonuses: Vec<EventRarityBonusRate>,
    /// 常见合法稀有度/master-rank 的无分支查表；手工测试上下文可留空回退原逻辑。
    rarity_bonus_x10: Option<[[i32; 6]; 6]>,
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
    /// 自定义 VS 角色支援团约束；Unit::None 表示不限制。
    pub custom_support_unit_by_char: [Unit; 27],
    /// 箱活或模拟活动的统一团过滤条件。
    pub filter_unit: Option<Unit>,
    /// 模拟 WL 活动合成出的支援 limited 加成行；真实活动为空。
    pub support_limited_bonuses: Vec<super::types::WBSupportDeckUnitEventLimitedBonus>,
}

fn common_event_unit(bonuses: &[EventDeckBonus], fallback: Option<&str>) -> Option<Unit> {
    let mut units = Vec::new();
    for unit in bonuses
        .iter()
        .filter_map(|entry| entry.unit.as_deref().and_then(parse_unit_code))
    {
        if !units.contains(&unit) {
            units.push(unit);
        }
    }
    if units.len() == 1 {
        units.into_iter().next()
    } else {
        fallback.and_then(parse_unit_code)
    }
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
        .map(|entry| entry.member_count_limit.max(0) as usize)
        // 终章（legacy 180 与模拟 WL3 终章）最多 4 张享受 limited bonus。
        .unwrap_or_else(|| {
            if crate::types::is_world_bloom_finale_event(event_id) {
                4
            } else {
                5
            }
        })
}

fn load_skill_limit(table: &[EventSkillScoreUpLimit], event_id: i32) -> Option<u32> {
    table
        .iter()
        .find(|entry| entry.event_id == event_id)
        // 表内存的是百分比（如 230 = 230%），实际加分上限是扣除基数 100% 后的点数。
        .map(|entry| (entry.score_up_limit - 100).max(0) as u32)
}

fn resolve_skill_limit(game: &GameData<'_>, params: &BuildParams, event_id: i32) -> Option<u32> {
    if matches!(
        params.live_type,
        crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
    ) {
        return None;
    }
    // 游戏真实数据优先：真实终章一旦在表中给出上限，以数据为准，不走兜底常量。
    if let Some(limit) = load_skill_limit(game.event_skill_score_up_limits, event_id) {
        return Some(limit);
    }
    // 数据缺行时的终章兜底：legacy 终章 180 与模拟 WL3 终章均沿用上一届
    // 真实终章的 140 点规则。
    if crate::types::is_world_bloom_finale_event(event_id) {
        return Some(140);
    }
    None
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
        return Some(super::world_bloom::world_bloom_event_turn(event_id));
    }
    None
}

fn resolve_world_bloom_character_id(game: &GameData<'_>, params: &BuildParams) -> Option<i32> {
    if params.world_bloom_character_id.is_some() {
        return params.world_bloom_character_id;
    }
    let event_id = params.event_id?;
    game.world_blooms
        .iter()
        .find(|entry| entry.event_id == event_id)
        .and_then(|entry| entry.game_character_id)
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
) -> Result<Option<EventContext>, BuildError> {
    // 模拟 WL 组卡：真实 event_id 优先；否则按 world_bloom_finale_turn /
    // world_bloom_event_turn 解析假活动，并合成 event cards / deck bonuses /
    // 章节与荣誉加成行。
    let wb_event_id = super::world_bloom::resolve_wb_event_id(params)?;
    let synth_rows =
        wb_event_id.map(|event_id| super::world_bloom::synthesize_wb_rows(game, event_id));
    let event_type = if wb_event_id.is_some() {
        EventType::WorldBloom // 假活动固定为 world_bloom
    } else {
        let Some(event_type) = resolve_event_type(game, params).or_else(|| {
            if params.event_unit.is_some()
                || params.event_attr.is_some()
                || !params.custom_bonus_character_ids.is_empty()
                || params.custom_bonus_attr.is_some()
            {
                Some(EventType::Marathon)
            } else {
                None
            }
        }) else {
            return Ok(None);
        };
        event_type
    };
    if matches!(event_type, EventType::WorldBloom)
        && wb_event_id.is_none()
        && params.event_id.is_none()
        && params.world_bloom_event_turn.is_none()
        && params.world_bloom_finale_turn.is_none()
    {
        return Err(BuildError::InvalidConfig(
            "world_bloom 模拟需要 world_bloom_event_turn（或提供真实活动 event_id）".to_string(),
        ));
    }
    let event_id = wb_event_id.unwrap_or_else(|| params.event_id.unwrap_or_default());
    let world_bloom_event_turn = if let Some(id) = wb_event_id {
        Some(super::world_bloom::world_bloom_event_turn(id))
    } else if matches!(event_type, EventType::WorldBloom) {
        resolve_world_bloom_event_turn(game, params)
    } else {
        params.world_bloom_event_turn
    };

    let raw_deck_bonuses = game
        .event_deck_bonuses
        .iter()
        .filter(|entry| entry.event_id == event_id)
        .cloned()
        .collect::<Vec<_>>();
    let filter_unit = common_event_unit(&raw_deck_bonuses, params.event_unit.as_deref());
    let mut deck_bonuses = raw_deck_bonuses
        .into_iter()
        .map(|rule| PreparedEventDeckBonus {
            character_id: rule.character_id,
            attr: rule
                .attr
                .as_deref()
                .and_then(parse_attr_code)
                .and_then(attr_to_pool_index),
            unit: rule.unit.as_deref().and_then(parse_unit_code),
            bonus_rate: rule.bonus_rate,
            has_attr_rule: rule.attr.is_some(),
        })
        .collect::<Vec<_>>();
    if let Some(rows) = &synth_rows {
        // 模拟活动：合成行以 character 精确匹配（无团/属性轴）。
        deck_bonuses.extend(rows.deck_bonuses.iter().map(|rule| PreparedEventDeckBonus {
            character_id: rule.character_id,
            attr: None,
            unit: None,
            bonus_rate: rule.bonus_rate,
            has_attr_rule: false,
        }));
    }

    let has_simulated_bonus = params.event_attr.is_some()
        || params.event_unit.is_some()
        || params.custom_bonus_attr.is_some()
        || !params.custom_bonus_character_ids.is_empty();
    let rarity_event_id = if event_id == 0 && !has_simulated_bonus {
        None
    } else {
        game.event_rarity_bonus_rates
            .iter()
            .any(|entry| entry.event_id == event_id)
            .then_some(event_id)
            .or_else(|| {
                game.event_rarity_bonus_rates
                    .first()
                    .map(|entry| entry.event_id)
            })
    };

    let rarity_bonuses = game
        .event_rarity_bonus_rates
        .iter()
        .filter(|entry| Some(entry.event_id) == rarity_event_id)
        .cloned()
        .collect::<Vec<_>>();
    let mut rarity_bonus_x10 = [[0i32; 6]; 6];
    for (rarity, row) in rarity_bonus_x10.iter_mut().enumerate() {
        for (rank, value) in row.iter_mut().enumerate() {
            *value = rarity_bonuses
                .iter()
                .filter(|entry| entry.card_rarity_type == rarity as i32)
                .filter(|entry| entry.master_rank <= rank as i32)
                .max_by_key(|entry| entry.master_rank)
                .map(|entry| entry.bonus_rate_x10)
                .unwrap_or(0);
        }
    }

    Ok(Some(EventContext {
        event_id,
        event_type,
        event_cards: {
            let mut cards = game
                .event_cards
                .iter()
                .filter(|entry| entry.event_id == event_id)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(rows) = &synth_rows {
                cards.extend(rows.event_cards.iter().cloned());
            }
            cards
        },
        deck_bonuses,
        rarity_bonuses,
        rarity_bonus_x10: Some(rarity_bonus_x10),
        honor_bonuses: {
            let mut bonuses = game
                .event_honor_bonuses
                .iter()
                .filter(|entry| entry.event_id == event_id)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(rows) = &synth_rows {
                bonuses.extend(rows.honor_bonuses.iter().cloned());
            }
            bonuses
        },
        skill_score_up_limit: resolve_skill_limit(game, params, event_id),
        card_bonus_count_limit: load_card_bonus_limit(game.event_card_bonus_limits, event_id),
        diff_attr_bonus: if matches!(event_type, EventType::WorldBloom) {
            load_diff_attr_bonus(game.world_bloom_different_attribute_bonuses)
        } else {
            [0; 6]
        },
        support_deck_count: load_support_deck_count(world_bloom_event_turn, event_type),
        // 模拟活动不合成默认章节角色：由调用方按需显式指定
        // world_bloom_character_id。
        world_bloom_character_id: resolve_world_bloom_character_id(game, params),
        world_bloom_event_turn,
        custom_character_ids: if params.custom_bonus_character_ids.is_empty() {
            custom_character_ids(game, params.event_unit.as_deref())
        } else {
            params.custom_bonus_character_ids.clone()
        },
        custom_attr: params
            .custom_bonus_attr
            .as_deref()
            .or(params.event_attr.as_deref())
            .and_then(parse_attr_code),
        custom_support_unit_by_char: {
            let mut units = [Unit::None; 27];
            for entry in &params.custom_bonus_character_support_units {
                if (1..=26).contains(&entry.character_id) {
                    units[entry.character_id as usize] = entry.unit;
                }
            }
            units
        },
        filter_unit,
        support_limited_bonuses: synth_rows
            .map(|rows| rows.support_limited_bonuses)
            .unwrap_or_default(),
    }))
}

fn card_matches_rule(
    master: &MasterCard,
    card_attr: u8,
    primary_unit: Option<Unit>,
    support_unit: Option<Unit>,
    rule: &PreparedEventDeckBonus,
) -> bool {
    let character_ok = rule
        .character_id
        .is_none_or(|character_id| character_id == master.character_id);
    let attr_ok = rule.attr.is_none_or(|attr| card_attr == attr);
    let unit_ok = match rule.unit {
        Some(unit) => {
            if master.character_id >= 21 {
                // VS 卡以支援团为准：支援团等于规则团（或未设支援团）才命中；
                // 主团 piapro 不参与命中（与参照实现一致：VS 卡的队伍身份
                // 跟随玩家为其选择的支援团）。
                support_unit.is_none_or(|support_unit| support_unit == unit)
            } else {
                primary_unit.is_some_and(|card_unit| card_unit == unit)
            }
        }
        None => true,
    };
    character_ok && attr_ok && unit_ok
}

fn load_rarity_bonus_x10(
    user_card: &UserCard,
    master: &MasterCard,
    event_ctx: &EventContext,
) -> i32 {
    if let Some(table) = event_ctx.rarity_bonus_x10.as_ref()
        && let (Ok(rarity), Ok(rank)) = (
            usize::try_from(master.card_rarity_type),
            usize::try_from(user_card.master_rank),
        )
        && let Some(value) = table.get(rarity).and_then(|row| row.get(rank))
    {
        return *value;
    }
    event_ctx
        .rarity_bonuses
        .iter()
        .filter(|entry| entry.card_rarity_type == master.card_rarity_type)
        .filter(|entry| entry.master_rank <= user_card.master_rank)
        .max_by_key(|entry| entry.master_rank)
        .map(|entry| entry.bonus_rate_x10)
        .unwrap_or(0)
}

#[cfg(test)]
fn load_custom_bonus_x2(
    master: &MasterCard,
    card_attr: u8,
    support_unit: Option<Unit>,
    support_unit_unrestricted: bool,
    event_ctx: &EventContext,
) -> i32 {
    if event_ctx.custom_character_ids.is_empty() && event_ctx.custom_attr.is_none() {
        return 0;
    }
    let char_match =
        custom_character_matches(master, support_unit, support_unit_unrestricted, event_ctx);
    let attr_match = event_ctx
        .custom_attr
        .and_then(attr_to_pool_index)
        .is_some_and(|attr| card_attr == attr);
    if char_match && attr_match {
        100
    } else if char_match || attr_match {
        50
    } else {
        0
    }
}

fn custom_character_matches(
    master: &MasterCard,
    support_unit: Option<Unit>,
    support_unit_unrestricted: bool,
    event_ctx: &EventContext,
) -> bool {
    if !event_ctx
        .custom_character_ids
        .contains(&master.character_id)
    {
        return false;
    }
    let required = usize::try_from(master.character_id)
        .ok()
        .filter(|id| *id < event_ctx.custom_support_unit_by_char.len())
        .map(|id| event_ctx.custom_support_unit_by_char[id])
        .unwrap_or(Unit::None);
    if matches!(required, Unit::None) {
        return true;
    }
    support_unit_unrestricted || support_unit == Some(required)
}

/// 构建单卡的热路径活动 bonus，同时返回角色/属性轴命中标记。
pub(crate) fn build_card_event_bonus(
    user_card: &UserCard,
    master: &MasterCard,
    card_attr: u8,
    primary_unit: Option<Unit>,
    support_unit: Option<Unit>,
    support_unit_unrestricted: bool,
    limited_bonus_x10: i32,
    event_ctx: &EventContext,
) -> (EventBonusExact, bool, bool) {
    let rarity_bonus_x10 = load_rarity_bonus_x10(user_card, master, event_ctx);
    let custom_char =
        custom_character_matches(master, support_unit, support_unit_unrestricted, event_ctx);
    let custom_attr = event_ctx
        .custom_attr
        .and_then(attr_to_pool_index)
        .is_some_and(|attr| card_attr == attr);
    let custom_bonus_x2 = if custom_char && custom_attr {
        100
    } else if custom_char || custom_attr {
        50
    } else {
        0
    };

    // 活动 deck bonus：多条规则命中时取最大值（与 C++/TS 一致）。
    let mut deck_bonus_x2 = 0i32;
    let mut deck_char = false;
    let mut deck_attr = false;
    for rule in &event_ctx.deck_bonuses {
        if card_matches_rule(master, card_attr, primary_unit, support_unit, rule) {
            deck_bonus_x2 = deck_bonus_x2.max(rule.bonus_rate.saturating_mul(2));
            if rule.character_id.is_some() {
                deck_char = true;
            }
            if rule.has_attr_rule {
                deck_attr = true;
            }
        }
    }
    let base_bonus_x10 = rarity_bonus_x10 + (custom_bonus_x2 + deck_bonus_x2) * 5;

    (
        EventBonusExact::from_x10(
            base_bonus_x10.clamp(0, u16::MAX as i32) as u16,
            limited_bonus_x10.clamp(0, u16::MAX as i32) as u16,
        ),
        custom_char || deck_char,
        custom_attr || deck_attr,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_context() -> EventContext {
        let mut support_units = [Unit::None; 27];
        support_units[21] = Unit::Street;
        EventContext {
            event_id: 0,
            event_type: EventType::Marathon,
            event_cards: Vec::new(),
            deck_bonuses: Vec::new(),
            rarity_bonuses: Vec::new(),
            rarity_bonus_x10: None,
            honor_bonuses: Vec::new(),
            skill_score_up_limit: None,
            card_bonus_count_limit: 5,
            diff_attr_bonus: [0; 6],
            support_deck_count: 0,
            world_bloom_character_id: None,
            world_bloom_event_turn: None,
            custom_character_ids: vec![21],
            custom_attr: Some(Attr::Cute),
            custom_support_unit_by_char: support_units,
            filter_unit: None,
            support_limited_bonuses: Vec::new(),
        }
    }

    fn master(support_unit: Option<&str>, attr: &str) -> MasterCard {
        MasterCard {
            id: 1,
            character_id: 21,
            attr: attr.to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: String::new(),
            skill_id: 1,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: support_unit.map(str::to_string),
            max_level: None,
            max_skill_level: None,
            max_master_rank: None,
        }
    }

    fn custom_bonus(master: &MasterCard, ctx: &EventContext) -> i32 {
        let support_unit = master.support_unit.as_deref().and_then(parse_unit_code);
        let support_unit_unrestricted = master.support_unit.as_deref().is_none_or(|value| {
            value.trim().is_empty() || value.trim().eq_ignore_ascii_case("none")
        });
        load_custom_bonus_x2(
            master,
            parse_attr_code(&master.attr)
                .and_then(attr_to_pool_index)
                .unwrap_or(u8::MAX),
            support_unit,
            support_unit_unrestricted,
            ctx,
        )
    }

    #[test]
    fn custom_support_unit_accepts_matching_or_none_and_rejects_other_unit() {
        let ctx = custom_context();
        assert_eq!(custom_bonus(&master(Some("street"), "cool"), &ctx), 50);
        assert_eq!(custom_bonus(&master(Some("none"), "cool"), &ctx), 50);
        assert_eq!(custom_bonus(&master(None, "cute"), &ctx), 100);
        assert_eq!(custom_bonus(&master(Some("none"), "cute"), &ctx), 100);
        assert_eq!(custom_bonus(&master(Some("idol"), "cool"), &ctx), 0);
        assert_eq!(
            custom_bonus(&master(Some("future_unknown_unit"), "cool"), &ctx),
            0
        );
    }

    #[test]
    fn rarity_master_rank_bonus_keeps_tenth_percent_precision() {
        let mut ctx = custom_context();
        ctx.custom_character_ids.clear();
        ctx.custom_attr = None;
        ctx.rarity_bonuses.push(EventRarityBonusRate {
            event_id: 0,
            card_rarity_type: 4,
            master_rank: 2,
            bonus_rate_x10: 2,
        });
        let mut fast = [[0i32; 6]; 6];
        fast[4][2] = 2;
        ctx.rarity_bonus_x10 = Some(fast);
        let user_card = UserCard {
            card_id: 1,
            level: 1,
            skill_level: 1,
            master_rank: 2,
            special_training_status: "none".to_string(),
            default_image: "original".to_string(),
            episodes_read: Vec::new(),
            is_virtual: false,
            has_canvas_bonus_override: None,
        };
        let _game = GameData {
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
            event_rarity_bonus_rates: &[],
            event_honor_bonuses: &[],
            event_card_bonus_limits: &[],
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
            honors: &[],
            bonds_honors: &[],
        };

        let (bonus, _, _) = build_card_event_bonus(
            &user_card,
            &master(None, "cool"),
            0,
            None,
            None,
            true,
            0,
            &ctx,
        );
        assert_eq!(bonus.base_x10(), 2);
        assert_eq!(bonus.base_rate(), 0.2);
    }
}
