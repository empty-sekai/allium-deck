use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::handler::{
    BondsHonor, CardEpisode, CardMysekaiCanvasBonus, CardParameter, CardRarity, CharacterRank,
    Event, EventCard, EventCardBonusLimit, EventDeckBonus, EventFixtureBonusLimit, EventHonorBonus,
    EventRarityBonusRate, EventSkillScoreUpLimit, GameCharacterUnit, GameData, Honor, HonorLevel,
    MasterCard, MasterLesson, MusicDifficulty, MusicMeta, Skill, SkillEffect, UserAreaItem,
    UserCard, UserChallengeDeck, UserDeck, UserFixtureBonus, UserGateBonus, UserHonor, UserProfile,
    UserWBSupportDeck, WBSupportDeckBonus, WBSupportDeckUnitEventLimitedBonus,
    WorldBloomDiffAttrBonus,
};
use crate::search::{DeckResult, SearchParams};
use crate::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 引擎错误类型。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// JSON 或参数解析失败。
    #[error("参数解析失败: {0}")]
    Parse(#[from] serde_json::Error),
    /// 卡池构建失败。
    #[error("卡池构建失败: {0}")]
    Build(String),
    /// 搜索失败。
    #[error("搜索失败: {0}")]
    Search(String),
}

/// 开源入口：JSON 入、JSON 出。
///
/// `masterdata_json` 当前接受 `OwnedGameData` 的 JSON 表示；`music_metas_json`
/// 可传歌曲元数据数组补齐 `music_metas` 与 `music_difficulties`。
/// `user_data_json` 接受上传链路产出的 camelCase 用户数据。
/// 返回值包含 top-5 decks。
pub fn recommend_json(
    masterdata_json: &str,
    music_metas_json: &str,
    user_data_json: &str,
    params_json: &str,
) -> Result<String, EngineError> {
    let mut owned: OwnedGameData = serde_json::from_str(masterdata_json)?;
    if owned.music_metas.is_empty() && !music_metas_json.trim().is_empty() {
        owned.music_metas = serde_json::from_str(music_metas_json)?;
    }
    if owned.music_difficulties.is_empty() {
        owned.music_difficulties = owned
            .music_metas
            .iter()
            .map(|meta| MusicDifficulty {
                music_id: meta.music_id,
                difficulty: "master".to_string(),
                event_rate: Some(meta.event_rate_solo),
            })
            .collect();
    }

    let user = parse_user_profile_json(user_data_json)?;
    let params = parse_build_params_json(params_json)?;
    let decks = recommend(&user, &owned.as_ref(), &params)?;
    let response = JsonDeckResponse {
        decks: decks.iter().map(JsonDeckResult::from).collect(),
    };
    serde_json::to_string(&response).map_err(EngineError::from)
}

/// 内部入口：结构体入、结构体出，避免请求路径上的 JSON 序列化。
pub fn recommend(
    user: &UserProfile,
    game: &GameData<'_>,
    params: &crate::handler::BuildParams,
) -> Result<Vec<DeckResult>, EngineError> {
    let (pool, ctx) = crate::handler::build_card_pool(user, game, params)
        .map_err(|error| EngineError::Build(error.to_string()))?;
    let search_params = SearchParams {
        top_k: 5,
        timeout_ms: 0,
    };
    Ok(crate::search::search(&pool, &ctx, &search_params))
}

#[derive(Debug, Serialize)]
struct JsonDeckResponse {
    decks: Vec<JsonDeckResult>,
}

#[derive(Debug, Serialize)]
struct JsonDeckResult {
    cards: [usize; 5],
    score: u64,
}

impl From<&DeckResult> for JsonDeckResult {
    fn from(result: &DeckResult) -> Self {
        Self {
            cards: result.cards.map(|card| card.raw()),
            score: result.score,
        }
    }
}

/// 将上传链路的 camelCase 用户数据转换为内部 `UserProfile`。
pub fn parse_user_profile_json(input: &str) -> Result<UserProfile, serde_json::Error> {
    let value = serde_json::from_str::<Value>(input)?;
    Ok(user_profile_from_value(&value))
}

