#![cfg(target_arch = "wasm32")]

//! Browser WASM entry point for the standalone npm package.
//!
//! masterdata 由调用方提供：浏览器侧 JS 通常已持有 masterdata JSON，
//! `load_masterdata(map, metas)` 直接复用同一份数据——引擎内一次性完成
//! 「raw JSON → 扁平结构」转换并缓存，零额外下载，数据新鲜度与代码发版解耦。
//!
//! 导出面：`load_masterdata` / `recommend` / `createUserData` +
//! `recommendWithUserData` / `recommend_area_items` / `recommendMusic` /
//! `calculate_exact_live` / `get_world_bloom_support_cards`
//! （见 `auxiliary_api.rs`；不提供批量入口）。
//!
//! 薄壳：无组卡逻辑复制——改组卡只动 `search/`+`handler/`，此入口自动跟随。

mod auxiliary_api;
mod options;

use serde::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use wasm_bindgen::prelude::*;

use allium_deck::auxiliary::AuxiliaryData;
use allium_deck::engine::{OwnedGameData, parse_build_params_json, parse_user_profile_json};
use allium_deck::handler::{
    GameData, MasterCard, UserCard, UserProfile, build_card_pool, cultivated_user_cards,
};
use allium_deck::pool::CardPool;
use allium_deck::search::{
    DeckResult, SearchContext, SearchParams, search_targets, summarize_deck,
};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn performance_now() -> f64;
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// 一份已扁平化的引擎数据：组卡用主数据 + 辅助接口用附加表。
pub(crate) struct EngineData {
    game: OwnedGameData,
    auxiliary: AuxiliaryData,
}

thread_local! {
    /// `load_masterdata` 一次性扁平化后的数据，整个页面生命周期复用。
    static MASTER_DATA: RefCell<Option<std::rc::Rc<EngineData>>> = const { RefCell::new(None) };
}

/// 取当前引擎数据；未初始化时直接报错。
pub(crate) fn engine_data() -> Result<std::rc::Rc<EngineData>, JsValue> {
    MASTER_DATA
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| JsValue::from_str("masterdata 未初始化：请先调用 load_masterdata(...)"))
}

/// 载入 masterdata：`masterdata_json` 形如
/// `{"cards": "<cards.json 原文>", "events": "...", ...}`（各表 JSON 原文，
/// 键名裸表名或带 `.json` 后缀均可），`music_metas_json` 为音乐元数据表
/// 原文（可为空字符串）。
///
/// 数据在引擎内做一次「raw JSON → 扁平结构」转换并缓存，之后每次
/// recommend 零重复成本。辅助表（areas/areaItems/shopItems/ingameNotes/
/// ingameCombos）缺省时对应辅助接口报「未载入」错误，不影响组卡。
#[wasm_bindgen]
pub fn load_masterdata(masterdata_json: &str, music_metas_json: &str) -> Result<(), JsValue> {
    let raw: BTreeMap<String, String> = serde_json::from_str(masterdata_json)
        .map_err(|err| JsValue::from_str(&format!("masterdata JSON 解析失败: {err}")))?;
    // 统一为文件名键（含 .json 后缀）：调用方传裸表名时自动补全。
    let map = raw
        .into_iter()
        .map(|(mut name, text)| {
            if !name.ends_with(".json") {
                name.push_str(".json");
            }
            (name, text)
        })
        .collect::<BTreeMap<_, _>>();
    let auxiliary = AuxiliaryData::from_strings(&map).map_err(to_js)?;
    let sources = allium_deck::engine::MasterdataSources::from_strings(
        map.into_iter(),
        music_metas_json.to_string(),
    );
    let owned = allium_deck::engine::OwnedGameData::from_sources(&sources)
        .map_err(|err| JsValue::from_str(&err))?;
    MASTER_DATA.with(|slot| {
        *slot.borrow_mut() = Some(std::rc::Rc::new(EngineData {
            game: owned,
            auxiliary,
        }));
    });
    Ok(())
}

