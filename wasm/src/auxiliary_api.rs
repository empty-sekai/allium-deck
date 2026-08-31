//! 辅助计算的 wasm 导出：区域道具推荐 / 曲目推荐 / 精确打歌分 / WL 支援卡。
//!
//! 核心算法在本 crate 的 `auxiliary` 与 `handler::world_bloom` 中，
//! 此文件只做 JSON 壳；输出键名为 snake_case。

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use allium_deck::auxiliary::{
    recommend_music, MusicDeck, MusicDeckCard, MusicRecommendOptions,
};
use allium_deck::handler::{world_bloom_support_cards, BuildParams};

use crate::options::{
    event_type_from_options, field, opt_bool, opt_i32, opt_str, parse_options, require_live_type,
    skill_order_from_options, user_from_options, DeckIn,
};
use crate::{engine_data, to_js};

/// 区域道具推荐。
///
/// options：`{ user_data / user_data_str, card_ids: [..] }`；
/// 返回按 `power_per_coin` 降序的升级建议数组。
#[wasm_bindgen]
pub fn recommend_area_items(options_json: &str) -> Result<String, wasm_bindgen::JsValue> {
    let opts = parse_options(options_json)?;
    let data = engine_data()?;
    let user = user_from_options(&opts)?;
    let game = data.game.as_ref();

    let card_ids = field(&opts, "card_ids", "cardIds")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_i64().map(|v| v as i32))
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("card_ids is required."))?;

    let result = data
        .auxiliary
        .recommend_area_items(&user, &game, &card_ids)
        .map_err(to_js)?;
    serde_json::to_string(&result).map_err(to_js)
}

/// 曲目推荐：对一张已定卡组给全部曲目/难度打分排序。
///
/// options：`{ deck, live_type, event_type?/event_id?, skill_order_choose_strategy?,
/// specific_skill_order?, multi_live_teammate_score_up?, multi_live_teammate_power? }`。
#[wasm_bindgen(js_name = recommendMusic)]
pub fn recommend_music_api(options_json: &str) -> Result<String, wasm_bindgen::JsValue> {
    let opts = parse_options(options_json)?;
    let data = engine_data()?;
    let game = data.game.as_ref();

    // 烤森 multi → cheerful 的转换已内置于核心 recommend_music。
    let live_type = require_live_type(&opts)?;
    let event_type = event_type_from_options(&opts, &game.events)?;

    let specific_skill_order = field(&opts, "specific_skill_order", "specificSkillOrder")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_u64().map(|v| v as usize))
                .collect::<Vec<_>>()
        });

    let deck_json = DeckIn::from_options(&opts)?;
    let deck = MusicDeck {
        total_power: deck_json.total_power,
        event_bonus_rate: deck_json.event_bonus_rate,
        support_deck_bonus_rate: deck_json.support_deck_bonus_rate,
        cards: deck_json
            .cards
            .iter()
            .map(|card| MusicDeckCard {
                skill_score_up: card.skill_score_up,
                skill_life_recovery: card.skill_life_recovery,
            })
            .collect(),
    };

    let options = MusicRecommendOptions {
        live_type,
        event_type,
        skill_order: skill_order_from_options(&opts)?,
        specific_skill_order,
        multi_teammate_score_up: opt_i32(&opts, "multi_live_teammate_score_up", "multiLiveTeammateScoreUp"),
        multi_teammate_power: opt_i32(&opts, "multi_live_teammate_power", "multiLiveTeammatePower"),
    };

    let result = recommend_music(game.music_metas, &deck, &options)
    .map_err(to_js)?;
    serde_json::to_string(&result).map_err(to_js)
}