fn user_profile_from_value(value: &Value) -> UserProfile {
    UserProfile {
        user_cards: array(value, "userCards")
            .iter()
            .filter_map(user_card_from_value)
            .collect(),
        user_characters: array(value, "userCharacters")
            .iter()
            .filter_map(|entry| {
                Some(crate::handler::UserCharacter {
                    character_id: i32_field(entry, "characterId")?,
                    character_rank: i32_field(entry, "characterRank").unwrap_or(0),
                })
            })
            .collect(),
        user_area_items: array(value, "userAreas")
            .iter()
            .flat_map(|area| array(area, "areaItems"))
            .filter_map(|item| {
                Some(UserAreaItem {
                    area_item_id: i32_field(&item, "areaItemId")?,
                    level: i32_field(&item, "level").unwrap_or(0),
                })
            })
            .collect(),
        user_decks: array(value, "userDecks")
            .iter()
            .filter_map(user_deck_from_value)
            .collect(),
        user_world_bloom_support_decks: array(value, "userWorldBloomSupportDecks")
            .iter()
            .filter_map(user_wb_support_deck_from_value)
            .collect(),
        user_challenge_live_solo_decks: array(value, "userChallengeLiveSoloDecks")
            .iter()
            .filter_map(|entry| {
                Some(UserChallengeDeck {
                    character_id: i32_field(entry, "characterId")?,
                    card_id: i32_field(entry, "cardId")?,
                })
            })
            .collect(),
        user_mysekai_fixture_bonuses: array(
            value,
            "userMysekaiFixtureGameCharacterPerformanceBonuses",
        )
        .iter()
        .filter_map(|entry| {
            Some(UserFixtureBonus {
                character_id: i32_field(entry, "gameCharacterId")?,
                event_id: i32_field(entry, "eventId"),
                total_bonus_rate: i32_field(entry, "totalBonusRate").unwrap_or(0),
            })
        })
        .collect(),
        user_mysekai_gate_bonuses: array(value, "userMysekaiGates")
            .iter()
            .filter_map(|entry| {
                let gate_id = i32_field(entry, "mysekaiGateId")?;
                let level = i32_field(entry, "mysekaiGateLevel").unwrap_or(0);
                Some(UserGateBonus {
                    unit: gate_unit(gate_id).to_string(),
                    bonus_rate: (level.max(0) as f64) * 0.1,
                })
            })
            .collect(),
        user_mysekai_canvas_bonus_cards: array(value, "userMysekaiCanvases")
            .iter()
            .filter_map(|entry| i32_field(entry, "cardId"))
            .collect(),
        user_honors: array(value, "userHonors")
            .iter()
            .filter_map(|entry| {
                Some(UserHonor {
                    honor_id: i32_field(entry, "honorId")?,
                    level: i32_field(entry, "level").unwrap_or(1),
                })
            })
            .collect(),
    }
}

fn user_card_from_value(value: &Value) -> Option<UserCard> {
    Some(UserCard {
        card_id: i32_field(value, "cardId")?,
        level: i32_field(value, "level").unwrap_or(1),
        skill_level: i32_field(value, "skillLevel").unwrap_or(1),
        master_rank: i32_field(value, "masterRank").unwrap_or(0),
        special_training_status: string_field(value, "specialTrainingStatus")
            .unwrap_or_else(|| "none".to_string()),
        default_image: string_field(value, "defaultImage")
            .unwrap_or_else(|| "original".to_string()),
        episodes_read: array(value, "episodes")
            .iter()
            .filter(|episode| {
                string_field(episode, "scenarioStatus").as_deref() == Some("already_read")
            })
            .filter_map(|episode| i32_field(episode, "cardEpisodeId"))
            .collect(),
        is_virtual: false,
        has_canvas_bonus_override: None,
    })
}

fn user_deck_from_value(value: &Value) -> Option<UserDeck> {
    let cards = (1..=5)
        .filter_map(|index| i32_field(value, &format!("member{index}")))
        .collect::<Vec<_>>();
    Some(UserDeck {
        deck_id: i32_field(value, "deckId").unwrap_or(0),
        cards,
    })
}

fn user_wb_support_deck_from_value(value: &Value) -> Option<UserWBSupportDeck> {
    let cards = array(value, "cardIds")
        .iter()
        .filter_map(|entry| entry.as_i64().map(|value| value as i32))
        .collect::<Vec<_>>();
    Some(UserWBSupportDeck {
        character_id: i32_field(value, "characterId")?,
        cards,
    })
}

