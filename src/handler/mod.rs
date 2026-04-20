mod card_config;
mod event_bonus;
mod gather;
mod music;
mod power;
mod skill;
mod support_bonus;
pub mod types;

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::search::{SearchContext, SupportDeck};
use crate::types::{DefaultImage, FINAL_CHAPTER_EVENT_ID};

use card_config::apply_card_config;
use event_bonus::{
    build_card_event_bonus, build_event_context, build_leader_honor_bonus,
    build_leader_limit_bonus, EventContext,
};
use gather::{sort_and_gather, CardIntermediate, FullPrecisionCard};
use music::build_music_params;
use power::{build_power, resolve_unit_mask};
use skill::{build_skill, SkillState};
use support_bonus::calc_wb_support_bonus;
use types::{attr_to_pool_index, default_image_kind, parse_attr_code, parse_unit_code};

pub use types::*;

/// handler 构建阶段的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// 过滤后无候选卡。
    EmptyPool,
    /// 候选卡超过 512-bit mask 容量。
    TooManyCards(usize),
}

impl Display for BuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPool => f.write_str("候选卡池为空"),
            Self::TooManyCards(count) => write!(f, "候选卡数量超过 mask 容量: {count}"),
        }
    }
}

impl Error for BuildError {}

fn enrich_master(master: &types::MasterCard, game: &types::GameData<'_>) -> types::MasterCard {
    let mut master = master.clone();
    if master.max_level.is_none() || master.max_skill_level.is_none() {
        if let Some(rarity) = game
            .card_rarities
            .iter()
            .find(|entry| entry.card_rarity_type == master.card_rarity_type)
        {
            master.max_level.get_or_insert(rarity.max_level);
            master.max_skill_level.get_or_insert(rarity.max_skill_level);
        }
    }
    if master.max_master_rank.is_none() {
        let max_master_rank = game
            .master_lessons
            .iter()
            .filter(|entry| entry.card_rarity_type == master.card_rarity_type)
            .map(|entry| entry.master_rank)
            .max()
            .unwrap_or(0);
        master.max_master_rank = Some(max_master_rank);
    }
    master
}

fn merged_configs(params: &types::BuildParams) -> types::CardConfigSet {
    let mut configs = params.card_configs.clone();
    configs
        .single_card_configs
        .extend(params.single_card_configs.iter().cloned());
    configs
}

fn normalize_user_cards(
    user: &types::UserProfile,
    params: &types::BuildParams,
) -> Vec<types::UserCard> {
    let mut cards = user.user_cards.clone();
    for &card_id in &params.fixed_cards {
        if cards.iter().any(|card| card.card_id == card_id) {
            continue;
        }
        cards.push(types::UserCard {
            card_id,
            level: 1,
            skill_level: 1,
            master_rank: 0,
            special_training_status: "none".to_string(),
            default_image: "original".to_string(),
            episodes_read: Vec::new(),
            is_virtual: true,
            has_canvas_bonus_override: None,
        });
    }
    cards
}

fn keep_card(card: &CardIntermediate, params: &types::BuildParams) -> bool {
    if params.excluded_cards.contains(&card.game_card_id) {
        return false;
    }

    if let Some(unit) = params
        .unit_filter
        .as_deref()
        .and_then(parse_unit_code)
        .and_then(types::unit_to_pool_index)
    {
        let piapro_bit = 1u8 << 5;
        let wanted = 1u8 << unit;
        if card.unit_mask_raw & (wanted | piapro_bit) == 0 {
            return false;
        }
    }

    if let Some(attr) = params
        .attr_filter
        .as_deref()
        .and_then(parse_attr_code)
        .and_then(attr_to_pool_index)
    {
        if card.attr != attr {
            return false;
        }
    }

    if params.filter_other_unit {
        if let Some(unit) = params
            .event_unit
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
        {
            let piapro_bit = 1u8 << 5;
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & (wanted | piapro_bit) == 0 {
                return false;
            }
        }
    }

    if let Some(challenge_char_id) = params.challenge_live_character_id {
        if card.character_id != challenge_char_id as u8 {
            return false;
        }
    }

    true
}

fn normalize_boost_rate_pct(boost: Option<i32>) -> u32 {
    match boost {
        Some(value) if value > 0 && value <= 20 => (value * 100) as u32,
        Some(value) if value > 0 => value as u32,
        _ => 100,
    }
}

