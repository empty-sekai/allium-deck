use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::handler::{
    BondsHonor, CardEpisode, CardMysekaiCanvasBonus, CardParameter, CardRarity, CharacterRank,
    Event, EventCard, EventCardBonusLimit, EventDeckBonus, EventFixtureBonusLimit, EventHonorBonus,
    EventRarityBonusRate, EventSkillScoreUpLimit, GameCharacterUnit, GameData, Honor, HonorLevel,
    MasterCard, MasterLesson, MusicDifficulty, MusicMeta, MysekaiGate, MysekaiGateLevel, Skill,
    SkillEffect, UserAreaItem, UserCard, UserChallengeDeck, UserDeck, UserFixtureBonus,
    UserGateBonus, UserHonor, UserProfile, UserWBSupportDeck, WBSupportDeckBonus,
    WBSupportDeckUnitEventLimitedBonus, WorldBloom, WorldBloomDiffAttrBonus,
};
use crate::search::{DeckResult, SearchParams};
use crate::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};
use serde::de::{DeserializeOwned, Error as _};
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
    let build = crate::handler::build_card_pool(user, game, params);
    let (pool, ctx) = match build {
        Ok(ok) => ok,
        // 精确档位组卡：候选不足等价于「所有目标档位都不可达」，
        // 返回空结果而非错误（与逐档搜索的空 bucket 行为一致）。
        Err(crate::handler::BuildError::EmptyPool) if !params.target_bonus_list.is_empty() => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(EngineError::Build(error.to_string())),
    };
    let search_params = SearchParams {
        top_k: params.limit,
        timeout_ms: params.timeout_ms,
    };
    Ok(crate::search::search_targets(
        &pool,
        &ctx,
        &search_params,
        &params.target_bonus_list,
    ))
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
                    mysekai_gate_id: Some(gate_id),
                    mysekai_gate_level: Some(level),
                    unit: String::new(),
                    bonus_rate: 0.0,
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

/// 将 camelCase params JSON 解析为内部 `BuildParams`（standalone CLI / 开源入口共用）。
pub fn parse_build_params_json(
    input: &str,
) -> Result<crate::handler::BuildParams, serde_json::Error> {
    let value = serde_json::from_str::<Value>(input)?;
    if !value.is_object() {
        return Err(serde_json::Error::custom("参数必须是 JSON 对象"));
    }
    let mut params = crate::handler::BuildParams::default();
    if let Some(region) = string_field(&value, "region") {
        params.region = region;
    }
    params.event_id =
        i32_field_checked(&value, "eventId")?.or(i32_field_checked(&value, "event_id")?);
    params.event_type =
        string_field(&value, "eventType").or_else(|| string_field(&value, "event_type"));
    params.live_type = parse_live_type_checked(
        string_field(&value, "liveType")
            .or_else(|| string_field(&value, "live_type"))
            .as_deref()
            .unwrap_or("solo"),
    )?;
    params.target = parse_target_checked(field_alias_checked(&value, "target", "target")?)?;
    params.limit = bounded_usize_field(
        &value,
        "limit",
        "limit",
        1,
        crate::handler::types::MAX_BUILD_LIMIT,
        false,
    )?
    .unwrap_or(10);
    params.member = bounded_usize_field(
        &value,
        "member",
        "member",
        crate::types::DECK_SIZE,
        crate::types::DECK_SIZE,
        true,
    )?;
    params.timeout_ms = bounded_u64_field(
        &value,
        "timeoutMs",
        "timeout_ms",
        1,
        crate::handler::types::MAX_BUILD_TIMEOUT_MS,
    )?
    .unwrap_or(crate::handler::types::MAX_BUILD_TIMEOUT_MS);
    params.target_bonus_list = bounded_int_array_alias(
        &value,
        "targetBonusList",
        "target_bonus_list",
        0,
        crate::handler::types::MAX_TARGET_BONUS,
        crate::handler::types::MAX_TARGET_BONUS_BUCKETS,
    )?;
    params.minimize = bool_field(&value, "minimize").unwrap_or(false);
    params.music_id =
        i32_field_checked(&value, "musicId")?.or(i32_field_checked(&value, "music_id")?);
    params.music_diff =
        string_field(&value, "musicDiff").or_else(|| string_field(&value, "music_diff"));
    params.fixed_cards = int_array_alias(&value, "fixedCards", "fixed_cards");
    params.fixed_characters = int_array_alias(&value, "fixedCharacters", "fixed_characters");
    params.forced_leader_character_id = i32_field_checked(&value, "forcedLeaderCharacterId")?
        .or(i32_field_checked(&value, "forced_leader_character_id")?);
    params.excluded_cards = int_array_alias(&value, "excludedCards", "excluded_cards");
    params.world_bloom_character_id = i32_field_checked(&value, "worldBloomCharacterId")?
        .or(i32_field_checked(&value, "world_bloom_character_id")?);
    params.world_bloom_event_turn = i32_field_checked(&value, "worldBloomEventTurn")?
        .or(i32_field_checked(&value, "world_bloom_event_turn")?);
    params.world_bloom_finale_turn = i32_field_checked(&value, "worldBloomFinaleTurn")?
        .or(i32_field_checked(&value, "world_bloom_finale_turn")?);
    params.challenge_live_character_id = i32_field_checked(&value, "challengeLiveCharacterId")?
        .or(i32_field_checked(&value, "challenge_live_character_id")?);
    params.event_unit =
        string_field(&value, "eventUnit").or_else(|| string_field(&value, "event_unit"));
    params.event_attr =
        string_field(&value, "eventAttr").or_else(|| string_field(&value, "event_attr"));
    validate_optional_enum(
        params.event_type.as_deref(),
        "event_type",
        &[
            "marathon",
            "cheerful",
            "cheerful_carnival",
            "cheerfulcarnival",
            "world_bloom",
            "worldbloom",
            "wl",
        ],
    )?;
    validate_optional_enum(
        params.event_attr.as_deref(),
        "event_attr",
        &["mysterious", "cute", "cool", "pure", "happy"],
    )?;
    // 词表单一权威：与 unit_filter 同用 parse_unit_code，
    // 避免 event_unit 拒绝而 unit_filter 接受同一别名的分叉。
    if let Some(unit) = params.event_unit.as_deref()
        && crate::handler::types::parse_unit_code(unit).is_none()
    {
        return Err(serde_json::Error::custom(format!(
            "event_unit 非法: {unit}"
        )));
    }
    params.custom_bonus_character_ids = bounded_int_array_alias(
        &value,
        "customBonusCharacterIds",
        "custom_bonus_character_ids",
        1,
        26,
        26,
    )?;
    params.custom_bonus_attr =
        optional_string_alias_checked(&value, "customBonusAttr", "custom_bonus_attr")?;
    params.custom_bonus_character_support_units = parse_custom_bonus_support_units(&value)?;
    params.filter_other_unit = bool_field(&value, "filterOtherUnit").unwrap_or(false);
    params.support_master_max = bool_field(&value, "supportMasterMax")
        .or_else(|| bool_field(&value, "support_master_max"))
        .unwrap_or(false);
    params.support_skill_max = bool_field(&value, "supportSkillMax")
        .or_else(|| bool_field(&value, "support_skill_max"))
        .unwrap_or(false);
    params.keep_after_training_state =
        bool_field(&value, "keepAfterTrainingState").unwrap_or(false);
    params.best_skill_as_leader = bool_field(&value, "bestSkillAsLeader").unwrap_or(true);
    params.skill_reference_strategy = parse_skill_reference_strategy_checked(
        string_field(&value, "skillReferenceChooseStrategy")
            .or_else(|| string_field(&value, "skillReferenceStrategy"))
            .as_deref()
            .unwrap_or("average"),
    )?;
    params.live_skill_order = parse_live_skill_order_checked(
        string_field(&value, "liveSkillOrder")
            .or_else(|| string_field(&value, "skillOrderChooseStrategy"))
            .or_else(|| string_field(&value, "skill_order_choose_strategy"))
            .as_deref()
            // 技能发动顺序在游戏内不可控，期望值按平均计；max/min/specific
            // 供调用方做上界/下界/指定顺序估算。
            .unwrap_or("average"),
    )?;
    params.specific_skill_order = parse_specific_skill_order(&value)?;
    params.multi_teammate_score_up = i32_field_checked(&value, "multiLiveTeammateScoreUp")?
        .or(i32_field_checked(&value, "multi_teammate_score_up")?);
    params.multi_teammate_power = i32_field_checked(&value, "multiLiveTeammatePower")?
        .or(i32_field_checked(&value, "multi_teammate_power")?);
    params.multi_live_score_up_lower_bound = value
        .get("multiLiveScoreUpLowerBound")
        .or_else(|| value.get("multi_live_score_up_lower_bound"))
        .and_then(Value::as_f64);
    params.boost = match i32_field_checked(&value, "boost")? {
        Some(boost) if (0..=10).contains(&boost) => Some(boost),
        Some(_) => return Err(serde_json::Error::custom("boost 必须在 0..=10 范围内")),
        None => None,
    };
    params.other_score =
        i32_field_checked(&value, "otherScore")?.or(i32_field_checked(&value, "other_score")?);
    params.life = i32_field_checked(&value, "life")?;
    params.unit_filter =
        string_field(&value, "unitFilter").or_else(|| string_field(&value, "unit_filter"));
    params.attr_filter =
        string_field(&value, "attrFilter").or_else(|| string_field(&value, "attr_filter"));
    params.card_configs = parse_card_config_set(&value);
    params.single_card_configs = parse_single_card_configs(&value);
    crate::handler::validate_build_params(&params)
        .map_err(|error| serde_json::Error::custom(error.to_string()))?;
    Ok(params)
}