fn parse_build_params_json(input: &str) -> Result<crate::handler::BuildParams, serde_json::Error> {
    let value = serde_json::from_str::<Value>(input)?;
    let mut params = crate::handler::BuildParams::default();
    if let Some(region) = string_field(&value, "region") {
        params.region = region;
    }
    params.event_id = i32_field(&value, "eventId").or_else(|| i32_field(&value, "event_id"));
    params.event_type =
        string_field(&value, "eventType").or_else(|| string_field(&value, "event_type"));
    params.live_type = parse_live_type(
        string_field(&value, "liveType")
            .or_else(|| string_field(&value, "live_type"))
            .as_deref()
            .unwrap_or("solo"),
    );
    params.target = parse_target(string_field(&value, "target").as_deref().unwrap_or("score"));
    params.music_id = i32_field(&value, "musicId").or_else(|| i32_field(&value, "music_id"));
    params.music_diff =
        string_field(&value, "musicDiff").or_else(|| string_field(&value, "music_diff"));
    params.fixed_cards = int_array(&value, "fixedCards");
    params.fixed_characters = int_array(&value, "fixedCharacters");
    params.excluded_cards = int_array(&value, "excludedCards");
    params.world_bloom_character_id = i32_field(&value, "worldBloomCharacterId")
        .or_else(|| i32_field(&value, "world_bloom_character_id"));
    params.world_bloom_event_turn = i32_field(&value, "worldBloomEventTurn")
        .or_else(|| i32_field(&value, "world_bloom_event_turn"));
    params.challenge_live_character_id = i32_field(&value, "challengeLiveCharacterId")
        .or_else(|| i32_field(&value, "challenge_live_character_id"));
    params.event_unit =
        string_field(&value, "eventUnit").or_else(|| string_field(&value, "event_unit"));
    params.event_attr =
        string_field(&value, "eventAttr").or_else(|| string_field(&value, "event_attr"));
    params.filter_other_unit = bool_field(&value, "filterOtherUnit").unwrap_or(false);
    params.keep_after_training_state =
        bool_field(&value, "keepAfterTrainingState").unwrap_or(false);
    params.best_skill_as_leader = bool_field(&value, "bestSkillAsLeader").unwrap_or(true);
    params.skill_reference_strategy = parse_skill_reference_strategy(
        string_field(&value, "skillReferenceChooseStrategy")
            .or_else(|| string_field(&value, "skillReferenceStrategy"))
            .as_deref()
            .unwrap_or("average"),
    );
    params.live_skill_order = parse_live_skill_order(
        string_field(&value, "liveSkillOrder")
            .as_deref()
            .unwrap_or("best"),
    );
    params.multi_teammate_score_up = i32_field(&value, "multiLiveTeammateScoreUp")
        .or_else(|| i32_field(&value, "multi_teammate_score_up"));
    params.multi_teammate_power = i32_field(&value, "multiLiveTeammatePower")
        .or_else(|| i32_field(&value, "multi_teammate_power"));
    params.multi_live_score_up_lower_bound = value
        .get("multiLiveScoreUpLowerBound")
        .or_else(|| value.get("multi_live_score_up_lower_bound"))
        .and_then(Value::as_f64);
    params.boost = i32_field(&value, "boost");
    params.other_score =
        i32_field(&value, "otherScore").or_else(|| i32_field(&value, "other_score"));
    params.life = i32_field(&value, "life");
    params.unit_filter =
        string_field(&value, "unitFilter").or_else(|| string_field(&value, "unit_filter"));
    params.attr_filter =
        string_field(&value, "attrFilter").or_else(|| string_field(&value, "attr_filter"));
    Ok(params)
}

fn array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn int_array(value: &Value, key: &str) -> Vec<i32> {
    array(value, key)
        .iter()
        .filter_map(|entry| entry.as_i64().map(|value| value as i32))
        .collect()
}

fn i32_field(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .map(|value| value as i32)
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn gate_unit(gate_id: i32) -> &'static str {
    match gate_id {
        1 => "light_sound",
        2 => "idol",
        3 => "street",
        4 => "theme_park",
        5 => "school_refusal",
        _ => "piapro",
    }
}

fn parse_live_type(value: &str) -> LiveType {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => LiveType::Auto,
        "multi" => LiveType::Multi,
        "cheerful" => LiveType::Cheerful,
        "challenge" => LiveType::Challenge,
        "challenge_auto" | "challengeauto" => LiveType::ChallengeAuto,
        "mysekai" => LiveType::Mysekai,
        _ => LiveType::Solo,
    }
}

fn parse_target(value: &str) -> ScoreTarget {
    match value.trim().to_ascii_lowercase().as_str() {
        "power" => ScoreTarget::Power,
        "skill" => ScoreTarget::Skill,
        "mysekai" => ScoreTarget::Mysekai,
        _ => ScoreTarget::Score,
    }
}

fn parse_skill_reference_strategy(value: &str) -> SkillReferenceStrategy {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" => SkillReferenceStrategy::Max,
        "min" => SkillReferenceStrategy::Min,
        _ => SkillReferenceStrategy::Average,
    }
}

fn parse_live_skill_order(value: &str) -> LiveSkillOrder {
    match value.trim().to_ascii_lowercase().as_str() {
        "worst" => LiveSkillOrder::Worst,
        "average" => LiveSkillOrder::Average,
        "specific" => LiveSkillOrder::Specific,
        _ => LiveSkillOrder::Best,
    }
}

/// 持有 `GameData<'_>` 借用所需的所有 `Vec<T>`。
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct OwnedGameData {
    pub cards: Vec<MasterCard>,
    pub card_parameters: Vec<CardParameter>,
    pub card_rarities: Vec<CardRarity>,
    pub card_episodes: Vec<CardEpisode>,
    pub master_lessons: Vec<MasterLesson>,
    pub skills: Vec<Skill>,
    pub skill_effects: Vec<SkillEffect>,
    pub area_item_levels: Vec<crate::handler::AreaItemLevel>,
    pub game_character_units: Vec<GameCharacterUnit>,
    pub character_ranks: Vec<CharacterRank>,
    pub card_mysekai_canvas_bonuses: Vec<CardMysekaiCanvasBonus>,
    pub events: Vec<Event>,
    pub event_cards: Vec<EventCard>,
    pub event_deck_bonuses: Vec<EventDeckBonus>,
    pub event_card_bonus_limits: Vec<EventCardBonusLimit>,
    pub event_honor_bonuses: Vec<EventHonorBonus>,
    pub world_bloom_different_attribute_bonuses: Vec<WorldBloomDiffAttrBonus>,
    pub wb_support_deck_bonuses_wl1: Vec<WBSupportDeckBonus>,
    pub wb_support_deck_bonuses_wl2: Vec<WBSupportDeckBonus>,
    pub wb_support_deck_bonuses_wl3: Vec<WBSupportDeckBonus>,
    pub world_bloom_support_deck_unit_event_limited_bonuses:
        Vec<WBSupportDeckUnitEventLimitedBonus>,
    pub event_mysekai_fixture_performance_bonus_limits: Vec<EventFixtureBonusLimit>,
    pub event_skill_score_up_limits: Vec<EventSkillScoreUpLimit>,
    pub music_metas: Vec<MusicMeta>,
    pub music_difficulties: Vec<MusicDifficulty>,
    pub event_rarity_bonus_rates: Vec<EventRarityBonusRate>,
    pub honors: Vec<Honor>,
    pub bonds_honors: Vec<BondsHonor>,
}