/// 精确打歌分：给定战力/技能/谱面逐 note 计算。
///
/// options：`{ live_type, power, skills: [..], music_score, fever_music_score?,
/// multi_sum_power? }`；`music_score`/`fever_music_score` 接受 JSON 字符串或对象。
#[wasm_bindgen]
pub fn calculate_exact_live(options_json: &str) -> Result<String, wasm_bindgen::JsValue> {
    let opts = parse_options(options_json)?;
    let data = engine_data()?;

    let live_type = require_live_type(&opts)?;
    let power = opt_i32(&opts, "power", "power").filter(|power| *power > 0).ok_or_else(|| {
        wasm_bindgen::JsValue::from_str("power must be positive.")
    })?;
    let skills = field(&opts, "skills", "skills")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_f64())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let music_score = json_text(&opts, "music_score", "musicScore")
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("music_score is required."))?;
    let fever_music_score = json_text(&opts, "fever_music_score", "feverMusicScore");
    let multi_sum_power = opt_i32(&opts, "multi_sum_power", "multiSumPower").unwrap_or(0);

    let detail = data
        .auxiliary
        .calculate_exact_live(
            power,
            &skills,
            live_type,
            &music_score,
            multi_sum_power,
            fever_music_score.as_deref(),
        )
        .map_err(to_js)?;
    serde_json::to_string(&detail).map_err(to_js)
}

#[derive(Serialize)]
struct SupportCardOut {
    card_id: i32,
    bonus: f64,
}

/// World Bloom 支援卡逐卡加成。
///
/// options：`{ user_data / user_data_str, event_id | world_bloom_event_turn |
/// world_bloom_finale_turn, world_bloom_character_id?, event_unit?,
/// forced_leader_character_id?, support_master_max?, support_skill_max?,
/// filter_other_unit? }`；返回按 (bonus 降序, card_id 升序) 排序的数组。
#[wasm_bindgen]
pub fn get_world_bloom_support_cards(options_json: &str) -> Result<String, wasm_bindgen::JsValue> {
    let opts = parse_options(options_json)?;
    let data = engine_data()?;
    let user = user_from_options(&opts)?;
    let game = data.game.as_ref();

    let params = BuildParams {
        event_id: opt_i32(&opts, "event_id", "eventId"),
        world_bloom_event_turn: opt_i32(&opts, "world_bloom_event_turn", "worldBloomEventTurn"),
        world_bloom_finale_turn: opt_i32(&opts, "world_bloom_finale_turn", "worldBloomFinaleTurn"),
        world_bloom_character_id: opt_i32(&opts, "world_bloom_character_id", "worldBloomCharacterId"),
        forced_leader_character_id: opt_i32(
            &opts,
            "forced_leader_character_id",
            "forcedLeaderCharacterId",
        ),
        event_unit: opt_str(&opts, "event_unit", "eventUnit"),
        ..BuildParams::default()
    };

    let mut result = world_bloom_support_cards(
        &user,
        &game,
        &params,
        opt_bool(&opts, "support_master_max", "supportMasterMax").unwrap_or(false),
        opt_bool(&opts, "support_skill_max", "supportSkillMax").unwrap_or(false),
        opt_bool(&opts, "filter_other_unit", "filterOtherUnit").unwrap_or(false),
    )
    .map_err(to_js)?;
    // 按 (bonus 降序, card_id 升序) 排序后仅输出两字段（平局时 card_id 小者在前）。
    result.sort_by(|left, right| {
        right
            .bonus
            .total_cmp(&left.bonus)
            .then_with(|| left.card_id.cmp(&right.card_id))
    });
    let out = result
        .iter()
        .map(|card| SupportCardOut {
            card_id: card.card_id,
            bonus: card.bonus,
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&out).map_err(to_js)
}

/// `music_score` 类字段统一取原文：字符串透传，对象重新序列化。
fn json_text(opts: &serde_json::Value, snake: &str, camel: &str) -> Option<String> {
    match field(opts, snake, camel)? {
        serde_json::Value::String(text) => Some(text.clone()),
        value @ serde_json::Value::Object(_) => serde_json::to_string(value).ok(),
        _ => None,
    }
}