fn user_character_rank(user: &types::UserProfile, character_id: i32) -> i32 {
    user.user_characters
        .iter()
        .find(|entry| entry.character_id == character_id)
        .map(|entry| entry.character_rank)
        .unwrap_or(0)
}

fn skill_states_for_card(
    user_card: &types::UserCard,
    master: &types::MasterCard,
    params: &types::BuildParams,
) -> Vec<SkillState> {
    if master.special_training_skill_id.is_none() {
        return vec![SkillState::BeforeTraining];
    }
    if params.keep_after_training_state {
        if matches!(
            default_image_kind(&user_card.default_image),
            DefaultImage::SpecialTraining
        ) {
            vec![SkillState::AfterTraining]
        } else {
            vec![SkillState::BeforeTraining]
        }
    } else {
        vec![SkillState::AfterTraining, SkillState::BeforeTraining]
    }
}

fn build_support_deck(
    full: &[FullPrecisionCard],
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> SupportDeck {
    let Some(event_ctx) = event_ctx else {
        return SupportDeck::default();
    };
    if event_ctx.support_deck_count == 0 {
        return SupportDeck::default();
    }

    let mut cards: Vec<(u16, u16)> = Vec::with_capacity(full.len());
    for card in full {
        let bonus = calc_wb_support_bonus(
            card,
            game,
            event_ctx.event_id,
            event_ctx.world_bloom_event_turn,
            event_ctx.world_bloom_character_id,
        );
        if let Some((_, existing_bonus)) = cards
            .iter_mut()
            .find(|(game_card_id, _)| *game_card_id == card.game_card_id)
        {
            *existing_bonus = (*existing_bonus).max(bonus);
        } else {
            cards.push((card.game_card_id, bonus));
        }
    }
    cards.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));

    SupportDeck {
        cards,
        count: event_ctx.support_deck_count,
    }
}

fn build_search_context(
    full: &[FullPrecisionCard],
    game: &types::GameData<'_>,
    params: &types::BuildParams,
    event_ctx: Option<&EventContext>,
    music: Option<&music::MusicParams>,
) -> SearchContext {
    let support_deck = build_support_deck(full, game, event_ctx);
    let support_bonus_top_sum = support_deck
        .cards
        .iter()
        .take(support_deck.count as usize)
        .map(|(_, bonus)| *bonus as u32)
        .sum::<u32>();
    let diff_attr_bonus = event_ctx.map(|ctx| ctx.diff_attr_bonus).unwrap_or([0; 6]);
    let mut skill_values = full
        .iter()
        .map(|card| card.skill_max_exact as u32)
        .collect::<Vec<_>>();
    skill_values.sort_unstable_by(|left, right| right.cmp(left));
    let skill_ub_global = skill_values.into_iter().take(5).sum::<u32>();

    SearchContext {
        target: params.target,
        bonus_targets: params
            .target_bonus_list
            .iter()
            .copied()
            .filter(|value| *value > 0)
            .map(|value| value as u32)
            .collect(),
        music_rate_pct: music.map(|music| music.event_rate_pct).unwrap_or(100),
        boost_rate_pct: normalize_boost_rate_pct(params.boost),
        base_score: music.map(|music| music.meta.base_score).unwrap_or(1.0),
        base_score_auto: music.map(|music| music.meta.base_score_auto).unwrap_or(1.0),
        fever_score: music.map(|music| music.meta.fever_score).unwrap_or(0.0),
        skill_scores: music
            .map(|music| {
                [
                    music.meta.solo_skill_scores,
                    music.meta.multi_skill_scores,
                    music.meta.auto_skill_scores,
                ]
            })
            .unwrap_or([[0.0; 6]; 3]),
        other_score: params.other_score.unwrap_or(0),
        life: params.life.unwrap_or(1000),
        diff_attr_bonus,
        support_deck,
        is_world_bloom: event_ctx
            .is_some_and(|ctx| matches!(ctx.event_type, crate::types::EventType::WorldBloom)),
        is_final_chapter: event_ctx.is_some_and(|ctx| ctx.event_id == FINAL_CHAPTER_EVENT_ID),
        live_type: params.live_type,
        event_type: event_ctx.map(|ctx| ctx.event_type),
        keep_after_training_state: params.keep_after_training_state,
        skill_reference_strategy: params.skill_reference_strategy,
        best_skill_as_leader: params.best_skill_as_leader,
        live_skill_order: params.live_skill_order,
        specific_skill_order: params.specific_skill_order,
        multi_teammate_score_up: params.multi_teammate_score_up,
        multi_teammate_power: params.multi_teammate_power,
        extra_bonus_ub: diff_attr_bonus.into_iter().max().unwrap_or(0) as u32
            + support_bonus_top_sum,
        w_power: 1.0,
        w_bonus: 1.0,
        skill_ub_global,
        card_bonus_count_limit: event_ctx.map(|ctx| ctx.card_bonus_count_limit).unwrap_or(5),
        honor_bonus: 0,
        leader_honor_bonus: full.iter().map(|card| card.leader_honor_bonus).collect(),
        leader_limit_bonus: full.iter().map(|card| card.leader_limit_bonus).collect(),
        skill_is_after_training: full
            .iter()
            .map(|card| card.skill.is_after_training)
            .collect(),
        trained_to_special_image: full
            .iter()
            .map(|card| matches!(card.default_image, DefaultImage::SpecialTraining))
            .collect(),
    }
}