impl OwnedGameData {
    /// 从磁盘加载 masterdata 和 music metas。
    pub fn load(masterdata_dir: &Path, music_metas_path: &Path) -> Result<Self, String> {
        let raw_game_character_units: Vec<RawGameCharacterUnit> =
            load_json(&masterdata_dir.join("gameCharacterUnits.json"))?;
        let raw_cards: Vec<RawCard> = load_json(&masterdata_dir.join("cards.json"))?;
        let events: Vec<RawEvent> = load_json(&masterdata_dir.join("events.json"))?;

        let skill_unit_map = infer_skill_units(&raw_cards, &raw_game_character_units);
        let event_ids = events.iter().map(|event| event.id).collect::<Vec<_>>();

        let music_rows: Vec<RawMusicMetaRow> = load_json(music_metas_path)?;
        let master_music_rows = music_rows
            .into_iter()
            .filter(|row| row.difficulty.eq_ignore_ascii_case("master"))
            .collect::<Vec<_>>();

        Ok(Self {
            cards: raw_cards
                .iter()
                .map(|card| MasterCard {
                    id: card.id,
                    character_id: card.character_id,
                    attr: card.attr.clone(),
                    card_rarity_type: rarity_type_to_index(&card.card_rarity_type),
                    skill_id: card.skill_id,
                    special_training_skill_id: card.special_training_skill_id,
                    special_training_power1_bonus_fixed: card.special_training_power1_bonus_fixed,
                    special_training_power2_bonus_fixed: card.special_training_power2_bonus_fixed,
                    special_training_power3_bonus_fixed: card.special_training_power3_bonus_fixed,
                    support_unit: normalize_unit_string(card.support_unit.as_deref()),
                    max_level: None,
                    max_skill_level: None,
                    max_master_rank: None,
                })
                .collect(),
            card_parameters: raw_cards.iter().flat_map(flatten_card_parameters).collect(),
            card_rarities: load_json::<Vec<RawCardRarity>>(
                &masterdata_dir.join("cardRarities.json"),
            )?
            .into_iter()
            .map(|rarity| CardRarity {
                card_rarity_type: rarity_type_to_index(&rarity.card_rarity_type),
                max_level: rarity.max_level,
                max_skill_level: rarity.max_skill_level,
            })
            .collect(),
            card_episodes: load_json::<Vec<RawCardEpisode>>(
                &masterdata_dir.join("cardEpisodes.json"),
            )?
            .into_iter()
            .map(|episode| CardEpisode {
                card_id: episode.card_id,
                episode_no: episode.id,
                power1_bonus_fixed: episode.power1_bonus_fixed,
                power2_bonus_fixed: episode.power2_bonus_fixed,
                power3_bonus_fixed: episode.power3_bonus_fixed,
            })
            .collect(),
            master_lessons: load_json::<Vec<RawMasterLesson>>(
                &masterdata_dir.join("masterLessons.json"),
            )?
            .into_iter()
            .map(|lesson| MasterLesson {
                card_rarity_type: rarity_type_to_index(&lesson.card_rarity_type),
                master_rank: lesson.master_rank,
                power1_bonus_fixed: lesson.power1_bonus_fixed,
                power2_bonus_fixed: lesson.power2_bonus_fixed,
                power3_bonus_fixed: lesson.power3_bonus_fixed,
            })
            .collect(),
            skills: flatten_skills(
                &load_json::<Vec<RawSkill>>(&masterdata_dir.join("skills.json"))?,
                &skill_unit_map,
            )
            .0,
            skill_effects: flatten_skills(
                &load_json::<Vec<RawSkill>>(&masterdata_dir.join("skills.json"))?,
                &skill_unit_map,
            )
            .1,
            area_item_levels: flatten_area_item_levels(load_json::<Vec<RawAreaItemLevel>>(
                &masterdata_dir.join("areaItemLevels.json"),
            )?),
            game_character_units: raw_game_character_units
                .iter()
                .map(|entry| GameCharacterUnit {
                    game_character_id: entry.game_character_id,
                    unit: entry.unit.clone(),
                })
                .collect(),
            character_ranks: load_json::<Vec<RawCharacterRank>>(
                &masterdata_dir.join("characterRanks.json"),
            )?
            .into_iter()
            .map(|rank| CharacterRank {
                character_rank: rank.character_rank,
                power_bonus_rate: rank.power1_bonus_rate,
            })
            .collect(),
            card_mysekai_canvas_bonuses: load_json::<Vec<RawCardMysekaiCanvasBonus>>(
                &masterdata_dir.join("cardMysekaiCanvasBonuses.json"),
            )?
            .into_iter()
            .map(|entry| CardMysekaiCanvasBonus {
                card_rarity_type: rarity_type_to_index(&entry.card_rarity_type),
                power1_bonus_fixed: entry.power1_bonus_fixed,
                power2_bonus_fixed: entry.power2_bonus_fixed,
                power3_bonus_fixed: entry.power3_bonus_fixed,
            })
            .collect(),
            events: events
                .into_iter()
                .map(|event| Event {
                    id: event.id,
                    event_type: event.event_type,
                })
                .collect(),
            event_cards: load_json::<Vec<RawEventCard>>(&masterdata_dir.join("eventCards.json"))?
                .into_iter()
                .map(|entry| EventCard {
                    event_id: entry.event_id,
                    card_id: entry.card_id,
                    bonus_rate: entry.bonus_rate.round() as i32,
                    leader_bonus_rate: entry.leader_bonus_rate.round() as i32,
                })
                .collect(),
            event_deck_bonuses: load_json::<Vec<RawEventDeckBonus>>(
                &masterdata_dir.join("eventDeckBonuses.json"),
            )?
            .into_iter()
            .map(|entry| {
                let mapped_unit = entry
                    .game_character_unit_id
                    .and_then(|id| raw_game_character_units.iter().find(|unit| unit.id == id));
                EventDeckBonus {
                    event_id: entry.event_id,
                    character_id: mapped_unit.map(|unit| unit.game_character_id),
                    unit: mapped_unit.map(|unit| unit.unit.clone()),
                    attr: entry.card_attr,
                    bonus_rate: entry.bonus_rate.round() as i32,
                }
            })
            .collect(),
            event_card_bonus_limits: load_json::<Vec<RawEventCardBonusLimit>>(
                &masterdata_dir.join("eventCardBonusLimits.json"),
            )?
            .into_iter()
            .map(|entry| EventCardBonusLimit {
                event_id: entry.event_id,
                member_count_limit: entry.member_count_limit,
            })
            .collect(),
            event_honor_bonuses: load_json::<Vec<RawEventHonorBonus>>(
                &masterdata_dir.join("eventHonorBonuses.json"),
            )?
            .into_iter()
            .map(|entry| EventHonorBonus {
                event_id: entry.event_id,
                honor_id: entry.honor_id,
                leader_game_character_id: entry.leader_game_character_id,
                bonus_rate: entry.bonus_rate,
            })
            .collect(),
            world_bloom_different_attribute_bonuses: load_json::<Vec<RawWorldBloomDiffAttrBonus>>(
                &masterdata_dir.join("worldBloomDifferentAttributeBonuses.json"),
            )?
            .into_iter()
            .map(|entry| WorldBloomDiffAttrBonus {
                attr_count: entry.attr_count,
                bonus_rate: entry.bonus_rate.round() as i32,
            })
            .collect(),
            wb_support_deck_bonuses_wl1: Vec::<WBSupportDeckBonus>::new(),
            wb_support_deck_bonuses_wl2: Vec::<WBSupportDeckBonus>::new(),
            wb_support_deck_bonuses_wl3: Vec::<WBSupportDeckBonus>::new(),
            world_bloom_support_deck_unit_event_limited_bonuses: Vec::<
                WBSupportDeckUnitEventLimitedBonus,
            >::new(),
            event_mysekai_fixture_performance_bonus_limits: load_optional_json::<
                Vec<RawEventFixtureBonusLimit>,
            >(
                &masterdata_dir.join("eventMysekaiFixtureGameCharacterPerformanceBonusLimits.json"),
            )?
            .into_iter()
            .map(|entry| EventFixtureBonusLimit {
                event_id: entry.event_id,
                bonus_rate_limit: entry.bonus_rate_limit,
            })
            .collect(),
            event_skill_score_up_limits: load_json::<Vec<RawEventSkillScoreUpLimit>>(
                &masterdata_dir.join("eventSkillScoreUpLimits.json"),
            )?
            .into_iter()
            .map(|entry| EventSkillScoreUpLimit {
                event_id: entry.event_id,
                score_up_limit: entry.score_up_rate_limit,
            })
            .collect(),
            music_metas: master_music_rows
                .iter()
                .map(|row| MusicMeta {
                    music_id: row.music_id,
                    event_rate_solo: row.event_rate,
                    event_rate_multi: row.event_rate,
                    event_rate_auto: row.event_rate,
                    base_score: row.base_score,
                    base_score_auto: row.base_score_auto,
                    fever_score: row.fever_score,
                    solo_skill_scores: row.skill_score_solo,
                    multi_skill_scores: row.skill_score_multi,
                    auto_skill_scores: row.skill_score_auto,
                    music_time: row.music_time,
                    tap_count: row.tap_count,
                })
                .collect(),
            music_difficulties: master_music_rows
                .iter()
                .map(|row| MusicDifficulty {
                    music_id: row.music_id,
                    difficulty: row.difficulty.clone(),
                    event_rate: Some(row.event_rate),
                })
                .collect(),
            event_rarity_bonus_rates: load_json::<Vec<RawEventRarityBonusRate>>(
                &masterdata_dir.join("eventRarityBonusRates.json"),
            )?
            .into_iter()
            .flat_map(|entry| {
                event_ids
                    .iter()
                    .copied()
                    .map(move |event_id| EventRarityBonusRate {
                        event_id,
                        card_rarity_type: rarity_type_to_index(&entry.card_rarity_type),
                        master_rank: entry.master_rank,
                        bonus_rate: entry.bonus_rate.round() as i32,
                    })
            })
            .collect(),
            honors: load_optional_json::<Vec<RawHonor>>(&masterdata_dir.join("honors.json"))?
                .into_iter()
                .map(|entry| Honor {
                    id: entry.id,
                    levels: entry
                        .levels
                        .into_iter()
                        .map(|lv| HonorLevel {
                            level: lv.level,
                            bonus: lv.bonus,
                        })
                        .collect(),
                })
                .collect(),
            bonds_honors: load_optional_json::<Vec<RawIdOnly>>(
                &masterdata_dir.join("bondsHonors.json"),
            )?
            .into_iter()
            .map(|entry| BondsHonor { id: entry.id })
            .collect(),
        })
    }

