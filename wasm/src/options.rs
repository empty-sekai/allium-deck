//! 辅助接口的 options JSON 解析。
//!
//! 键名以 snake_case 为主，同时兼容 camelCase 别名。

use serde_json::Value;

use allium_deck::handler::{Event, UserProfile};
use allium_deck::types::{EventType, LiveSkillOrder, LiveType};

use crate::to_js;

/// 解析 options JSON 原文。
pub(crate) fn parse_options(options_json: &str) -> Result<Value, wasm_bindgen::JsValue> {
    serde_json::from_str(options_json)
        .map_err(|err| wasm_bindgen::JsValue::from_str(&format!("options JSON 解析失败: {err}")))
}

/// 依次读取 `snake`/`camel` 两个键名。
pub(crate) fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    value.get(snake).or_else(|| value.get(camel))
}

pub(crate) fn opt_str(value: &Value, snake: &str, camel: &str) -> Option<String> {
    field(value, snake, camel)?.as_str().map(str::to_string)
}

pub(crate) fn opt_i32(value: &Value, snake: &str, camel: &str) -> Option<i32> {
    field(value, snake, camel)?.as_i64().map(|v| v as i32)
}

pub(crate) fn opt_bool(value: &Value, snake: &str, camel: &str) -> Option<bool> {
    field(value, snake, camel)?.as_bool()
}

fn required_str(value: &Value, snake: &str, camel: &str) -> Result<String, wasm_bindgen::JsValue> {
    opt_str(value, snake, camel)
        .ok_or_else(|| wasm_bindgen::JsValue::from_str(&format!("{snake} is required.")))
}

/// 用户数据提取优先级：`user_data_str` → `user_data`。
/// 两者均接受 JSON 字符串或已展开的对象。
pub(crate) fn user_from_options(opts: &Value) -> Result<UserProfile, wasm_bindgen::JsValue> {
    let raw = field(opts, "user_data_str", "userDataStr")
        .or_else(|| field(opts, "user_data", "userData"));
    let text = match raw {
        Some(Value::String(text)) => text.clone(),
        Some(value @ Value::Object(_)) => serde_json::to_string(value).map_err(to_js)?,
        _ => {
            return Err(wasm_bindgen::JsValue::from_str(
                "Either user_data / user_data_str is required.",
            ));
        }
    };
    allium_deck::engine::parse_user_profile_json(&text).map_err(to_js)
}

/// live_type 字符串 → 引擎枚举。
pub(crate) fn parse_live_type(value: &str) -> Result<LiveType, wasm_bindgen::JsValue> {
    match value.trim().to_ascii_lowercase().as_str() {
        "solo" => Ok(LiveType::Solo),
        "auto" => Ok(LiveType::Auto),
        "multi" => Ok(LiveType::Multi),
        "cheerful" => Ok(LiveType::Cheerful),
        "challenge" => Ok(LiveType::Challenge),
        "challenge_auto" | "challengeauto" => Ok(LiveType::ChallengeAuto),
        _ => Err(wasm_bindgen::JsValue::from_str(&format!(
            "Invalid live type: {value}"
        ))),
    }
}

pub(crate) fn require_live_type(opts: &Value) -> Result<LiveType, wasm_bindgen::JsValue> {
    parse_live_type(&required_str(opts, "live_type", "liveType")?)
}

/// event_type 字符串 → 引擎枚举；缺省 marathon。
pub(crate) fn parse_event_type(value: &str) -> Result<EventType, wasm_bindgen::JsValue> {
    match value.trim().to_ascii_lowercase().as_str() {
        "marathon" => Ok(EventType::Marathon),
        "cheerful_carnival" => Ok(EventType::CheerfulCarnival),
        "world_bloom" => Ok(EventType::WorldBloom),
        _ => Err(wasm_bindgen::JsValue::from_str(&format!(
            "Invalid event type: {value}"
        ))),
    }
}

/// 解析活动类型：显式 `event_type` 优先，否则用 `event_id` 反查活动主表。
pub(crate) fn event_type_from_options(
    opts: &Value,
    events: &[Event],
) -> Result<EventType, wasm_bindgen::JsValue> {
    if let Some(text) = opt_str(opts, "event_type", "eventType") {
        return parse_event_type(&text);
    }
    if let Some(event_id) = opt_i32(opts, "event_id", "eventId") {
        let event = events
            .iter()
            .find(|event| event.id == event_id)
            .ok_or_else(|| {
                wasm_bindgen::JsValue::from_str(&format!(
                    "Event not found for event_id: {event_id}"
                ))
            })?;
        return parse_event_type(&event.event_type);
    }
    Ok(EventType::Marathon)
}

/// `skill_order_choose_strategy` → 引擎枚举：
/// average→Average、max→Best、min→Worst、specific→Specific。
pub(crate) fn skill_order_from_options(
    opts: &Value,
) -> Result<LiveSkillOrder, wasm_bindgen::JsValue> {
    let strategy = opt_str(
        opts,
        "skill_order_choose_strategy",
        "skillOrderChooseStrategy",
    )
    .unwrap_or_else(|| "average".to_string());
    match strategy.as_str() {
        "average" => Ok(LiveSkillOrder::Average),
        "max" => Ok(LiveSkillOrder::Best),
        "min" => Ok(LiveSkillOrder::Worst),
        "specific" => Ok(LiveSkillOrder::Specific),
        _ => Err(wasm_bindgen::JsValue::from_str(&format!(
            "Invalid skill order choose strategy: {strategy}"
        ))),
    }
}

/// 曲目推荐的 deck 输入子集——只消费总战力/加成率/技能行。
#[derive(Default, serde::Deserialize)]
#[serde(default)]
pub(crate) struct DeckIn {
    pub total_power: i32,
    pub event_bonus_rate: f64,
    pub support_deck_bonus_rate: f64,
    pub cards: Vec<DeckCardIn>,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
pub(crate) struct DeckCardIn {
    pub skill_score_up: f64,
    pub skill_life_recovery: f64,
}

impl DeckIn {
    pub(crate) fn from_options(opts: &Value) -> Result<Self, wasm_bindgen::JsValue> {
        let deck = field(opts, "deck", "deck")
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("deck is required."))?;
        if !deck.is_object() {
            return Err(wasm_bindgen::JsValue::from_str("deck must be an object."));
        }
        serde_json::from_value(deck.clone())
            .map_err(|err| wasm_bindgen::JsValue::from_str(&format!("deck JSON 解析失败: {err}")))
    }
}