fn compute_honor_bonus(user: &types::UserProfile, game: &types::GameData<'_>) -> u32 {
    user.user_honors
        .iter()
        .filter_map(|uh| {
            let honor = game.honors.iter().find(|h| h.id == uh.honor_id)?;
            let level = honor.levels.iter().find(|lv| lv.level == uh.level)?;
            Some(level.bonus.max(0) as u32)
        })
        .sum()
}

fn resolve_fixture_bonus_limit(
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> Option<i32> {
    let event_id = event_ctx?.event_id;
    game.event_mysekai_fixture_performance_bonus_limits
        .iter()
        .find(|entry| entry.event_id == event_id)
        .map(|entry| entry.bonus_rate_limit)
}

/// 将 masterdata + userdata 构建为搜索使用的 `CardPool` 与 `SearchContext`。
pub fn build_card_pool(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
) -> Result<(crate::pool::CardPool, SearchContext), BuildError> {
    let event_ctx = build_event_context(game, params);
    let fixture_bonus_limit = resolve_fixture_bonus_limit(game, event_ctx.as_ref());
    let music = build_music_params(game, params);
    let configs = merged_configs(params);
    let normalized_cards = normalize_user_cards(user, params);
    let mut cards = Vec::new();

    for original_user_card in normalized_cards {
        let Some(master) = game
            .cards
            .iter()
            .find(|card| card.id == original_user_card.card_id)
        else {
            continue;
        };
        let master = enrich_master(master, game);
        let mut user_card = original_user_card.clone();
        if !apply_card_config(&mut user_card, &master, &configs) {
            continue;
        }

        let unit_mask_raw = resolve_unit_mask(&master, game);
        if unit_mask_raw == 0 {
            continue;
        }
        let Some(attr) = parse_attr_code(&master.attr).and_then(attr_to_pool_index) else {
            continue;
        };

        let power = build_power(&user_card, &master, game, user, fixture_bonus_limit);
        let event_bonus = event_ctx
            .as_ref()
            .map(|ctx| build_card_event_bonus(&user_card, &master, game, ctx))
            .unwrap_or_default();
        let leader_honor_bonus = event_ctx
            .as_ref()
            .map(|ctx| build_leader_honor_bonus(user, &master, ctx))
            .unwrap_or(0);
        let leader_limit_bonus = event_ctx
            .as_ref()
            .map(|ctx| build_leader_limit_bonus(&master, ctx))
            .unwrap_or(0);
        let character_rank = user_character_rank(user, master.character_id);

        for skill_state in skill_states_for_card(&user_card, &master, params) {
            let skill = build_skill(
                &user_card,
                &master,
                game,
                character_rank,
                event_ctx.as_ref().and_then(|ctx| ctx.skill_score_up_limit),
                skill_state,
            );
            let default_image = match skill_state {
                SkillState::BeforeTraining => DefaultImage::Original,
                SkillState::AfterTraining => DefaultImage::SpecialTraining,
            };

            let ep_sort_key = i64::from(power.power_max)
                + i64::from(event_bonus.base_bonus as i32 + event_bonus.limited_bonus as i32)
                    * 1_000;
            let intermediate = CardIntermediate {
                game_card_id: master.id,
                card_rarity_type: master.card_rarity_type,
                character_id: master.character_id.clamp(0, u8::MAX as i32) as u8,
                attr,
                unit_mask_raw,
                default_image,
                master_rank: user_card.master_rank,
                skill_level: user_card.skill_level,
                power: power.clone(),
                skill,
                event_bonus,
                leader_honor_bonus,
                leader_limit_bonus,
                ep_sort_key,
            };

            if keep_card(&intermediate, params) {
                cards.push(intermediate);
            }
        }
    }

    if cards.is_empty() {
        return Err(BuildError::EmptyPool);
    }
    if cards.len() > crate::pool::MASK_WORDS * 64 {
        return Err(BuildError::TooManyCards(cards.len()));
    }

    let (pool, full) = sort_and_gather(cards, params.target, event_ctx.is_some());
    let mut search_ctx =
        build_search_context(&full, game, params, event_ctx.as_ref(), music.as_ref());
    search_ctx.honor_bonus = compute_honor_bonus(user, game);
    Ok((pool, search_ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{EventBonusHot, SkillSlot};
    use crate::types::{LiveType, ScoreTarget, SkillReferenceStrategy};

    fn sample_game<'a>(
        cards: &'a [MasterCard],
        params: &'a [types::CardParameter],
        rarities: &'a [types::CardRarity],
        episodes: &'a [types::CardEpisode],
        lessons: &'a [types::MasterLesson],
        skills: &'a [types::Skill],
        effects: &'a [types::SkillEffect],
        area_items: &'a [types::AreaItemLevel],
        units: &'a [types::GameCharacterUnit],
    ) -> GameData<'a> {
        GameData {
            cards,
            card_parameters: params,
            card_rarities: rarities,
            card_episodes: episodes,
            master_lessons: lessons,
            skills,
            skill_effects: effects,
            area_item_levels: area_items,
            game_character_units: units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
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
        }
    }

    fn sample_user_card(card_id: i32) -> UserCard {
        UserCard {
            card_id,
            level: 1,
            skill_level: 1,
            master_rank: 0,
            special_training_status: "none".to_string(),
            default_image: "original".to_string(),
            episodes_read: Vec::new(),
            is_virtual: false,
            has_canvas_bonus_override: None,
        }
    }

    #[test]
    fn handler_build_power_uses_f32_item_accumulation() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 101,
            param2: 101,
            param3: 101,
        }];
        let area_items = [
            types::AreaItemLevel {
                area_item_id: 1,
                level: 1,
                unit: None,
                attr: None,
                character_id: None,
                power_rate: 1.0,
                power_all_match_rate: 1.0,
            },
            types::AreaItemLevel {
                area_item_id: 2,
                level: 1,
                unit: None,
                attr: None,
                character_id: None,
                power_rate: 1.0,
                power_all_match_rate: 1.0,
            },
        ];
        let game_units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let game = sample_game(
            &cards,
            &params,
            &[],
            &[],
            &[],
            &[],
            &[],
            &area_items,
            &game_units,
        );
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            user_area_items: vec![
                types::UserAreaItem {
                    area_item_id: 1,
                    level: 1,
                },
                types::UserAreaItem {
                    area_item_id: 2,
                    level: 1,
                },
            ],
            ..UserProfile::default()
        };

        let result = build_power(&sample_user_card(1), &cards[0], &game, &user, None);
        assert_eq!(result.resolved[1][0].area_item_bonus, 6);
        assert_eq!(result.resolved[1][0].total, 309);
    }

    #[test]
    fn handler_build_card_pool_only_clamps_fixture_bonus_for_matching_event() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        }];
        let skills = [types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        }];
        let effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 100,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let events = [types::Event {
            id: FINAL_CHAPTER_EVENT_ID,
            event_type: "marathon".to_string(),
        }];
        let fixture_limits = [types::EventFixtureBonusLimit {
            event_id: FINAL_CHAPTER_EVENT_ID,
            bonus_rate_limit: 20,
        }];
        let game = GameData {
            cards: &cards,
            card_parameters: &params,
            card_rarities: &[],
            card_episodes: &[],
            master_lessons: &[],
            skills: &skills,
            skill_effects: &effects,
            area_item_levels: &[],
            game_character_units: &units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            events: &events,
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &fixture_limits,
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            user_mysekai_fixture_bonuses: vec![types::UserFixtureBonus {
                character_id: 1,
                event_id: None,
                total_bonus_rate: 30,
            }],
            ..UserProfile::default()
        };

        let (pool, _) = build_card_pool(&user, &game, &BuildParams::default()).unwrap();
        let idx = pool.card_idx(0).unwrap();
        assert_eq!(pool.power_max(idx), 309);

        let params = BuildParams {
            event_id: Some(FINAL_CHAPTER_EVENT_ID),
            ..BuildParams::default()
        };
        let (pool, _) = build_card_pool(&user, &game, &params).unwrap();
        let idx = pool.card_idx(0).unwrap();
        assert_eq!(pool.power_max(idx), 306);
    }

    #[test]
    fn handler_build_skill_covers_normal_unit_count_diff_and_ref() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let game_units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let skills = [types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        }];
        let empty_game = sample_game(&cards, &[], &[], &[], &[], &skills, &[], &[], &game_units);

        let normal_effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 120,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let game = GameData {
            skill_effects: &normal_effects,
            ..empty_game
        };
        let normal = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            0,
            Some(140),
            SkillState::BeforeTraining,
        );
        assert_eq!(
            normal.slot,
            SkillSlot {
                skill_type: 0,
                value: 120
            }
        );

        let unit_count_effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_unit_count".to_string(),
                value: 10,
                additional_value: None,
                unit_member_count: Some(1),
                unit: Some("idol".to_string()),
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_unit_count".to_string(),
                value: 50,
                additional_value: None,
                unit_member_count: Some(5),
                unit: Some("idol".to_string()),
                activate_character_rank: None,
            },
        ];
        let game = GameData {
            skill_effects: &unit_count_effects,
            ..empty_game
        };
        let unit_count = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            0,
            None,
            SkillState::BeforeTraining,
        );
        assert_eq!(unit_count.slot.skill_type, 1);
        assert_eq!(
            unit_count
                .unit_count
                .as_ref()
                .map(|entry| entry.score_up[0]),
            Some(10)
        );
        assert_eq!(
            unit_count
                .unit_count
                .as_ref()
                .map(|entry| entry.score_up[4]),
            Some(50)
        );

        let diff_effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_diff".to_string(),
            value: 20,
            additional_value: Some(5),
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let game = GameData {
            skill_effects: &diff_effects,
            ..empty_game
        };
        let diff = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            0,
            None,
            SkillState::BeforeTraining,
        );
        assert_eq!(
            diff.diff,
            Some(crate::pool::DiffSkill {
                base: 20,
                increment: 5
            })
        );

        let ref_effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 100,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_reference".to_string(),
                value: 20,
                additional_value: Some(60),
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let game = GameData {
            skill_effects: &ref_effects,
            ..empty_game
        };
        let ref_skill = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            0,
            Some(140),
            SkillState::BeforeTraining,
        );
        assert_eq!(
            ref_skill.ref_skill,
            Some(crate::pool::RefSkill { rate: 20, max: 40 })
        );
        assert_eq!(ref_skill.skill_max, 140);
    }

    #[test]
    fn handler_build_card_pool_splits_bfes_dual_skill_cards() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            skill_id: 10,
            special_training_skill_id: Some(11),
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let skills = [
            types::Skill {
                id: 10,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 11,
                level: 1,
                is_after_training: true,
            },
        ];
        let effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 80,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 11,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 120,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let game = sample_game(&cards, &[], &[], &[], &[], &skills, &effects, &[], &units);
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            ..UserProfile::default()
        };
        let params = BuildParams {
            target: ScoreTarget::Skill,
            ..BuildParams::default()
        };

        let (pool, _) =
            build_card_pool(&user, &game, &params).expect("dual-skill pool should build");
        assert_eq!(pool.count(), 2);
        assert_eq!(
            pool.skill_max(pool.card_idx(0).expect("after skill entry")),
            120
        );
        assert_eq!(
            pool.skill_max(pool.card_idx(1).expect("before skill entry")),
            80
        );
    }

    #[test]
    fn handler_apply_card_config_supports_override_and_disable() {
        let master = MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        };
        let mut user_card = sample_user_card(1);
        let mut configs = CardConfigSet::default();
        configs.rarity_4_config.level_max = true;
        configs.single_card_configs.push(types::SingleCardConfig {
            card_id: 1,
            config: types::CardRarityConfig {
                disable: true,
                ..types::CardRarityConfig::default()
            },
        });
        assert!(!apply_card_config(&mut user_card, &master, &configs));

        let mut user_card = sample_user_card(1);
        let mut configs = CardConfigSet::default();
        configs.rarity_4_config.level_max = true;
        configs.rarity_4_config.skill_max = true;
        configs.rarity_4_config.master_max = true;
        assert!(apply_card_config(&mut user_card, &master, &configs));
        assert_eq!(user_card.level, 60);
        assert_eq!(user_card.skill_level, 4);
        assert_eq!(user_card.master_rank, 5);
    }

    #[test]
    fn handler_sort_and_gather_reindexes_dense_order() {
        let card = |game_card_id: i32, power_max: i32| CardIntermediate {
            game_card_id,
            card_rarity_type: 4,
            character_id: game_card_id as u8,
            attr: 0,
            unit_mask_raw: 1,
            default_image: crate::types::DefaultImage::Original,
            master_rank: 0,
            skill_level: 1,
            power: power::PowerResult {
                resolved: [[crate::types::PowerDetail::default(); 4]; 6],
                power_min: power_max - 10,
                power_max,
            },
            skill: skill::SkillResult {
                slot: SkillSlot::default(),
                unit_count: None,
                diff: None,
                ref_skill: None,
                skill_min: 1,
                skill_max: 2,
                full: crate::types::SkillInfo::default(),
            },
            event_bonus: EventBonusHot {
                base_bonus: 1,
                limited_bonus: 1,
            },
            leader_honor_bonus: 0,
            leader_limit_bonus: 0,
            ep_sort_key: power_max as i64,
        };
        let (pool, _) = sort_and_gather(
            vec![card(1, 100), card(3, 300), card(2, 200)],
            ScoreTarget::Power,
            false,
        );
        assert_eq!(pool.count(), 3);
        assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), 3);
        assert_eq!(pool.game_id(pool.card_idx(1).unwrap()), 2);
        assert_eq!(pool.game_id(pool.card_idx(2).unwrap()), 1);
    }

    #[test]
    fn handler_build_card_pool_end_to_end_minimal() {
        let cards = [
            MasterCard {
                id: 1,
                character_id: 1,
                attr: "cool".to_string(),
                card_rarity_type: 4,
                skill_id: 10,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 2,
                character_id: 2,
                attr: "cute".to_string(),
                card_rarity_type: 4,
                skill_id: 11,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 3,
                character_id: 3,
                attr: "happy".to_string(),
                card_rarity_type: 4,
                skill_id: 12,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
        ];
        let params_table = [
            types::CardParameter {
                card_id: 1,
                level: 1,
                param1: 100,
                param2: 100,
                param3: 100,
            },
            types::CardParameter {
                card_id: 2,
                level: 1,
                param1: 110,
                param2: 110,
                param3: 110,
            },
            types::CardParameter {
                card_id: 3,
                level: 1,
                param1: 120,
                param2: 120,
                param3: 120,
            },
        ];
        let rarities = [types::CardRarity {
            card_rarity_type: 4,
            max_level: 60,
            max_skill_level: 4,
        }];
        let skills = [
            types::Skill {
                id: 10,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 11,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 12,
                level: 1,
                is_after_training: false,
            },
        ];
        let effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 100,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 11,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 110,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 12,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 120,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let units = [
            types::GameCharacterUnit {
                game_character_id: 1,
                unit: "idol".to_string(),
            },
            types::GameCharacterUnit {
                game_character_id: 2,
                unit: "street".to_string(),
            },
            types::GameCharacterUnit {
                game_character_id: 3,
                unit: "themepark".to_string(),
            },
        ];
        let music = [types::MusicMeta {
            music_id: 99,
            event_rate_solo: 100,
            event_rate_multi: 110,
            event_rate_auto: 90,
            base_score: 1.0,
            base_score_auto: 1.0,
            fever_score: 0.0,
            solo_skill_scores: [0.0; 6],
            multi_skill_scores: [0.0; 6],
            auto_skill_scores: [0.0; 6],
            music_time: 100.0,
            tap_count: 500,
        }];
        let game = GameData {
            cards: &cards,
            card_parameters: &params_table,
            card_rarities: &rarities,
            card_episodes: &[],
            master_lessons: &[],
            skills: &skills,
            skill_effects: &effects,
            area_item_levels: &[],
            game_character_units: &units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &music,
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_cards: vec![
                sample_user_card(1),
                sample_user_card(2),
                sample_user_card(3),
            ],
            ..UserProfile::default()
        };
        let params = BuildParams {
            music_id: Some(99),
            live_type: LiveType::Solo,
            target: ScoreTarget::Score,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            ..BuildParams::default()
        };

        let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
        assert_eq!(pool.count(), 3);
        assert_eq!(ctx.music_rate_pct, 100);
        assert_eq!(ctx.target, ScoreTarget::Score);
        assert_eq!(ctx.leader_honor_bonus.len(), 3);
    }
}