    /// 借用为 `GameData<'_>`。
    pub fn as_ref(&self) -> GameData<'_> {
        GameData {
            cards: &self.cards,
            card_parameters: &self.card_parameters,
            card_rarities: &self.card_rarities,
            card_episodes: &self.card_episodes,
            master_lessons: &self.master_lessons,
            skills: &self.skills,
            skill_effects: &self.skill_effects,
            area_item_levels: &self.area_item_levels,
            game_character_units: &self.game_character_units,
            character_ranks: &self.character_ranks,
            card_mysekai_canvas_bonuses: &self.card_mysekai_canvas_bonuses,
            events: &self.events,
            event_cards: &self.event_cards,
            event_deck_bonuses: &self.event_deck_bonuses,
            event_card_bonus_limits: &self.event_card_bonus_limits,
            event_honor_bonuses: &self.event_honor_bonuses,
            world_bloom_different_attribute_bonuses: &self.world_bloom_different_attribute_bonuses,
            wb_support_deck_bonuses_wl1: &self.wb_support_deck_bonuses_wl1,
            wb_support_deck_bonuses_wl2: &self.wb_support_deck_bonuses_wl2,
            wb_support_deck_bonuses_wl3: &self.wb_support_deck_bonuses_wl3,
            world_bloom_support_deck_unit_event_limited_bonuses: &self
                .world_bloom_support_deck_unit_event_limited_bonuses,
            event_mysekai_fixture_performance_bonus_limits: &self
                .event_mysekai_fixture_performance_bonus_limits,
            event_skill_score_up_limits: &self.event_skill_score_up_limits,
            music_metas: &self.music_metas,
            music_difficulties: &self.music_difficulties,
            event_rarity_bonus_rates: &self.event_rarity_bonus_rates,
            honors: &self.honors,
            bonds_honors: &self.bonds_honors,
        }
    }
}