/// 解析稀有度默认卡配置集合（满级/满技能/满破/剧情/画布/禁用）。
///
/// 同时接受 camelCase（`rarity4Config.levelMax`）与 snake_case（`rarity_4_config.level_max`）。
fn parse_card_config_set(value: &Value) -> crate::handler::CardConfigSet {
    crate::handler::CardConfigSet {
        rarity_1_config: parse_card_rarity_config(value, "rarity1Config", "rarity_1_config"),
        rarity_2_config: parse_card_rarity_config(value, "rarity2Config", "rarity_2_config"),
        rarity_3_config: parse_card_rarity_config(value, "rarity3Config", "rarity_3_config"),
        rarity_4_config: parse_card_rarity_config(value, "rarity4Config", "rarity_4_config"),
        rarity_birthday_config: parse_card_rarity_config(
            value,
            "rarityBirthdayConfig",
            "rarity_birthday_config",
        ),
        single_card_configs: Vec::new(),
    }
}

/// 解析单卡覆盖配置数组。
fn parse_single_card_configs(value: &Value) -> Vec<crate::handler::SingleCardConfig> {
    let entries = value
        .get("singleCardConfigs")
        .or_else(|| value.get("single_card_configs"));
    let Some(entries) = entries.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let card_id = i32_field(entry, "cardId").or_else(|| i32_field(entry, "card_id"))?;
            let config = entry
                .get("config")
                .map(card_rarity_config_from_value)
                .unwrap_or_else(|| card_rarity_config_from_value(entry));
            Some(crate::handler::SingleCardConfig { card_id, config })
        })
        .collect()
}

fn parse_card_rarity_config(
    value: &Value,
    camel_key: &str,
    snake_key: &str,
) -> crate::handler::CardRarityConfig {
    value
        .get(camel_key)
        .or_else(|| value.get(snake_key))
        .map(card_rarity_config_from_value)
        .unwrap_or_default()
}