/// 组卡入口。需先 `load_masterdata`。`user_json`/`params_json` 为上传链路
/// camelCase 格式；返回卡组 JSON（真实游戏卡 ID + 展示指标），条数由
/// params 的 `limit` 决定（缺省 10，上限 30）。
#[wasm_bindgen]
pub fn recommend(user_json: &str, params_json: &str) -> Result<String, JsValue> {
    let data = engine_data()?;
    let user = parse_user_profile_json(user_json).map_err(to_js)?;
    recommend_with_user(&data, &user, params_json)
}

/// 解析一次用户数据、多次复用的句柄。
///
/// `region` 词表：jp/tw/en/kr/cn。句柄与 masterdata 生命周期解耦：
/// masterdata 重载后旧句柄仍可用，但数据视图可能过期，由调用方自行重载。
#[wasm_bindgen]
pub struct UserDataHandle {
    region: String,
    user: UserProfile,
}

#[wasm_bindgen]
impl UserDataHandle {
    #[wasm_bindgen(getter)]
    pub fn region(&self) -> String {
        self.region.clone()
    }
}

/// 创建用户数据句柄：解析成本只付一次，后续 `recommend_with_user_data`
/// 直接复用（解析成本只付一次）。
#[wasm_bindgen]
pub fn create_user_data(user_json: &str, region: &str) -> Result<UserDataHandle, JsValue> {
    if !matches!(region, "jp" | "tw" | "en" | "kr" | "cn") {
        return Err(JsValue::from_str(&format!("Invalid region: {region}")));
    }
    let user = parse_user_profile_json(user_json).map_err(to_js)?;
    Ok(UserDataHandle {
        region: region.to_string(),
        user,
    })
}

/// 组卡入口（句柄式）：options 即 `recommend` 的 `params_json`。
#[wasm_bindgen(js_name = recommendWithUserData)]
pub fn recommend_with_user_data(
    options_json: &str,
    handle: &UserDataHandle,
) -> Result<String, JsValue> {
    let data = engine_data()?;
    recommend_with_user(&data, &handle.user, options_json)
}

fn recommend_with_user(
    data: &std::rc::Rc<EngineData>,
    user: &UserProfile,
    params_json: &str,
) -> Result<String, JsValue> {
    let params = parse_build_params_json(params_json).map_err(to_js)?;
    // OwnedGameData::as_ref 返回借用视图（非拷贝本体），生命周期跟随 data。
    let game: &GameData<'_> = &data.game.as_ref();

    let build_pool_start = performance_now();
    let (pool, ctx) = build_card_pool(user, game, &params).map_err(to_js)?;
    let build_pool_ms = elapsed_ms(build_pool_start);
    // 只需要养成态卡表本身；克隆整个 UserProfile 后立刻覆盖 user_cards 是纯浪费。
    let cultivated = cultivated_user_cards(user, game, &params);
    let user_cards = cultivated
        .iter()
        .map(|card| (card.card_id, card))
        .collect::<HashMap<_, _>>();

    let search_start = performance_now();
    // 走统一搜索入口（与 engine::recommend 同一条分派路径）：
    // 无档位 → 完整搜索流水线；有档位 → 逐档独立 Top-K。
    let results = search_targets(
        &pool,
        &ctx,
        &SearchParams {
            top_k: params.limit.clamp(1, 30),
            timeout_ms: params.timeout_ms,
        },
        &params.target_bonus_list,
    );
    let search_ms = elapsed_ms(search_start);

    // 每张输出卡都线性扫 cards 主表的话是 top_k × 5 次全表扫描；先建一次索引。
    let master_cards = game
        .cards
        .iter()
        .map(|card| (card.id, card))
        .collect::<HashMap<_, _>>();
    let decks: Vec<DeckOut> = results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            DeckOut::build(
                index + 1,
                &pool,
                &ctx,
                game,
                &master_cards,
                user,
                &user_cards,
                result,
            )
        })
        .collect();

    serde_json::to_string(&DeckResponse {
        decks,
        performance: DeckPerformance {
            build_pool_ms,
            search_ms,
            pool_size: pool.count(),
        },
    })
    .map_err(to_js)
}