fn flatten_card_parameters(card: &RawCard) -> Vec<CardParameter> {
    let len = card
        .card_parameters
        .param1
        .len()
        .min(card.card_parameters.param2.len())
        .min(card.card_parameters.param3.len());
    (0..len)
        .map(|index| CardParameter {
            card_id: card.id,
            level: index as i32 + 1,
            param1: card.card_parameters.param1[index],
            param2: card.card_parameters.param2[index],
            param3: card.card_parameters.param3[index],
        })
        .collect()
}

fn flatten_area_item_levels(raw: Vec<RawAreaItemLevel>) -> Vec<crate::handler::AreaItemLevel> {
    let mut raw = raw;
    raw.sort_by(|left, right| {
        (
            left.area_item_id,
            normalize_target_token(left.target_unit.as_deref()),
            normalize_target_token(left.target_card_attr.as_deref()),
            left.target_game_character_id,
            left.level,
        )
            .cmp(&(
                right.area_item_id,
                normalize_target_token(right.target_unit.as_deref()),
                normalize_target_token(right.target_card_attr.as_deref()),
                right.target_game_character_id,
                right.level,
            ))
    });

    let mut result = Vec::with_capacity(raw.len());
    for item in raw {
        let unit = normalize_target_token(item.target_unit.as_deref());
        let attr = normalize_target_token(item.target_card_attr.as_deref());

        result.push(crate::handler::AreaItemLevel {
            area_item_id: item.area_item_id,
            level: item.level,
            unit,
            attr,
            character_id: item.target_game_character_id,
            power_rate: item.power1_bonus_rate,
            power_all_match_rate: item.power1_all_match_bonus_rate,
        });
    }
    result
}