fn card_rarity_config_from_value(value: &Value) -> crate::handler::CardRarityConfig {
    let flag = |camel: &str, snake: &str| {
        bool_field(value, camel)
            .or_else(|| bool_field(value, snake))
            .unwrap_or(false)
    };
    crate::handler::CardRarityConfig {
        disable: flag("disable", "disable"),
        level_max: flag("levelMax", "level_max"),
        level: i32_field(value, "level"),
        skill_max: flag("skillMax", "skill_max"),
        skill_level: i32_field(value, "skillLevel").or_else(|| i32_field(value, "skill_level")),
        episode_read: flag("episodeRead", "episode_read"),
        episode_read_count: i32_field(value, "episodeReadCount")
            .or_else(|| i32_field(value, "episode_read_count")),
        master_max: flag("masterMax", "master_max"),
        master_rank: i32_field(value, "masterRank").or_else(|| i32_field(value, "master_rank")),
        canvas: flag("canvas", "canvas"),
    }
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

/// camelCase 优先、snake_case 兜底的整数数组读取。
fn int_array_alias(value: &Value, camel_key: &str, snake_key: &str) -> Vec<i32> {
    let camel = int_array(value, camel_key);
    if camel.is_empty() {
        int_array(value, snake_key)
    } else {
        camel
    }
}

fn parse_custom_bonus_support_units(
    value: &Value,
) -> Result<Vec<crate::types::CustomSupportUnit>, serde_json::Error> {
    let Some(raw) = field_alias_checked(
        value,
        "customBonusCharacterSupportUnits",
        "custom_bonus_character_support_units",
    )?
    else {
        return Ok(Vec::new());
    };
    let entries = raw
        .as_object()
        .ok_or_else(|| serde_json::Error::custom("custom bonus support units 必须是对象"))?;
    let mut result = entries
        .iter()
        .map(|(character_id, unit)| {
            let character_id = character_id
                .parse::<i32>()
                .map_err(|_| serde_json::Error::custom("custom bonus support character id 非法"))?;
            let unit = unit
                .as_str()
                .and_then(crate::handler::types::parse_unit_code)
                .filter(|unit| {
                    matches!(
                        unit,
                        crate::types::Unit::LightSound
                            | crate::types::Unit::Idol
                            | crate::types::Unit::Street
                            | crate::types::Unit::Themepark
                            | crate::types::Unit::SchoolRefusal
                            | crate::types::Unit::Piapro
                    )
                })
                .ok_or_else(|| serde_json::Error::custom("custom bonus support unit 非法"))?;
            Ok(crate::types::CustomSupportUnit { character_id, unit })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    result.sort_unstable_by_key(|entry| entry.character_id);
    Ok(result)
}

fn field_alias_checked<'a>(
    value: &'a Value,
    camel_key: &str,
    snake_key: &str,
) -> Result<Option<&'a Value>, serde_json::Error> {
    if camel_key == snake_key {
        return Ok(value.get(camel_key));
    }
    match (value.get(camel_key), value.get(snake_key)) {
        (Some(camel), Some(snake)) if camel != snake => Err(serde_json::Error::custom(format!(
            "{camel_key} 与 {snake_key} 冲突"
        ))),
        (Some(camel), _) => Ok(Some(camel)),
        (_, Some(snake)) => Ok(Some(snake)),
        (None, None) => Ok(None),
    }
}

fn optional_string_alias_checked(
    value: &Value,
    camel_key: &str,
    snake_key: &str,
) -> Result<Option<String>, serde_json::Error> {
    let Some(raw) = field_alias_checked(value, camel_key, snake_key)? else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    raw.as_str()
        .map(|text| Some(text.to_string()))
        .ok_or_else(|| serde_json::Error::custom(format!("{snake_key} / attr 必须是字符串")))
}

fn bounded_usize_field(
    value: &Value,
    camel_key: &str,
    snake_key: &str,
    min: usize,
    max: usize,
    allow_null: bool,
) -> Result<Option<usize>, serde_json::Error> {
    let Some(raw) = field_alias_checked(value, camel_key, snake_key)? else {
        return Ok(None);
    };
    if allow_null && raw.is_null() {
        return Ok(None);
    }
    let parsed = raw
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
        .filter(|number| (min..=max).contains(number))
        .ok_or_else(|| {
            serde_json::Error::custom(format!("{snake_key} 必须在 {min}..={max} 范围内"))
        })?;
    Ok(Some(parsed))
}

fn bounded_u64_field(
    value: &Value,
    camel_key: &str,
    snake_key: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, serde_json::Error> {
    let Some(raw) = field_alias_checked(value, camel_key, snake_key)? else {
        return Ok(None);
    };
    let parsed = raw
        .as_u64()
        .filter(|number| (min..=max).contains(number))
        .ok_or_else(|| {
            serde_json::Error::custom(format!("{snake_key} 必须在 {min}..={max} 范围内"))
        })?;
    Ok(Some(parsed))
}

fn bounded_int_array_alias(
    value: &Value,
    camel_key: &str,
    snake_key: &str,
    min: i32,
    max: i32,
    max_len: usize,
) -> Result<Vec<i32>, serde_json::Error> {
    let Some(raw) = field_alias_checked(value, camel_key, snake_key)? else {
        return Ok(Vec::new());
    };
    let entries = raw
        .as_array()
        .ok_or_else(|| serde_json::Error::custom(format!("{snake_key} 必须是数组")))?;
    if entries.len() > max_len {
        return Err(serde_json::Error::custom(format!(
            "{snake_key} 最多支持 {max_len} 项"
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    entries
        .iter()
        .map(|entry| {
            let number = entry
                .as_i64()
                .and_then(|number| i32::try_from(number).ok())
                .filter(|number| (min..=max).contains(number))
                .ok_or_else(|| {
                    serde_json::Error::custom(format!(
                        "{snake_key} value 必须在 {min}..={max} 范围内"
                    ))
                })?;
            if !seen.insert(number) {
                return Err(serde_json::Error::custom(format!(
                    "{snake_key} 不得包含重复值"
                )));
            }
            Ok(number)
        })
        .collect()
}

/// 严格整数字段：键存在但类型不对时直接报错，不再静默丢弃
/// （例如字符串形式的 event_id 曾会无声丢掉整个活动上下文）。
fn i32_field_checked(value: &Value, key: &str) -> Result<Option<i32>, serde_json::Error> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| serde_json::Error::custom(format!("{key} 必须是整数"))),
    }
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

fn parse_live_type_checked(value: &str) -> Result<LiveType, serde_json::Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "solo" => Ok(LiveType::Solo),
        "auto" => Ok(LiveType::Auto),
        "multi" => Ok(LiveType::Multi),
        "cheerful" => Ok(LiveType::Cheerful),
        "challenge" => Ok(LiveType::Challenge),
        "challenge_auto" | "challengeauto" => Ok(LiveType::ChallengeAuto),
        "mysekai" => Ok(LiveType::Mysekai),
        _ => Err(serde_json::Error::custom("live_type 非法")),
    }
}

fn parse_specific_skill_order(value: &Value) -> Result<Option<[usize; 5]>, serde_json::Error> {
    let Some(entry) = value
        .get("specificSkillOrder")
        .or_else(|| value.get("specific_skill_order"))
    else {
        return Ok(None);
    };
    let values = match entry {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_u64()
                    .map(|value| value as usize)
                    .ok_or_else(|| serde_json::Error::custom("specific_skill_order 必须是非负整数"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Value::String(text) => text
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<usize>()
                    .map_err(|_| serde_json::Error::custom("specific_skill_order 必须是非负整数"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(serde_json::Error::custom("specific_skill_order 非法")),
    };
    if values.len() != 5 {
        return Err(serde_json::Error::custom(
            "specific_skill_order 必须包含 5 个索引",
        ));
    }
    let mut order = [0usize; 5];
    order.copy_from_slice(&values[..5]);
    if order.iter().any(|value| *value >= 5) {
        return Err(serde_json::Error::custom(
            "specific_skill_order 索引必须在 0..5",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !order.iter().all(|value| seen.insert(*value)) {
        return Err(serde_json::Error::custom(
            "specific_skill_order 索引不得重复",
        ));
    }
    Ok(Some(order))
}

fn parse_target_checked(value: Option<&Value>) -> Result<ScoreTarget, serde_json::Error> {
    let Some(value) = value else {
        return Ok(ScoreTarget::Score);
    };
    let value = value
        .as_str()
        .ok_or_else(|| serde_json::Error::custom("target 必须是字符串"))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "score" => Ok(ScoreTarget::Score),
        "power" => Ok(ScoreTarget::Power),
        "skill" => Ok(ScoreTarget::Skill),
        "bonus" => Ok(ScoreTarget::Bonus),
        "mysekai" => Ok(ScoreTarget::Mysekai),
        _ => Err(serde_json::Error::custom("target 非法")),
    }
}

fn parse_skill_reference_strategy_checked(
    value: &str,
) -> Result<SkillReferenceStrategy, serde_json::Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" => Ok(SkillReferenceStrategy::Max),
        "min" => Ok(SkillReferenceStrategy::Min),
        "average" => Ok(SkillReferenceStrategy::Average),
        _ => Err(serde_json::Error::custom(
            "skill_reference_choose_strategy 非法",
        )),
    }
}

fn parse_live_skill_order_checked(value: &str) -> Result<LiveSkillOrder, serde_json::Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "min" | "worst" => Ok(LiveSkillOrder::Worst),
        "average" => Ok(LiveSkillOrder::Average),
        "specific" => Ok(LiveSkillOrder::Specific),
        "max" | "best" => Ok(LiveSkillOrder::Best),
        _ => Err(serde_json::Error::custom(
            "skill_order_choose_strategy 非法",
        )),
    }
}

fn validate_optional_enum(
    value: Option<&str>,
    field: &str,
    allowed: &[&str],
) -> Result<(), serde_json::Error> {
    if value.is_some_and(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        !allowed.contains(&normalized.as_str())
    }) {
        return Err(serde_json::Error::custom(format!("{field} 非法")));
    }
    Ok(())
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
    pub mysekai_gates: Vec<MysekaiGate>,
    pub mysekai_gate_levels: Vec<MysekaiGateLevel>,
    pub events: Vec<Event>,
    pub event_cards: Vec<EventCard>,
    pub event_deck_bonuses: Vec<EventDeckBonus>,
    pub event_card_bonus_limits: Vec<EventCardBonusLimit>,
    pub event_honor_bonuses: Vec<EventHonorBonus>,
    pub world_bloom_different_attribute_bonuses: Vec<WorldBloomDiffAttrBonus>,
    pub world_blooms: Vec<WorldBloom>,
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
        let sources = MasterdataSources::from_dir(masterdata_dir, music_metas_path)?;
        Self::from_sources(&sources)
    }

    /// 从内存中的原始 JSON 来源组装（wasm/网络/测试用，无文件系统依赖）。
    ///
    /// 这是真正的 raw→flatten 扁平化逻辑；`load` 只是先把磁盘读成 `MasterdataSources`
    /// 再委托到这里。两条路共用同一套组装代码，不分叉。
    pub fn from_sources(sources: &MasterdataSources) -> Result<Self, String> {
        let raw_game_character_units: Vec<RawGameCharacterUnit> =
            sources.required("gameCharacterUnits.json")?;
        let raw_cards: Vec<RawCard> = sources.required("cards.json")?;
        let events: Vec<RawEvent> = sources.required("events.json")?;

        let skill_unit_map = infer_skill_units(&raw_cards, &raw_game_character_units);
        let event_ids = events.iter().map(|event| event.id).collect::<Vec<_>>();

        // 保留所有难度行（easy/normal/hard/expert/master/append），base_score/skill_scores 分难度。
        // 旧逻辑只留 master 行，导致 build_music_params 按非 master 难度选行时匹配不到、回落 master，
        // 使所有难度算出相同分数（难度参数形同虚设）。
        let mut music_rows: Vec<RawMusicMetaRow> = sources.music_rows()?;
        add_omakase_music_rows(&mut music_rows);

        Ok(Self {
            cards: raw_cards
                .iter()
                .map(|card| MasterCard {
                    id: card.id,
                    character_id: card.character_id,
                    attr: card.attr.clone(),
                    card_rarity_type: rarity_type_to_index(&card.card_rarity_type),
                    rarity: card.card_rarity_type.clone(),
                    asset_bundle_name: card.asset_bundle_name.clone().unwrap_or_else(|| {
                        let training = card.special_training_skill_id.is_some();
                        if training {
                            format!("card_{:06}_normal", card.id)
                        } else {
                            format!("chara_{:06}", card.id)
                        }
                    }),
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
            card_rarities: sources
                .required::<Vec<RawCardRarity>>("cardRarities.json")?
                .into_iter()
                .map(|rarity| CardRarity {
                    card_rarity_type: rarity_type_to_index(&rarity.card_rarity_type),
                    max_level: rarity.training_max_level.unwrap_or(rarity.max_level),
                    normal_max_level: rarity.max_level,
                    max_skill_level: rarity.max_skill_level,
                })
                .collect(),
            card_episodes: sources
                .required::<Vec<RawCardEpisode>>("cardEpisodes.json")?
                .into_iter()
                .map(|episode| CardEpisode {
                    card_id: episode.card_id,
                    episode_no: episode.id,
                    power1_bonus_fixed: episode.power1_bonus_fixed,
                    power2_bonus_fixed: episode.power2_bonus_fixed,
                    power3_bonus_fixed: episode.power3_bonus_fixed,
                })
                .collect(),
            master_lessons: sources
                .required::<Vec<RawMasterLesson>>("masterLessons.json")?
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
                &sources.required::<Vec<RawSkill>>("skills.json")?,
                &skill_unit_map,
            )
            .0,
            skill_effects: flatten_skills(
                &sources.required::<Vec<RawSkill>>("skills.json")?,
                &skill_unit_map,
            )
            .1,
            area_item_levels: flatten_area_item_levels(
                sources.required::<Vec<RawAreaItemLevel>>("areaItemLevels.json")?,
            ),
            game_character_units: raw_game_character_units
                .iter()
                .map(|entry| GameCharacterUnit {
                    game_character_id: entry.game_character_id,
                    unit: entry.unit.clone(),
                })
                .collect(),
            character_ranks: sources
                .required::<Vec<RawCharacterRank>>("characterRanks.json")?
                .into_iter()
                .map(|rank| CharacterRank {
                    character_rank: rank.character_rank,
                    power_bonus_rate: rank.power1_bonus_rate,
                })
                .collect(),
            card_mysekai_canvas_bonuses: sources
                .required::<Vec<RawCardMysekaiCanvasBonus>>("cardMysekaiCanvasBonuses.json")?
                .into_iter()
                .map(|entry| CardMysekaiCanvasBonus {
                    card_rarity_type: rarity_type_to_index(&entry.card_rarity_type),
                    power1_bonus_fixed: entry.power1_bonus_fixed,
                    power2_bonus_fixed: entry.power2_bonus_fixed,
                    power3_bonus_fixed: entry.power3_bonus_fixed,
                })
                .collect(),
            mysekai_gates: sources
                .optional::<Vec<RawMysekaiGate>>("mysekaiGates.json")?
                .into_iter()
                .map(|entry| MysekaiGate {
                    id: entry.id,
                    unit: entry.unit,
                })
                .collect(),
            mysekai_gate_levels: sources
                .optional::<Vec<RawMysekaiGateLevel>>("mysekaiGateLevels.json")?
                .into_iter()
                .map(|entry| MysekaiGateLevel {
                    mysekai_gate_id: entry.mysekai_gate_id,
                    level: entry.level,
                    power_bonus_rate: entry.power_bonus_rate,
                })
                .collect(),
            events: events
                .into_iter()
                .map(|event| Event {
                    id: event.id,
                    event_type: event.event_type,
                })
                .collect(),
            event_cards: sources
                .required::<Vec<RawEventCard>>("eventCards.json")?
                .into_iter()
                .map(|entry| EventCard {
                    event_id: entry.event_id,
                    card_id: entry.card_id,
                    bonus_rate_x10: (entry.bonus_rate * 10.0).round() as i32,
                    leader_bonus_rate_x10: (entry.leader_bonus_rate * 10.0).round() as i32,
                })
                .collect(),
            event_deck_bonuses: sources
                .required::<Vec<RawEventDeckBonus>>("eventDeckBonuses.json")?
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
                        bonus_rate_x10: (entry.bonus_rate * 10.0).round() as i32,
                    }
                })
                .collect(),
            // 积分上限/称号加成/技能上限三表缺省时走内建 fallback
            //（bonus 上限 4/5、终章技能上限 140），组卡主链路不受影响。
            event_card_bonus_limits: sources
                .optional::<Vec<RawEventCardBonusLimit>>("eventCardBonusLimits.json")?
                .into_iter()
                .map(|entry| EventCardBonusLimit {
                    event_id: entry.event_id,
                    member_count_limit: entry.member_count_limit,
                })
                .collect(),
            event_honor_bonuses: sources
                .optional::<Vec<RawEventHonorBonus>>("eventHonorBonuses.json")?
                .into_iter()
                .map(|entry| EventHonorBonus {
                    event_id: entry.event_id,
                    honor_id: entry.honor_id,
                    leader_game_character_id: entry.leader_game_character_id,
                    bonus_rate: entry.bonus_rate,
                })
                .collect(),
            world_bloom_different_attribute_bonuses: sources
                .required::<Vec<RawWorldBloomDiffAttrBonus>>(
                    "worldBloomDifferentAttributeBonuses.json",
                )?
                .into_iter()
                .map(|entry| WorldBloomDiffAttrBonus {
                    attr_count: entry.attr_count,
                    bonus_rate: entry.bonus_rate.round() as i32,
                })
                .collect(),
            world_blooms: sources
                .optional::<Vec<RawWorldBloom>>("worldBlooms.json")?
                .into_iter()
                .map(|entry| WorldBloom {
                    event_id: entry.event_id,
                    game_character_id: entry.game_character_id,
                    chapter_no: entry.chapter_no,
                    world_bloom_chapter_type: entry.world_bloom_chapter_type,
                })
                .collect(),
            wb_support_deck_bonuses_wl1: load_wl_support_bonuses(
                sources,
                "worldBloomSupportDeckBonusesWL1.json",
                EMBEDDED_WL1_SUPPORT_BONUSES,
            )?,
            wb_support_deck_bonuses_wl2: load_wl_support_bonuses(
                sources,
                "worldBloomSupportDeckBonusesWL2.json",
                EMBEDDED_WL2_SUPPORT_BONUSES,
            )?,
            wb_support_deck_bonuses_wl3: load_wl3_support_bonuses(sources)?,
            world_bloom_support_deck_unit_event_limited_bonuses: sources.optional::<Vec<
                WBSupportDeckUnitEventLimitedBonus,
            >>(
                "worldBloomSupportDeckUnitEventLimitedBonuses.json",
            )?,
            event_mysekai_fixture_performance_bonus_limits: sources
                .optional::<Vec<RawEventFixtureBonusLimit>>(
                    "eventMysekaiFixtureGameCharacterPerformanceBonusLimits.json",
                )?
                .into_iter()
                .map(|entry| EventFixtureBonusLimit {
                    event_id: entry.event_id,
                    bonus_rate_limit: entry.bonus_rate_limit,
                })
                .collect(),
            event_skill_score_up_limits: sources
                .optional::<Vec<RawEventSkillScoreUpLimit>>("eventSkillScoreUpLimits.json")?
                .into_iter()
                .map(|entry| EventSkillScoreUpLimit {
                    event_id: entry.event_id,
                    score_up_limit: entry.score_up_rate_limit,
                })
                .collect(),
            music_metas: music_rows
                .iter()
                .map(|row| MusicMeta {
                    music_id: row.music_id,
                    difficulty: row.difficulty.clone(),
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
            music_difficulties: music_rows
                .iter()
                .map(|row| MusicDifficulty {
                    music_id: row.music_id,
                    difficulty: row.difficulty.clone(),
                    event_rate: Some(row.event_rate),
                })
                .collect(),
            event_rarity_bonus_rates: sources
                .required::<Vec<RawEventRarityBonusRate>>("eventRarityBonusRates.json")?
                .into_iter()
                .flat_map(|entry| {
                    event_ids
                        .iter()
                        .copied()
                        .map(move |event_id| EventRarityBonusRate {
                            event_id,
                            card_rarity_type: rarity_type_to_index(&entry.card_rarity_type),
                            master_rank: entry.master_rank,
                            bonus_rate_x10: rate_to_x10_i32(entry.bonus_rate),
                        })
                })
                .collect(),
            honors: sources
                .optional::<Vec<RawHonor>>("honors.json")?
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
                    asset_bundle_name: entry.asset_bundle_name,
                })
                .collect(),
            bonds_honors: sources
                .optional::<Vec<RawIdOnly>>("bondsHonors.json")?
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
            mysekai_gates: &self.mysekai_gates,
            mysekai_gate_levels: &self.mysekai_gate_levels,
            events: &self.events,
            event_cards: &self.event_cards,
            event_deck_bonuses: &self.event_deck_bonuses,
            event_card_bonus_limits: &self.event_card_bonus_limits,
            event_honor_bonuses: &self.event_honor_bonuses,
            world_bloom_different_attribute_bonuses: &self.world_bloom_different_attribute_bonuses,
            world_blooms: &self.world_blooms,
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
    match &card.card_parameters {
        RawCardParameters::Grouped(grouped) => {
            let len = grouped
                .param1
                .len()
                .min(grouped.param2.len())
                .min(grouped.param3.len());
            (0..len)
                .map(|index| CardParameter {
                    card_id: card.id,
                    level: index as i32 + 1,
                    param1: grouped.param1[index],
                    param2: grouped.param2[index],
                    param3: grouped.param3[index],
                })
                .collect()
        }
        RawCardParameters::Rows(rows) => {
            let mut by_level: BTreeMap<i32, [Option<i32>; 3]> = BTreeMap::new();
            for row in rows {
                let slot = match row.card_parameter_type.as_str() {
                    "param1" => 0,
                    "param2" => 1,
                    "param3" => 2,
                    _ => continue,
                };
                by_level.entry(row.card_level).or_default()[slot] = Some(row.power);
            }
            by_level
                .into_iter()
                .filter_map(|(level, [param1, param2, param3])| {
                    Some(CardParameter {
                        card_id: card.id,
                        level,
                        param1: param1?,
                        param2: param2?,
                        param3: param3?,
                    })
                })
                .collect()
        }
    }
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

/// masterdata 的原始 JSON 来源（文件名 → 内容字符串）。
///
/// 把「从哪读字符串」与「raw→flatten 组装」解耦：`load` 从磁盘填充它，
/// wasm 端从内嵌/网络字符串填充它，两条路共用同一套 `from_sources` 扁平化逻辑。
/// `music_metas.json` 在游戏侧是独立命名空间，单列一个字段。
#[derive(Debug, Default, Clone)]
pub struct MasterdataSources {
    tables: BTreeMap<String, String>,
    music_metas: String,
}

impl MasterdataSources {
    /// 从内存 map 构造（wasm/测试用）。键为文件名（含 `.json`），值为该表 JSON 文本。
    pub fn from_strings(
        tables: impl IntoIterator<Item = (String, String)>,
        music_metas: String,
    ) -> Self {
        Self {
            tables: tables.into_iter().collect(),
            music_metas,
        }
    }

    /// 从磁盘目录读取所有 `*.json` + 独立的 music_metas 文件。
    pub fn from_dir(masterdata_dir: &Path, music_metas_path: &Path) -> Result<Self, String> {
        let mut tables = BTreeMap::new();
        let entries = fs::read_dir(masterdata_dir)
            .map_err(|err| format!("读取目录 {} 失败: {err}", masterdata_dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("遍历 masterdata 目录失败: {err}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let text = fs::read_to_string(&path)
                .map_err(|err| format!("读取 {} 失败: {err}", path.display()))?;
            tables.insert(name.to_string(), text);
        }
        let music_metas = fs::read_to_string(music_metas_path)
            .map_err(|err| format!("读取 {} 失败: {err}", music_metas_path.display()))?;
        Ok(Self {
            tables,
            music_metas,
        })
    }

    /// 必需表：缺失即报错。
    fn required<T: DeserializeOwned>(&self, file_name: &str) -> Result<T, String> {
        let text = self
            .tables
            .get(file_name)
            .ok_or_else(|| format!("缺少 masterdata 表 {file_name}"))?;
        serde_json::from_str(text).map_err(|err| format!("解析 {file_name} 失败: {err}"))
    }

    /// 可选表：缺失则取默认值（空）。
    fn optional<T: DeserializeOwned + Default>(&self, file_name: &str) -> Result<T, String> {
        match self.tables.get(file_name) {
            None => Ok(T::default()),
            Some(text) => {
                serde_json::from_str(text).map_err(|err| format!("解析 {file_name} 失败: {err}"))
            }
        }
    }

    /// music_metas 行。
    fn music_rows(&self) -> Result<Vec<RawMusicMetaRow>, String> {
        serde_json::from_str(&self.music_metas)
            .map_err(|err| format!("解析 music_metas 失败: {err}"))
    }
}

/// WL 支援加成表是仓库静态数据（moe 把它们放在 `data/`，不随游戏 masterdata 更新）。
/// 这里随 crate 内嵌一份，masterdata 目录缺文件时兜底，保证 WL 支援加成不为 0。
const EMBEDDED_WL1_SUPPORT_BONUSES: &str =
    include_str!("../data/worldBloomSupportDeckBonusesWL1.json");
const EMBEDDED_WL2_SUPPORT_BONUSES: &str =
    include_str!("../data/worldBloomSupportDeckBonusesWL2.json");
const EMBEDDED_WL3_SUPPORT_BONUSES: &str =
    include_str!("../data/worldBloomSupportDeckBonusesWL3.json");

/// 加载某一轮 WL 支援加成表：优先用 masterdata 来源里的文件，缺失则用内嵌静态副本。
fn load_wl_support_bonuses(
    sources: &MasterdataSources,
    file_name: &str,
    embedded: &str,
) -> Result<Vec<WBSupportDeckBonus>, String> {
    let from_disk = sources.optional::<Vec<WBSupportDeckBonus>>(file_name)?;
    if !from_disk.is_empty() {
        return Ok(from_disk);
    }
    serde_json::from_str(embedded).map_err(|err| format!("解析内嵌 {file_name} 失败: {err}"))
}

fn load_wl3_support_bonuses(
    sources: &MasterdataSources,
) -> Result<Vec<WBSupportDeckBonus>, String> {
    let exact =
        sources.optional::<Vec<WBSupportDeckBonus>>("worldBloomSupportDeckBonusesWL3.json")?;
    if !exact.is_empty() {
        return Ok(exact);
    }
    let legacy =
        sources.optional::<Vec<WBSupportDeckBonus>>("worldBloomSupportDeckBonuses.json")?;
    if !legacy.is_empty() {
        return Ok(legacy);
    }
    serde_json::from_str(EMBEDDED_WL3_SUPPORT_BONUSES)
        .map_err(|err| format!("解析内嵌 WL3 支援表失败: {err}"))
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
    #[serde(rename = "assetbundleName", default)]
    asset_bundle_name: Option<String>,
    #[serde(default)]
    special_training_power1_bonus_fixed: i32,
    #[serde(default)]
    special_training_power2_bonus_fixed: i32,
    #[serde(default)]
    special_training_power3_bonus_fixed: i32,
    card_parameters: RawCardParameters,
}

/// `cardParameters` 有两种编码形式，两者都要能读：
/// 按参数名分组的等级数组，或每行一个 `(cardLevel, cardParameterType, power)`
/// 的行式表。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawCardParameters {
    Grouped(RawGroupedCardParameters),
    Rows(Vec<RawCardParameterRow>),
}

impl Default for RawCardParameters {
    fn default() -> Self {
        Self::Grouped(RawGroupedCardParameters::default())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawGroupedCardParameters {
    #[serde(default)]
    param1: Vec<i32>,
    #[serde(default)]
    param2: Vec<i32>,
    #[serde(default)]
    param3: Vec<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCardParameterRow {
    card_level: i32,
    card_parameter_type: String,
    power: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCardRarity {
    card_rarity_type: String,
    max_level: i32,
    #[serde(default)]
    training_max_level: Option<i32>,
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
struct RawMysekaiGate {
    id: i32,
    unit: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMysekaiGateLevel {
    mysekai_gate_id: i32,
    level: i32,
    power_bonus_rate: f64,
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
struct RawWorldBloom {
    event_id: i32,
    #[serde(default)]
    game_character_id: Option<i32>,
    chapter_no: i32,
    #[serde(default)]
    world_bloom_chapter_type: Option<String>,
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

const OMAKASE_MUSIC_ID: i32 = 10000;
const OMAKASE_SOURCE_DIFFS: &[&str] = &["master", "expert", "hard"];
const OMAKASE_OUTPUT_DIFFS: &[&str] = &["easy", "normal", "hard", "expert", "master", "append"];

fn add_omakase_music_rows(rows: &mut Vec<RawMusicMetaRow>) {
    let existing_count = rows
        .iter()
        .filter(|row| {
            row.music_id == OMAKASE_MUSIC_ID
                && OMAKASE_OUTPUT_DIFFS
                    .iter()
                    .any(|diff| row.difficulty.eq_ignore_ascii_case(diff))
        })
        .count();
    if existing_count >= OMAKASE_OUTPUT_DIFFS.len() {
        return;
    }

    rows.retain(|row| row.music_id != OMAKASE_MUSIC_ID);

    let mut count = 0usize;
    let mut music_time = 0.0;
    let mut event_rate = 0i64;
    let mut base_score = 0.0;
    let mut base_score_auto = 0.0;
    let mut skill_score_solo = [0.0; 6];
    let mut skill_score_auto = [0.0; 6];
    let mut skill_score_multi = [0.0; 6];
    let mut fever_score = 0.0;
    let mut tap_count = 0i64;

    for row in rows.iter().filter(|row| {
        OMAKASE_SOURCE_DIFFS
            .iter()
            .any(|diff| row.difficulty.eq_ignore_ascii_case(diff))
    }) {
        count += 1;
        music_time += row.music_time;
        event_rate += i64::from(row.event_rate);
        base_score += row.base_score;
        base_score_auto += row.base_score_auto;
        for idx in 0..6 {
            skill_score_solo[idx] += row.skill_score_solo[idx];
            skill_score_auto[idx] += row.skill_score_auto[idx];
            skill_score_multi[idx] += row.skill_score_multi[idx];
        }
        fever_score += row.fever_score;
        tap_count += i64::from(row.tap_count);
    }

    if count == 0 {
        return;
    }

    let denom = count as f64;
    for idx in 0..6 {
        skill_score_solo[idx] /= denom;
        skill_score_auto[idx] /= denom;
        skill_score_multi[idx] /= denom;
    }
    let average = RawMusicMetaRow {
        music_id: OMAKASE_MUSIC_ID,
        difficulty: String::new(),
        music_time: music_time / denom,
        event_rate: (event_rate / count as i64) as i32,
        base_score: base_score / denom,
        base_score_auto: base_score_auto / denom,
        skill_score_solo,
        skill_score_auto,
        skill_score_multi,
        fever_score: fever_score / denom,
        tap_count: (tap_count / count as i64) as i32,
    };

    rows.extend(OMAKASE_OUTPUT_DIFFS.iter().map(|diff| {
        let mut row = average.clone();
        row.difficulty = (*diff).to_string();
        row
    }));
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventRarityBonusRate {
    card_rarity_type: String,
    master_rank: i32,
    bonus_rate: f64,
}

fn rate_to_x10_i32(value: f64) -> i32 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        (value * 10.0).round().clamp(0.0, i32::MAX as f64) as i32
    }
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
    #[serde(default)]
    asset_bundle_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHonorLevel {
    level: i32,
    #[serde(default)]
    bonus: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music_row(
        music_id: i32,
        difficulty: &str,
        base_score: f64,
        event_rate: i32,
    ) -> RawMusicMetaRow {
        RawMusicMetaRow {
            music_id,
            difficulty: difficulty.to_string(),
            music_time: base_score * 10.0,
            event_rate,
            base_score,
            base_score_auto: base_score + 1.0,
            skill_score_solo: [base_score; 6],
            skill_score_auto: [base_score + 2.0; 6],
            skill_score_multi: [base_score + 3.0; 6],
            fever_score: base_score + 4.0,
            tap_count: event_rate,
        }
    }

    #[test]
    fn add_omakase_music_rows_averages_master_expert_hard() {
        let mut rows = vec![
            music_row(1, "easy", 10.0, 100),
            music_row(1, "hard", 20.0, 101),
            music_row(2, "expert", 40.0, 102),
            music_row(3, "master", 60.0, 103),
            music_row(4, "append", 100.0, 999),
        ];

        add_omakase_music_rows(&mut rows);

        let omakase: Vec<_> = rows
            .iter()
            .filter(|row| row.music_id == OMAKASE_MUSIC_ID)
            .collect();
        assert_eq!(omakase.len(), OMAKASE_OUTPUT_DIFFS.len());
        assert!(
            OMAKASE_OUTPUT_DIFFS
                .iter()
                .all(|diff| omakase.iter().any(|row| row.difficulty == *diff))
        );
        let master = omakase
            .iter()
            .find(|row| row.difficulty == "master")
            .expect("omakase master row");
        assert!((master.base_score - 40.0).abs() < 1e-9);
        assert_eq!(master.event_rate, 102);
        assert_eq!(master.tap_count, 102);
        assert!((master.skill_score_multi[0] - 43.0).abs() < 1e-9);
    }

    #[test]
    fn parse_build_params_reads_camel_case_card_configs() {
        // P2 回归：之前 parse_build_params_json 完全不读 card_configs，
        // 满级/满技能/满破/剧情/画布开关被静默丢弃。
        let json = r#"{
            "region":"cn","liveType":"solo","target":"power",
            "rarity4Config":{"levelMax":true,"skillMax":true,"masterMax":true,
                             "episodeRead":true,"canvas":true,"level":51,
                             "skillLevel":2,"masterRank":3,"episodeReadCount":1},
            "rarity3Config":{"disable":true}
        }"#;
        let params = parse_build_params_json(json).expect("parse");
        let r4 = &params.card_configs.rarity_4_config;
        assert!(r4.level_max && r4.skill_max && r4.master_max && r4.episode_read && r4.canvas);
        assert_eq!(r4.level, Some(51));
        assert_eq!(r4.skill_level, Some(2));
        assert_eq!(r4.master_rank, Some(3));
        assert_eq!(r4.episode_read_count, Some(1));
        assert!(params.card_configs.rarity_3_config.disable);
        // 未提供的稀有度保持默认 false。
        assert!(!params.card_configs.rarity_1_config.level_max);
    }

    #[test]
    fn parse_build_params_accepts_snake_case_card_configs() {
        let json = r#"{"rarity_4_config":{"level_max":true,"master_max":true,
                     "level":52,"skill_level":3,"master_rank":2,
                     "episode_read_count":0}}"#;
        let params = parse_build_params_json(json).expect("parse");
        assert!(params.card_configs.rarity_4_config.level_max);
        assert!(params.card_configs.rarity_4_config.master_max);
        assert!(!params.card_configs.rarity_4_config.skill_max);
        assert_eq!(params.card_configs.rarity_4_config.level, Some(52));
        assert_eq!(params.card_configs.rarity_4_config.skill_level, Some(3));
        assert_eq!(params.card_configs.rarity_4_config.master_rank, Some(2));
        assert_eq!(
            params.card_configs.rarity_4_config.episode_read_count,
            Some(0)
        );
    }

    #[test]
    fn parse_build_params_reads_single_card_configs() {
        // 支持 {cardId, config:{...}} 与扁平 {cardId, levelMax:...} 两种形态。
        let json = r#"{"singleCardConfigs":[
            {"cardId":123,"config":{"levelMax":true}},
            {"cardId":456,"skillMax":true}
        ]}"#;
        let params = parse_build_params_json(json).expect("parse");
        assert_eq!(params.single_card_configs.len(), 2);
        assert_eq!(params.single_card_configs[0].card_id, 123);
        assert!(params.single_card_configs[0].config.level_max);
        assert_eq!(params.single_card_configs[1].card_id, 456);
        assert!(params.single_card_configs[1].config.skill_max);
    }

    #[test]
    fn parse_build_params_defaults_card_configs_empty_when_absent() {
        let params = parse_build_params_json(r#"{"region":"cn"}"#).expect("parse");
        assert!(!params.card_configs.rarity_4_config.level_max);
        assert!(params.single_card_configs.is_empty());
    }

    #[test]
    fn parse_build_params_reads_world_bloom_support_max_flags() {
        let camel = parse_build_params_json(r#"{"supportMasterMax":true,"supportSkillMax":true}"#)
            .expect("parse camel case");
        assert!(camel.support_master_max);
        assert!(camel.support_skill_max);

        let snake =
            parse_build_params_json(r#"{"support_master_max":true,"support_skill_max":true}"#)
                .expect("parse snake case");
        assert!(snake.support_master_max);
        assert!(snake.support_skill_max);
    }

    #[test]
    fn parse_build_params_rejects_out_of_range_boost() {
        // 回归：boost=11 曾被静默接受并按无 boost 计 PT（docs 承诺 0-10）。
        let err = parse_build_params_json(r#"{"boost":11}"#).expect_err("boost 11 应被拒绝");
        assert!(err.to_string().contains("boost"), "{err}");

        let params = parse_build_params_json(r#"{"boost":10}"#).expect("boost 10 合法");
        assert_eq!(params.boost, Some(10));
        let params = parse_build_params_json(r#"{"boost":0}"#).expect("boost 0 合法");
        assert_eq!(params.boost, Some(0));
    }

    #[test]
    fn parse_build_params_rejects_wrong_typed_integer_fields() {
        // 回归：字符串形式的 event_id 曾被静默丢弃，整个活动上下文无声消失。
        let err =
            parse_build_params_json(r#"{"event_id":"215"}"#).expect_err("字符串 event_id 应被拒绝");
        assert!(err.to_string().contains("event_id"), "{err}");

        let err =
            parse_build_params_json(r#"{"musicId":"74"}"#).expect_err("字符串 musicId 应被拒绝");
        assert!(err.to_string().contains("musicId"), "{err}");

        // 正确类型不受影响。
        let params = parse_build_params_json(r#"{"event_id":215}"#).expect("数字 event_id 合法");
        assert_eq!(params.event_id, Some(215));
    }

    #[test]
    fn parse_build_params_rejects_non_object_payload() {
        let err = parse_build_params_json("[1,2,3]").expect_err("顶层数组应被拒绝");
        assert!(err.to_string().contains("JSON 对象"), "{err}");
        let err = parse_build_params_json(r#""score""#).expect_err("顶层字符串应被拒绝");
        assert!(err.to_string().contains("JSON 对象"), "{err}");
    }

    #[test]
    fn parse_specific_skill_order_rejects_duplicates_and_garbage() {
        let err = parse_build_params_json(
            r#"{"liveSkillOrder":"specific","specificSkillOrder":[1,2,3,4,4]}"#,
        )
        .expect_err("重复索引应被拒绝");
        assert!(err.to_string().contains("重复"), "{err}");

        let err = parse_build_params_json(
            r#"{"liveSkillOrder":"specific","specificSkillOrder":"1,2,3,4,x"}"#,
        )
        .expect_err("非数字索引应被拒绝");
        assert!(err.to_string().contains("非负整数"), "{err}");

        let params = parse_build_params_json(
            r#"{"liveSkillOrder":"specific","specificSkillOrder":[4,3,2,1,0]}"#,
        )
        .expect("合法排列不受影响");
        assert_eq!(params.specific_skill_order, Some([4, 3, 2, 1, 0]));
    }

    #[test]
    fn parse_event_unit_accepts_the_shared_unit_vocabulary() {
        // 回归：event_unit 曾用独立硬编码词表，拒绝 unit_filter 已接受的别名。
        for unit in ["light_sound", "ln", "leoneed", "mmj", "vbs", "piapro"] {
            let parsed =
                parse_build_params_json(&format!(r#"{{"event_unit":"{unit}"}}"#)).expect(unit);
            assert_eq!(parsed.event_unit.as_deref(), Some(unit));
        }
        let err = parse_build_params_json(r#"{"event_unit":"bogus"}"#).expect_err("非法团应被拒绝");
        assert!(err.to_string().contains("event_unit"), "{err}");
    }

    #[test]
    fn parse_build_params_reads_specific_skill_order() {
        let params = parse_build_params_json(
            r#"{"liveSkillOrder":"specific","specificSkillOrder":[4,3,2,1,0]}"#,
        )
        .expect("parse");
        assert_eq!(params.live_skill_order, LiveSkillOrder::Specific);
        assert_eq!(params.specific_skill_order, Some([4, 3, 2, 1, 0]));

        let params = parse_build_params_json(
            r#"{"skill_order_choose_strategy":"specific","specific_skill_order":"0,1,2,3,4"}"#,
        )
        .expect("parse");
        assert_eq!(params.live_skill_order, LiveSkillOrder::Specific);
        assert_eq!(params.specific_skill_order, Some([0, 1, 2, 3, 4]));
    }

    #[test]
    fn parse_build_params_reads_bonus_and_custom_event_fields() {
        let params = parse_build_params_json(
            r#"{
                "target":"bonus",
                "limit":7,
                "member":5,
                "timeoutMs":1234,
                "targetBonusList":[225,250],
                "customBonusCharacterIds":[1,5,21],
                "customBonusAttr":"cute",
                "customBonusCharacterSupportUnits":{"21":"street"}
            }"#,
        )
        .expect("parse");

        assert_eq!(params.target, ScoreTarget::Bonus);
        assert_eq!(params.limit, 7);
        assert_eq!(params.member, Some(5));
        assert_eq!(params.timeout_ms, 1234);
        assert_eq!(params.target_bonus_list, vec![225, 250]);
        assert_eq!(params.custom_bonus_character_ids, vec![1, 5, 21]);
        assert_eq!(params.custom_bonus_attr.as_deref(), Some("cute"));
        assert_eq!(params.custom_bonus_character_support_units.len(), 1);
        assert_eq!(
            params.custom_bonus_character_support_units[0].character_id,
            21
        );
        assert_eq!(
            params.custom_bonus_character_support_units[0].unit,
            crate::Unit::Street
        );
    }

    #[test]
    fn parse_build_params_reads_snake_case_bonus_and_custom_event_fields() {
        let params = parse_build_params_json(
            r#"{
                "target":"bonus",
                "target_bonus_list":[200],
                "custom_bonus_character_ids":[2,6,21],
                "custom_bonus_attr":"pure",
                "custom_bonus_character_support_units":{"21":"idol"}
            }"#,
        )
        .expect("parse");

        assert_eq!(params.target_bonus_list, vec![200]);
        assert_eq!(params.custom_bonus_character_ids, vec![2, 6, 21]);
        assert_eq!(params.custom_bonus_attr.as_deref(), Some("pure"));
        assert_eq!(
            params.custom_bonus_character_support_units[0].unit,
            crate::Unit::Idol
        );
    }

    #[test]
    fn parse_build_params_reads_deck_constraint_fields_in_both_cases() {
        let camel = parse_build_params_json(
            r#"{
                "fixedCards":[101,102],
                "fixedCharacters":[3],
                "excludedCards":[999],
                "forcedLeaderCharacterId":5
            }"#,
        )
        .expect("parse");
        assert_eq!(camel.fixed_cards, vec![101, 102]);
        assert_eq!(camel.fixed_characters, vec![3]);
        assert_eq!(camel.excluded_cards, vec![999]);
        assert_eq!(camel.forced_leader_character_id, Some(5));

        let snake = parse_build_params_json(
            r#"{
                "fixed_cards":[201],
                "fixed_characters":[7,8],
                "excluded_cards":[888],
                "forced_leader_character_id":9
            }"#,
        )
        .expect("parse");
        assert_eq!(snake.fixed_cards, vec![201]);
        assert_eq!(snake.fixed_characters, vec![7, 8]);
        assert_eq!(snake.excluded_cards, vec![888]);
        assert_eq!(snake.forced_leader_character_id, Some(9));
    }

    #[test]
    fn parse_build_params_reads_the_full_simulated_event_option_set() {
        // 模拟活动不带 event_id，全部条件由这组键描述；浏览器 worker 用 snake_case 下发。
        for json in [
            r#"{
                "event_type":"marathon",
                "event_unit":"light_sound",
                "event_attr":"cool",
                "custom_bonus_character_ids":[1,5,21],
                "custom_bonus_character_support_units":{"21":"street"},
                "world_bloom_event_turn":3,
                "world_bloom_character_id":21
            }"#,
            r#"{
                "eventType":"marathon",
                "eventUnit":"light_sound",
                "eventAttr":"cool",
                "customBonusCharacterIds":[1,5,21],
                "customBonusCharacterSupportUnits":{"21":"street"},
                "worldBloomEventTurn":3,
                "worldBloomCharacterId":21
            }"#,
        ] {
            let params = parse_build_params_json(json).expect("parse");
            assert_eq!(params.event_id, None);
            assert_eq!(params.event_type.as_deref(), Some("marathon"));
            assert_eq!(params.event_unit.as_deref(), Some("light_sound"));
            assert_eq!(params.event_attr.as_deref(), Some("cool"));
            assert_eq!(params.custom_bonus_character_ids, vec![1, 5, 21]);
            assert_eq!(params.custom_bonus_character_support_units.len(), 1);
            assert_eq!(params.world_bloom_event_turn, Some(3));
            assert_eq!(params.world_bloom_character_id, Some(21));
        }
    }

    #[test]
    fn parse_build_params_rejects_invalid_bounded_compat_fields() {
        for (json, expected) in [
            (r#"{"limit":0}"#, "limit"),
            (r#"{"limit":101}"#, "limit"),
            (r#"{"member":4}"#, "member"),
            (r#"{"timeoutMs":-1}"#, "timeout"),
            (r#"{"timeoutMs":0}"#, "timeout"),
            (r#"{"limit":null}"#, "limit"),
            (r#"{"timeoutMs":null}"#, "timeout"),
            (r#"{"timeoutMs":1000,"timeout_ms":"bad"}"#, "冲突"),
            (r#"{"target":"unknown"}"#, "target"),
            (r#"{"liveType":"unknown"}"#, "live_type"),
            (r#"{"eventType":"unknown"}"#, "event_type"),
            (r#"{"eventAttr":"unknown"}"#, "event_attr"),
            (r#"{"eventUnit":"unknown"}"#, "event_unit"),
            (
                r#"{"skillReferenceChooseStrategy":"unknown"}"#,
                "skill_reference",
            ),
            (r#"{"skillOrderChooseStrategy":"unknown"}"#, "skill_order"),
            (
                r#"{"skillOrderChooseStrategy":"specific"}"#,
                "specific_skill_order",
            ),
            (
                r#"{"skillOrderChooseStrategy":"specific","specificSkillOrder":[0,1]}"#,
                "5 个索引",
            ),
            (r#"{"targetBonusList":[100]}"#, "bonus target"),
            (
                r#"{"liveType":"solo","multiLiveScoreUpLowerBound":100}"#,
                "multi live",
            ),
            (r#"{"targetBonusList":[100,100]}"#, "重复"),
            (r#"{"targetBonusList":[-1]}"#, "bonus"),
            (r#"{"customBonusCharacterIds":[0]}"#, "character"),
            (r#"{"customBonusAttr":"unknown"}"#, "attr"),
            (r#"{"customBonusAttr":1}"#, "attr"),
            (
                r#"{"customBonusCharacterSupportUnits":{"21":"unknown"}}"#,
                "support",
            ),
            (
                r#"{"customBonusCharacterIds":[22],"customBonusCharacterSupportUnits":{"21":"idol"}}"#,
                "custom bonus character",
            ),
        ] {
            let error = parse_build_params_json(json).expect_err("invalid params must fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} in {error:?} for {json}",
            );
        }
    }

    #[test]
    fn parse_build_params_preserves_null_for_optional_compat_fields() {
        let params = parse_build_params_json(r#"{"member":null,"customBonusAttr":null}"#)
            .expect("optional null fields are None");
        assert_eq!(params.member, None);
        assert_eq!(params.custom_bonus_attr, None);
    }

    #[test]
    fn embedded_wl_support_bonus_tables_are_present_and_parse() {
        // P1 回归：WL 支援加成表过去从未被加载（masterdata 目录无此文件 → 静默空 → 支援 bonus 恒 0）。
        // 现随 crate 内嵌，必须能解析且非空，否则 WL 组卡的 support_deck_bonus 会再次塌成 0。
        for (name, embedded) in [
            ("WL1", EMBEDDED_WL1_SUPPORT_BONUSES),
            ("WL2", EMBEDDED_WL2_SUPPORT_BONUSES),
            ("WL3", EMBEDDED_WL3_SUPPORT_BONUSES),
        ] {
            let parsed: Vec<crate::handler::WBSupportDeckBonus> = serde_json::from_str(embedded)
                .unwrap_or_else(|e| panic!("内嵌 {name} 解析失败: {e}"));
            assert!(!parsed.is_empty(), "内嵌 {name} 支援表为空");
            // 至少有一档稀有度带非零角色加成，确认字段映射正确（camelCase）。
            let has_nonzero = parsed.iter().any(|row| {
                row.world_bloom_support_deck_character_bonuses
                    .iter()
                    .any(|b| b.bonus_rate > 0.0)
            });
            assert!(
                has_nonzero,
                "内嵌 {name} 无任何非零角色加成，字段映射可能错误"
            );
        }
    }

    #[test]
    fn load_wl_support_bonuses_falls_back_to_embedded_when_file_absent() {
        // masterdata 来源缺该表时，必须回退到内嵌副本而非返回空。
        let empty = MasterdataSources::default();
        let wl1 = load_wl_support_bonuses(
            &empty,
            "definitely_nonexistent_wl1_xyz.json",
            EMBEDDED_WL1_SUPPORT_BONUSES,
        )
        .expect("load");
        assert!(!wl1.is_empty(), "缺文件时应回退到内嵌 WL1 表");
    }

    #[test]
    fn from_sources_matches_load_from_dir() {
        // 拆分护栏：from_sources（内存）与 load（磁盘）必须产出逐字节一致的 OwnedGameData。
        // 需要真实 masterdata；缺数据时跳过（CI 通过 env 提供）。
        let Some(dir) = std::env::var_os("ALLIUM_MASTERDATA_CN") else {
            eprintln!("跳过：未设 ALLIUM_MASTERDATA_CN");
            return;
        };
        let Some(mm) = std::env::var_os("ALLIUM_MUSIC_METAS") else {
            eprintln!("跳过：未设 ALLIUM_MUSIC_METAS");
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mm = std::path::PathBuf::from(mm);

        let via_load = OwnedGameData::load(&dir, &mm).expect("load");
        let sources = MasterdataSources::from_dir(&dir, &mm).expect("from_dir");
        let via_sources = OwnedGameData::from_sources(&sources).expect("from_sources");

        assert_eq!(
            serde_json::to_vec(&via_load).unwrap(),
            serde_json::to_vec(&via_sources).unwrap(),
            "from_sources 与 load 产出不一致"
        );
    }
    #[test]
    fn from_sources_tolerates_missing_limit_tables() {
        // 积分上限三表（eventCardBonusLimits/eventHonorBonuses/eventSkillScoreUpLimits）
        // 是可选供给：缺失时走内建 fallback，不得阻断 from_sources。
        let Some(dir) = std::env::var_os("ALLIUM_MASTERDATA_CN") else {
            eprintln!("跳过：未设 ALLIUM_MASTERDATA_CN");
            return;
        };
        let Some(mm) = std::env::var_os("ALLIUM_MUSIC_METAS") else {
            eprintln!("跳过：未设 ALLIUM_MUSIC_METAS");
            return;
        };
        let mut sources = MasterdataSources::from_dir(
            &std::path::PathBuf::from(dir),
            &std::path::PathBuf::from(mm),
        )
        .expect("from_dir");
        for name in [
            "eventCardBonusLimits.json",
            "eventHonorBonuses.json",
            "eventSkillScoreUpLimits.json",
        ] {
            sources.tables.remove(name);
        }

        let owned = OwnedGameData::from_sources(&sources).expect("from_sources");
        assert!(owned.event_card_bonus_limits.is_empty());
        assert!(owned.event_honor_bonuses.is_empty());
        assert!(owned.event_skill_score_up_limits.is_empty());
    }

    #[test]
    fn card_parameters_accept_both_grouped_and_row_encodings() {
        let grouped: RawCard = serde_json::from_str(
            r#"{
                "id": 7,
                "characterId": 1,
                "cardRarityType": "rarity_4",
                "attr": "cool",
                "skillId": 3,
                "cardParameters": {
                    "param1": [10, 11],
                    "param2": [20, 21],
                    "param3": [30, 31]
                }
            }"#,
        )
        .expect("grouped card");
        let rows: RawCard = serde_json::from_str(
            r#"{
                "id": 7,
                "characterId": 1,
                "cardRarityType": "rarity_4",
                "attr": "cool",
                "skillId": 3,
                "cardParameters": [
                    {"cardLevel": 2, "cardParameterType": "param3", "power": 31},
                    {"cardLevel": 1, "cardParameterType": "param1", "power": 10},
                    {"cardLevel": 1, "cardParameterType": "param2", "power": 20},
                    {"cardLevel": 2, "cardParameterType": "param1", "power": 11},
                    {"cardLevel": 1, "cardParameterType": "param3", "power": 30},
                    {"cardLevel": 2, "cardParameterType": "param2", "power": 21}
                ]
            }"#,
        )
        .expect("row card");

        let expected = vec![
            CardParameter {
                card_id: 7,
                level: 1,
                param1: 10,
                param2: 20,
                param3: 30,
            },
            CardParameter {
                card_id: 7,
                level: 2,
                param1: 11,
                param2: 21,
                param3: 31,
            },
        ];
        assert_eq!(flatten_card_parameters(&grouped), expected);
        assert_eq!(
            flatten_card_parameters(&rows),
            expected,
            "行式与分组式必须产出一致结果"
        );
    }

    #[test]
    fn card_parameters_rows_skip_levels_missing_a_dimension() {
        let rows: RawCard = serde_json::from_str(
            r#"{
                "id": 9,
                "characterId": 2,
                "cardRarityType": "rarity_3",
                "attr": "pure",
                "skillId": 5,
                "cardParameters": [
                    {"cardLevel": 1, "cardParameterType": "param1", "power": 1},
                    {"cardLevel": 1, "cardParameterType": "param2", "power": 2},
                    {"cardLevel": 1, "cardParameterType": "param3", "power": 3},
                    {"cardLevel": 2, "cardParameterType": "param1", "power": 4}
                ]
            }"#,
        )
        .expect("row card");
        assert_eq!(
            flatten_card_parameters(&rows),
            vec![CardParameter {
                card_id: 9,
                level: 1,
                param1: 1,
                param2: 2,
                param3: 3,
            }],
            "缺维度的等级不应产出半截行"
        );
    }
}