fn to_js<E: std::fmt::Display>(err: E) -> JsValue {
    JsValue::from_str(&err.to_string())
}

fn elapsed_ms(start: f64) -> f64 {
    performance_now() - start
}

#[derive(Serialize)]
struct DeckResponse {
    decks: Vec<DeckOut>,
    performance: DeckPerformance,
}

#[derive(Serialize)]
struct DeckPerformance {
    build_pool_ms: f64,
    search_ms: f64,
    pool_size: usize,
}

#[derive(Serialize)]
struct DeckOut {
    rank: usize,
    cards: Vec<CardOut>,
    total_power: i32,
    live_score: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_point: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_value: Option<i64>,
    skill_score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    multi_live_score_up: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_bonus_total: Option<f64>,
}

#[derive(Serialize)]
struct CardOut {
    card_id: i32,
    asset_key: String,
    rarity: String,
    attr: String,
    character_id: u8,
    attr_id: u8,
    unit_mask_raw: u8,
    level: i32,
    skill_level: i32,
    skill_score_up: f64,
    power_total: i32,
    pool_power_max: u32,
    pool_skill_min: u8,
    pool_skill_max: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_bonus: Option<f64>,
    master_rank: i32,
    trained: bool,
    has_canvas_bonus: bool,
    canvas_power: i32,
    episode1_read: bool,
    episode2_read: bool,
}

impl DeckOut {
    fn build(
        rank: usize,
        pool: &CardPool,
        ctx: &SearchContext,
        game: &GameData<'_>,
        master_cards: &HashMap<i32, &MasterCard>,
        original_user: &UserProfile,
        user_cards: &HashMap<i32, &UserCard>,
        result: &DeckResult,
    ) -> Self {
        // summarize_deck 给出展示指标 + 站位顺序（ordered_cards）。失败时回退裸结果顺序。
        match summarize_deck(pool, ctx, &result.cards) {
            Some(summary) => {
                let per_card = (0..5)
                    .map(|card_pos| {
                        let card_idx = summary.ordered_cards[card_pos];
                        CardOut::build(
                            pool,
                            ctx,
                            game,
                            master_cards,
                            original_user,
                            user_cards,
                            card_idx,
                            summary.card_power_total[card_pos],
                            summary.card_skill_score_up[card_pos],
                            (summary.card_event_bonus_rates[card_pos] > 0.0)
                                .then_some(summary.card_event_bonus_rates[card_pos]),
                        )
                    })
                    .collect();
                Self {
                    rank,
                    cards: per_card,
                    total_power: summary.total_power,
                    live_score: summary.live_score,
                    event_point: summary.event_point,
                    target_value: Some(target_value(result)),
                    skill_score: summary.multi_live_score_up,
                    multi_live_score_up: Some(summary.multi_live_score_up),
                    event_bonus_total: summary.event_bonus_total,
                }
            }
            None => {
                let cards = result
                    .cards
                    .iter()
                    .map(|&card_idx| {
                        CardOut::build(
                            pool,
                            ctx,
                            game,
                            master_cards,
                            original_user,
                            user_cards,
                            card_idx,
                            pool.power_max(card_idx).min(i32::MAX as u32) as i32,
                            f64::from(pool.skill_max(card_idx)),
                            None,
                        )
                    })
                    .collect();
                Self {
                    rank,
                    cards,
                    total_power: result
                        .cards
                        .iter()
                        .map(|card| pool.power_max(*card) as i32)
                        .sum(),
                    live_score: live_score_from_result(result),
                    event_point: None,
                    target_value: Some(target_value(result)),
                    skill_score: result
                        .cards
                        .iter()
                        .map(|card| f64::from(pool.skill_max(*card)))
                        .sum(),
                    multi_live_score_up: None,
                    event_bonus_total: None,
                }
            }
        }
    }
}