fn infer_skill_units(
    cards: &[RawCard],
    game_character_units: &[RawGameCharacterUnit],
) -> BTreeMap<i32, String> {
    let unit_map = game_character_units
        .iter()
        .map(|entry| (entry.game_character_id, entry.unit.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut result = BTreeMap::new();
    for card in cards {
        let Some(primary) = unit_map.get(&card.character_id) else {
            continue;
        };
        let target_unit = if primary == "piapro" {
            normalize_unit_string(card.support_unit.as_deref()).unwrap_or_else(|| primary.clone())
        } else {
            primary.clone()
        };
        result.entry(card.skill_id).or_insert(target_unit);
    }
    result
}

fn flatten_skills(
    skills: &[RawSkill],
    skill_unit_map: &BTreeMap<i32, String>,
) -> (Vec<Skill>, Vec<SkillEffect>) {
    let mut skill_rows = Vec::new();
    let mut effect_rows = Vec::new();

    for skill in skills {
        let mut by_level = BTreeMap::<i32, LevelSkillEffects>::new();

        for effect in &skill.skill_effects {
            for detail in &effect.skill_effect_details {
                let entry = by_level.entry(detail.level).or_default();
                match effect.skill_effect_type.as_str() {
                    "score_up" | "score_up_keep" | "score_up_condition_life" => {
                        entry.score_up = Some(
                            entry
                                .score_up
                                .unwrap_or(0)
                                .max(detail.activate_effect_value),
                        );
                        if let Some(enhance) = &effect.skill_enhance {
                            let unit = enhance
                                .skill_enhance_condition
                                .as_ref()
                                .map(|condition| condition.unit.clone());
                            entry.same_unit = Some((enhance.activate_effect_value, unit));
                        }
                    }
                    "life_recovery" => {
                        entry.life_recovery =
                            Some(entry.life_recovery.unwrap_or(0) + detail.activate_effect_value);
                    }
                    "score_up_character_rank" => {
                        if let Some(rank) = effect.activate_character_rank {
                            entry
                                .character_rank_bonus
                                .push((rank, detail.activate_effect_value));
                        }
                    }
                    "other_member_score_up_reference_rate" => {
                        entry.ref_rate = Some(detail.activate_effect_value);
                        entry.ref_max = detail.activate_effect_value2;
                    }
                    "score_up_unit_count" => {
                        if let Some(count) = effect.activate_unit_count {
                            entry.diff_count.push((count, detail.activate_effect_value));
                        }
                    }
                    _ => {}
                }
            }
        }

        for (level, effects) in by_level {
            skill_rows.push(Skill {
                id: skill.id,
                level,
                is_after_training: false,
            });
            if let Some(score_up) = effects.score_up {
                effect_rows.push(SkillEffect {
                    skill_id: skill.id,
                    skill_level: level,
                    effect_type: "score_up".to_string(),
                    value: score_up,
                    additional_value: None,
                    unit_member_count: None,
                    unit: None,
                    activate_character_rank: None,
                });
                if let Some((increment, unit)) = effects.same_unit {
                    for count in 1..=5 {
                        let multiplier = if count == 5 { 5 } else { count - 1 };
                        effect_rows.push(SkillEffect {
                            skill_id: skill.id,
                            skill_level: level,
                            effect_type: "score_up_unit_count".to_string(),
                            value: score_up + multiplier * increment,
                            additional_value: None,
                            unit_member_count: Some(count),
                            unit: unit
                                .clone()
                                .or_else(|| skill_unit_map.get(&skill.id).cloned()),
                            activate_character_rank: None,
                        });
                    }
                }
            }
            if let Some(life_recovery) = effects.life_recovery {
                effect_rows.push(SkillEffect {
                    skill_id: skill.id,
                    skill_level: level,
                    effect_type: "life_recovery".to_string(),
                    value: life_recovery,
                    additional_value: None,
                    unit_member_count: None,
                    unit: None,
                    activate_character_rank: None,
                });
            }
            for (rank, value) in effects.character_rank_bonus {
                effect_rows.push(SkillEffect {
                    skill_id: skill.id,
                    skill_level: level,
                    effect_type: "score_up_character_rank".to_string(),
                    value,
                    additional_value: None,
                    unit_member_count: None,
                    unit: None,
                    activate_character_rank: Some(rank),
                });
            }
            if let Some(ref_rate) = effects.ref_rate {
                effect_rows.push(SkillEffect {
                    skill_id: skill.id,
                    skill_level: level,
                    effect_type: "score_up_reference".to_string(),
                    value: ref_rate,
                    additional_value: effects.ref_max,
                    unit_member_count: None,
                    unit: None,
                    activate_character_rank: None,
                });
            }
            if let Some(score_up) = effects.score_up {
                let mut diff_values = effects.diff_count;
                diff_values.sort_unstable_by_key(|(count, _)| *count);
                if let Some((_, first_value)) = diff_values.first().copied() {
                    effect_rows.push(SkillEffect {
                        skill_id: skill.id,
                        skill_level: level,
                        effect_type: "score_up_diff".to_string(),
                        value: score_up,
                        additional_value: Some(first_value),
                        unit_member_count: None,
                        unit: None,
                        activate_character_rank: None,
                    });
                }
            }
            for (count, value, unit) in effects.unit_count {
                effect_rows.push(SkillEffect {
                    skill_id: skill.id,
                    skill_level: level,
                    effect_type: "score_up_unit_count".to_string(),
                    value,
                    additional_value: None,
                    unit_member_count: Some(count),
                    unit,
                    activate_character_rank: None,
                });
            }
        }
    }

    (skill_rows, effect_rows)
}

fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("读取 {} 失败: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("解析 {} 失败: {err}", path.display()))
}

fn load_optional_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    load_json(path)
}

fn normalize_unit_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "none")
        .map(ToOwned::to_owned)
}

