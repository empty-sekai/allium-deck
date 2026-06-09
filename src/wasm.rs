//! wasm 浏览器入口（仅 `wasm` feature）。
//!
//! 薄壳：masterdata 编译期内嵌，调用方只传 user + params。内部全程复用引擎现有函数
//! （parse_* / build_card_pool / search / summarize_deck / game_id），无任何组卡逻辑复制——
//! 改组卡只动 `search/`+`handler/`，此入口自动跟随。

use std::collections::HashMap;

use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::engine::{parse_build_params_json, parse_user_profile_json};
use crate::handler::{build_card_pool, cultivated_user_cards, GameData, MasterCard, UserCard};
use crate::pool::CardPool;
use crate::search::{search, summarize_deck, DeckResult, SearchContext, SearchParams};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// 浏览器组卡入口。`user_json`/`params_json` 为上传链路 camelCase 格式。
/// 返回 top-5 卡组 JSON（真实游戏卡 ID + 展示指标）。
#[wasm_bindgen]
pub fn recommend_embedded(user_json: &str, params_json: &str) -> Result<String, JsValue> {
    let owned = crate::embedded::embedded_gamedata().map_err(to_js)?;
    let user = parse_user_profile_json(user_json).map_err(to_js)?;
    let params = parse_build_params_json(params_json).map_err(to_js)?;
    let game = owned.as_ref();

    let (pool, ctx) = build_card_pool(&user, &game, &params).map_err(to_js)?;
    let mut render_user = user.clone();
    render_user.user_cards = cultivated_user_cards(&user, &game, &params);
    let user_cards = render_user
        .user_cards
        .iter()
        .map(|card| (card.card_id, card))
        .collect::<HashMap<_, _>>();

    let results = search(
        &pool,
        &ctx,
        &SearchParams {
            top_k: 5,
            timeout_ms: 0,
        },
    );

    let decks: Vec<DeckOut> = results
        .iter()
        .enumerate()
        .map(|(index, result)| DeckOut::build(index + 1, &pool, &ctx, &game, &user_cards, result))
        .collect();

    serde_json::to_string(&DeckResponse { decks }).map_err(to_js)
}

fn to_js<E: std::fmt::Display>(err: E) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[derive(Serialize)]
struct DeckResponse {
    decks: Vec<DeckOut>,
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
    level: i32,
    skill_level: i32,
    skill_score_up: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_bonus: Option<f64>,
    master_rank: i32,
    trained: bool,
    episode1_read: bool,
    episode2_read: bool,
}

impl DeckOut {
    fn build(
        rank: usize,
        pool: &CardPool,
        ctx: &SearchContext,
        game: &GameData<'_>,
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
                            user_cards,
                            card_idx,
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
                            user_cards,
                            card_idx,
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
        user_cards: &HashMap<i32, &UserCard>,
        card_idx: crate::pool::CardIdx,
        skill_score_up: f64,
        event_bonus: Option<f64>,
    ) -> Self {
        let card_id = pool.game_id(card_idx) as i32;
        let user_card = user_cards.get(&card_id).copied();
        let trained = user_card
            .map(default_image_is_trained)
            .unwrap_or_else(|| ctx.trained_to_special_image_at(card_idx.raw()));
        let meta = card_meta(game, card_id, trained);
        Self {
            card_id,
            asset_key: meta.asset_key,
            rarity: meta.rarity,
            attr: meta.attr,
            level: user_card.map(|card| card.level).unwrap_or(0),
            skill_level: user_card.map(|card| card.skill_level).unwrap_or(0),
            skill_score_up,
            event_bonus,
            master_rank: user_card.map(|card| card.master_rank).unwrap_or(0),
            trained,
            episode1_read: user_card
                .map(|card| card.episodes_read.len() >= 1)
                .unwrap_or(false),
            episode2_read: user_card
                .map(|card| card.episodes_read.len() >= 2)
                .unwrap_or(false),
        }
    }
}

struct CardMeta {
    asset_key: String,
    rarity: String,
    attr: String,
}

fn card_meta(game: &GameData<'_>, card_id: i32, trained: bool) -> CardMeta {
    let training = if trained { "after_training" } else { "normal" };
    match game.cards.iter().find(|card| card.id == card_id) {
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