impl CardOut {
    fn build(
        pool: &CardPool,
        ctx: &SearchContext,
        game: &GameData<'_>,
        master_cards: &HashMap<i32, &MasterCard>,
        original_user: &UserProfile,
        user_cards: &HashMap<i32, &UserCard>,
        card_idx: allium_deck::pool::CardIdx,
        power_total: i32,
        skill_score_up: f64,
        event_bonus: Option<f64>,
    ) -> Self {
        let card_id = pool.game_id(card_idx) as i32;
        let user_card = user_cards.get(&card_id).copied();
        let trained = user_card
            .map(default_image_is_trained)
            .unwrap_or_else(|| ctx.trained_to_special_image_at(card_idx.raw()));
        let meta = card_meta(master_cards, card_id, trained);
        let has_canvas_bonus = user_card
            .and_then(|card| card.has_canvas_bonus_override)
            .unwrap_or_else(|| {
                original_user
                    .user_mysekai_canvas_bonus_cards
                    .contains(&card_id)
            });
        Self {
            card_id,
            asset_key: meta.asset_key,
            rarity: meta.rarity.clone(),
            attr: meta.attr,
            character_id: pool.char_id(card_idx),
            attr_id: pool.attr(card_idx),
            unit_mask_raw: pool.unit_mask_raw(card_idx),
            level: user_card.map(|card| card.level).unwrap_or(0),
            skill_level: user_card.map(|card| card.skill_level).unwrap_or(0),
            skill_score_up,
            power_total,
            pool_power_max: pool.power_max(card_idx),
            pool_skill_min: pool.skill_min(card_idx),
            pool_skill_max: pool.skill_max(card_idx),
            event_bonus,
            master_rank: user_card.map(|card| card.master_rank).unwrap_or(0),
            trained,
            has_canvas_bonus,
            canvas_power: canvas_power(game, &meta.rarity, has_canvas_bonus),
            episode1_read: user_card
                .map(|card| card.episodes_read.len() >= 1)
                .unwrap_or(false),
            episode2_read: user_card
                .map(|card| card.episodes_read.len() >= 2)
                .unwrap_or(false),
        }
    }
}

fn canvas_power(game: &GameData<'_>, rarity: &str, enabled: bool) -> i32 {
    if !enabled {
        return 0;
    }
    let rarity_type = rarity_type_to_index(rarity);
    game.card_mysekai_canvas_bonuses
        .iter()
        .find(|bonus| bonus.card_rarity_type == rarity_type)
        .map(|bonus| bonus.power1_bonus_fixed + bonus.power2_bonus_fixed + bonus.power3_bonus_fixed)
        .unwrap_or(0)
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

struct CardMeta {
    asset_key: String,
    rarity: String,
    attr: String,
}

fn card_meta(master_cards: &HashMap<i32, &MasterCard>, card_id: i32, trained: bool) -> CardMeta {
    let training = if trained { "after_training" } else { "normal" };
    match master_cards.get(&card_id).copied() {
        Some(MasterCard {
            asset_bundle_name,
            rarity,
            attr,
            ..
        }) if !asset_bundle_name.is_empty() => CardMeta {
            asset_key: format!("thumbnail/chara/{asset_bundle_name}_{training}"),
            rarity: rarity.clone(),
            attr: attr.clone(),
        },
        Some(card) => CardMeta {
            asset_key: format!("thumbnail/chara/{card_id}_{training}"),
            rarity: card.rarity.clone(),
            attr: card.attr.clone(),
        },
        None => CardMeta {
            asset_key: format!("thumbnail/chara/{card_id}_{training}"),
            rarity: "rarity_4".to_string(),
            attr: "cool".to_string(),
        },
    }
}

fn default_image_is_trained(card: &UserCard) -> bool {
    matches!(
        card.default_image.trim().to_ascii_lowercase().as_str(),
        "special_training" | "trained" | "after_training"
    )
}

fn target_value(result: &DeckResult) -> i64 {
    result.score.min(i64::MAX as u64) as i64
}

fn live_score_from_result(result: &DeckResult) -> i32 {
    let high = result.score >> 32;
    let value = if high > 0 { high } else { result.score };
    value.min(i32::MAX as u64) as i32
}