fn normalize_target_token(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "none" && *value != "any")
        .map(ToOwned::to_owned)
}

fn rarity_type_to_index(value: &str) -> i32 {
    match value.trim().to_ascii_lowercase().as_str() {
        "rarity_1" => 1,
        "rarity_2" => 2,
        "rarity_3" => 3,
        "rarity_4" => 4,
        "rarity_birthday" | "birthday" => 5,
        _ => 4,
    }
}

#[derive(Debug, Clone, Default)]
struct LevelSkillEffects {
    score_up: Option<i32>,
    life_recovery: Option<i32>,
    ref_rate: Option<i32>,
    ref_max: Option<i32>,
    same_unit: Option<(i32, Option<String>)>,
    character_rank_bonus: Vec<(i32, i32)>,
    diff_count: Vec<(i32, i32)>,
    unit_count: Vec<(i32, i32, Option<String>)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCard {
    id: i32,
    character_id: i32,
    card_rarity_type: String,
    attr: String,
    #[serde(default)]
    support_unit: Option<String>,
    skill_id: i32,
    #[serde(default)]
    special_training_skill_id: Option<i32>,
    #[serde(default)]
    special_training_power1_bonus_fixed: i32,
    #[serde(default)]
    special_training_power2_bonus_fixed: i32,
    #[serde(default)]
    special_training_power3_bonus_fixed: i32,
    card_parameters: RawCardParameters,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawCardParameters {
    #[serde(default)]
    param1: Vec<i32>,
    #[serde(default)]
    param2: Vec<i32>,
    #[serde(default)]
    param3: Vec<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCardRarity {
    card_rarity_type: String,
    max_level: i32,
    max_skill_level: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCardEpisode {
    id: i32,
    card_id: i32,
    power1_bonus_fixed: i32,
    power2_bonus_fixed: i32,
    power3_bonus_fixed: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMasterLesson {
    card_rarity_type: String,
    master_rank: i32,
    power1_bonus_fixed: i32,
    power2_bonus_fixed: i32,
    power3_bonus_fixed: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSkill {
    id: i32,
    #[serde(default)]
    skill_effects: Vec<RawSkillEffect>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSkillEffect {
    skill_effect_type: String,
    #[serde(default)]
    activate_unit_count: Option<i32>,
    #[serde(default)]
    activate_character_rank: Option<i32>,
    #[serde(default)]
    skill_enhance: Option<RawSkillEnhance>,
    #[serde(default)]
    skill_effect_details: Vec<RawSkillEffectDetail>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSkillEnhance {
    activate_effect_value: i32,
    #[serde(default)]
    skill_enhance_condition: Option<RawSkillEnhanceCondition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSkillEnhanceCondition {
    unit: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSkillEffectDetail {
    level: i32,
    activate_effect_value: i32,
    #[serde(default)]
    activate_effect_value2: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAreaItemLevel {
    area_item_id: i32,
    level: i32,
    #[serde(default)]
    target_unit: Option<String>,
    #[serde(default)]
    target_card_attr: Option<String>,
    #[serde(default)]
    target_game_character_id: Option<i32>,
    power1_bonus_rate: f64,
    power1_all_match_bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGameCharacterUnit {
    id: i32,
    game_character_id: i32,
    unit: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCharacterRank {
    character_rank: i32,
    power1_bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCardMysekaiCanvasBonus {
    card_rarity_type: String,
    power1_bonus_fixed: i32,
    power2_bonus_fixed: i32,
    power3_bonus_fixed: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEvent {
    id: i32,
    event_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventCard {
    card_id: i32,
    event_id: i32,
    bonus_rate: f64,
    leader_bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventDeckBonus {
    event_id: i32,
    #[serde(default)]
    game_character_unit_id: Option<i32>,
    #[serde(default)]
    card_attr: Option<String>,
    bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventCardBonusLimit {
    event_id: i32,
    member_count_limit: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventHonorBonus {
    event_id: i32,
    honor_id: i32,
    leader_game_character_id: i32,
    bonus_rate: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWorldBloomDiffAttrBonus {
    #[serde(rename = "attributeCount")]
    attr_count: i32,
    bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventFixtureBonusLimit {
    event_id: i32,
    bonus_rate_limit: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventSkillScoreUpLimit {
    event_id: i32,
    score_up_rate_limit: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct RawMusicMetaRow {
    music_id: i32,
    difficulty: String,
    music_time: f64,
    event_rate: i32,
    base_score: f64,
    base_score_auto: f64,
    skill_score_solo: [f64; 6],
    skill_score_auto: [f64; 6],
    skill_score_multi: [f64; 6],
    fever_score: f64,
    tap_count: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventRarityBonusRate {
    card_rarity_type: String,
    master_rank: i32,
    bonus_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct RawIdOnly {
    id: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHonor {
    id: i32,
    #[serde(default)]
    levels: Vec<RawHonorLevel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHonorLevel {
    level: i32,
    #[serde(default)]
    bonus: i32,
}
