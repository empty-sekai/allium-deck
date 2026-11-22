//! Auxiliary endpoints: the calculations that surround deck building.
//!
//! Every one of these runs on a search thread too. They are lighter than a deck search
//! but still CPU-bound, and keeping them off the runtime means one endpoint cannot
//! stall connection handling for the others.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use allium_deck::auxiliary::{
    AreaItemRecommendation, ExactLiveDetail, MusicDeck, MusicDeckCard, MusicRecommendOptions,
    MusicRecommendation, recommend_music_with_ceiling,
};
use allium_deck::handler::{
    BuildParams, parse_unit_code, resolve_event_score_ceiling,
    world_bloom_support_cards as support_cards_core,
};
use allium_deck::types::{
    EventType, LiveSkillOrder, LiveType, is_memory_event_id, memory_event_fake_id,
};

use super::{ApiError, ApiJson, AppState, UserInput, json_text, parse_params_for_region};
use crate::metrics::Outcome;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportCardOut {
    pub card_id: i32,
    pub bonus: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportCardsRequest {
    pub region: Option<String>,
    pub user: UserInput,
    /// Build parameters; the World Bloom keys are the ones that matter here.
    pub params: Option<Box<RawValue>>,
    #[serde(default, alias = "support_master_max")]
    pub support_master_max: bool,
    #[serde(default, alias = "support_skill_max")]
    pub support_skill_max: bool,
    #[serde(default, alias = "filter_other_unit")]
    pub filter_other_unit: bool,
}

/// `POST /v1/world-bloom/support-cards`
pub async fn world_bloom_support_cards(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<SupportCardsRequest>,
) -> Result<Json<Vec<SupportCardOut>>, ApiError> {
    let started = Instant::now();
    let result = run_support_cards(&state, request).await;
    record(&state, "world_bloom_support_cards", started, result)
}

async fn run_support_cards(
    state: &Arc<AppState>,
    request: SupportCardsRequest,
) -> Result<Vec<SupportCardOut>, ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let user = request.user.parse()?;
    let params: BuildParams = parse_params_for_region(request.params.as_deref(), &snapshot.name)?;
    let (master_max, skill_max, other_unit) = (
        request.support_master_max,
        request.support_skill_max,
        request.filter_other_unit,
    );

    let job = state
        .pool
        .execute(move || {
            let game = snapshot.game();
            support_cards_core(&user, &game, &params, master_max, skill_max, other_unit)
                .map_err(|error| ApiError::BadRequest(error.to_string()))
        })
        .await?;

    let mut cards = job.value?;
    // Strongest first, then by card id so ties are stable across calls.
    cards.sort_by(|left, right| {
        right
            .bonus
            .total_cmp(&left.bonus)
            .then_with(|| left.card_id.cmp(&right.card_id))
    });
    Ok(cards
        .into_iter()
        .map(|card| SupportCardOut {
            card_id: card.card_id,
            bonus: card.bonus,
        })
        .collect())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeckIn {
    #[serde(alias = "total_power")]
    pub total_power: i32,
    #[serde(alias = "event_bonus_rate")]
    pub event_bonus_rate: f64,
    #[serde(alias = "support_deck_bonus_rate")]
    pub support_deck_bonus_rate: f64,
    pub cards: Vec<DeckCardIn>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeckCardIn {
    #[serde(alias = "skill_score_up")]
    pub skill_score_up: f64,
    #[serde(alias = "skill_life_recovery")]
    pub skill_life_recovery: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicRecommendRequest {
    pub region: Option<String>,
    /// The already-chosen deck to score every song against.
    pub deck: DeckIn,
    #[serde(alias = "live_type")]
    pub live_type: String,
    #[serde(default, alias = "event_type")]
    pub event_type: Option<String>,
    #[serde(default, alias = "event_id")]
    pub event_id: Option<i32>,
    #[serde(default, alias = "event_me_unit")]
    pub event_me_unit: Option<String>,
    #[serde(
        default,
        alias = "skill_order_choose_strategy",
        alias = "liveSkillOrder"
    )]
    pub skill_order_choose_strategy: Option<String>,
    #[serde(default, alias = "specific_skill_order")]
    pub specific_skill_order: Option<Vec<usize>>,
    #[serde(default, alias = "multi_live_teammate_score_up")]
    pub multi_live_teammate_score_up: Option<i32>,
    #[serde(default, alias = "multi_live_teammate_power")]
    pub multi_live_teammate_power: Option<i32>,
}

/// `POST /v1/music/recommend`
pub async fn music_recommend(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<MusicRecommendRequest>,
) -> Result<Json<Vec<MusicRecommendation>>, ApiError> {
    let started = Instant::now();
    let result = run_music_recommend(&state, request).await;
    record(&state, "music_recommend", started, result)
}

async fn run_music_recommend(
    state: &Arc<AppState>,
    request: MusicRecommendRequest,
) -> Result<Vec<MusicRecommendation>, ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let live_type = parse_live_type(&request.live_type)?;
    let skill_order = parse_skill_order(request.skill_order_choose_strategy.as_deref())?;

    let deck = MusicDeck {
        total_power: request.deck.total_power,
        event_bonus_rate: request.deck.event_bonus_rate,
        support_deck_bonus_rate: request.deck.support_deck_bonus_rate,
        cards: request
            .deck
            .cards
            .iter()
            .map(|card| MusicDeckCard {
                skill_score_up: card.skill_score_up,
                skill_life_recovery: card.skill_life_recovery,
            })
            .collect(),
    };

    let event_type_text = request.event_type.clone();
    let event_id = request.event_id;
    let event_me_unit = request.event_me_unit.clone();
    let specific_skill_order = request.specific_skill_order.clone();
    let teammate_score_up = request.multi_live_teammate_score_up;
    let teammate_power = request.multi_live_teammate_power;

    let job = state
        .pool
        .execute(move || {
            let game = snapshot.game();
            let event_type = resolve_event_type(
                event_type_text.as_deref(),
                event_id,
                event_me_unit.as_deref(),
                game.events,
            )?;
            let options = MusicRecommendOptions {
                live_type,
                event_type,
                skill_order,
                specific_skill_order,
                multi_teammate_score_up: teammate_score_up,
                multi_teammate_power: teammate_power,
            };
            let ceiling = resolve_event_score_ceiling(&game)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?;
            recommend_music_with_ceiling(game.music_metas, &deck, &options, ceiling)
                .map_err(ApiError::BadRequest)
        })
        .await?;
    job.value
}

/// A chart, accepted as an object or as its JSON text.
#[derive(Debug, Deserialize)]
pub struct ChartInput(Box<RawValue>);

impl ChartInput {
    fn text(&self) -> Result<String, ApiError> {
        json_text(&self.0)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactLiveRequest {
    pub region: Option<String>,
    #[serde(alias = "live_type")]
    pub live_type: String,
    pub power: i32,
    #[serde(default)]
    pub skills: Vec<f64>,
    #[serde(alias = "music_score")]
    pub music_score: ChartInput,
    #[serde(default, alias = "fever_music_score")]
    pub fever_music_score: Option<ChartInput>,
    #[serde(default, alias = "multi_sum_power")]
    pub multi_sum_power: Option<i32>,
}

/// `POST /v1/live/exact-score`
pub async fn live_exact_score(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<ExactLiveRequest>,
) -> Result<Json<ExactLiveDetail>, ApiError> {
    let started = Instant::now();
    let result = run_exact_live(&state, request).await;
    record(&state, "live_exact_score", started, result)
}

async fn run_exact_live(
    state: &Arc<AppState>,
    request: ExactLiveRequest,
) -> Result<ExactLiveDetail, ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let live_type = parse_live_type(&request.live_type)?;
    if request.power <= 0 {
        return Err(ApiError::BadRequest("power must be positive".to_string()));
    }

    let power = request.power;
    let skills = request.skills.clone();
    let music_score = request.music_score.text()?;
    let fever = request
        .fever_music_score
        .as_ref()
        .map(ChartInput::text)
        .transpose()?;
    let multi_sum_power = request.multi_sum_power.unwrap_or(0);

    let job = state
        .pool
        .execute(move || {
            snapshot
                .auxiliary()
                .calculate_exact_live(
                    power,
                    &skills,
                    live_type,
                    &music_score,
                    multi_sum_power,
                    fever.as_deref(),
                )
                .map_err(ApiError::BadRequest)
        })
        .await?;
    job.value
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaItemsRequest {
    pub region: Option<String>,
    pub user: UserInput,
    #[serde(alias = "card_ids")]
    pub card_ids: Vec<i32>,
    #[serde(default, alias = "multi_unit_bonus_evaluation")]
    pub multi_unit_bonus_evaluation: allium_deck::power::MultiUnitBonusMode,
}

/// `POST /v1/area-items/recommend`
pub async fn area_items_recommend(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<AreaItemsRequest>,
) -> Result<Json<Vec<AreaItemRecommendation>>, ApiError> {
    let started = Instant::now();
    let result = run_area_items(&state, request).await;
    record(&state, "area_items_recommend", started, result)
}

async fn run_area_items(
    state: &Arc<AppState>,
    request: AreaItemsRequest,
) -> Result<Vec<AreaItemRecommendation>, ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let user = request.user.parse()?;
    let card_ids = request.card_ids.clone();

    let job = state
        .pool
        .execute(move || {
            let game = snapshot.game();
            snapshot
                .auxiliary()
                .recommend_area_items_for_region(
                    &user,
                    &game,
                    &card_ids,
                    request.multi_unit_bonus_evaluation,
                    &snapshot.name,
                )
                .map_err(ApiError::BadRequest)
        })
        .await?;
    job.value
}

/// Accepts every spelling the request contract documents.
fn parse_live_type(value: &str) -> Result<LiveType, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "solo" => Ok(LiveType::Solo),
        "auto" => Ok(LiveType::Auto),
        "multi" => Ok(LiveType::Multi),
        "cheerful" => Ok(LiveType::Cheerful),
        "challenge" => Ok(LiveType::Challenge),
        "challenge_auto" | "challengeauto" => Ok(LiveType::ChallengeAuto),
        "mysekai" => Ok(LiveType::Mysekai),
        other => Err(ApiError::BadRequest(format!("invalid live type: {other}"))),
    }
}

fn parse_skill_order(value: Option<&str>) -> Result<LiveSkillOrder, ApiError> {
    match value.unwrap_or("average") {
        "average" => Ok(LiveSkillOrder::Average),
        "max" | "best" => Ok(LiveSkillOrder::Best),
        "min" | "worst" => Ok(LiveSkillOrder::Worst),
        "specific" => Ok(LiveSkillOrder::Specific),
        other => Err(ApiError::BadRequest(format!(
            "invalid skill order strategy: {other}"
        ))),
    }
}

/// Explicit `eventType` / `eventMeUnit` wins; synthetic Memory Event IDs resolve to
/// marathon; otherwise `eventId` is looked up in the event table.
fn resolve_event_type(
    event_type: Option<&str>,
    event_id: Option<i32>,
    event_me_unit: Option<&str>,
    events: &[allium_deck::handler::Event],
) -> Result<EventType, ApiError> {
    if let Some(unit_raw) = event_me_unit {
        if parse_unit_code(unit_raw)
            .and_then(memory_event_fake_id)
            .is_none()
        {
            return Err(ApiError::BadRequest(format!(
                "invalid memory event unit: {unit_raw}"
            )));
        }
        if event_id.is_some()
            || event_type
                .is_some_and(|text| !matches!(parse_event_type(text), Ok(EventType::Marathon)))
        {
            return Err(ApiError::BadRequest(
                "event_me_unit requires a marathon simulation without other event selectors"
                    .to_string(),
            ));
        }
        return Ok(EventType::Marathon);
    }
    if let Some(text) = event_type {
        return parse_event_type(text);
    }
    if let Some(event_id) = event_id {
        if is_memory_event_id(event_id) {
            return Ok(EventType::Marathon);
        }
        let event = events
            .iter()
            .find(|event| event.id == event_id)
            .ok_or_else(|| ApiError::BadRequest(format!("no event with id {event_id}")))?;
        return parse_event_type(&event.event_type);
    }
    Ok(EventType::Marathon)
}

fn parse_event_type(value: &str) -> Result<EventType, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "marathon" => Ok(EventType::Marathon),
        "cheerful_carnival" | "cheerful" => Ok(EventType::CheerfulCarnival),
        "world_bloom" | "wl" => Ok(EventType::WorldBloom),
        other => Err(ApiError::BadRequest(format!("invalid event type: {other}"))),
    }
}

/// Records the request against the endpoint's metrics and wraps the body.
fn record<T>(
    state: &Arc<AppState>,
    endpoint: &str,
    started: Instant,
    result: Result<T, ApiError>,
) -> Result<Json<T>, ApiError> {
    let seconds = started.elapsed().as_secs_f64();
    match result {
        Ok(value) => {
            state.metrics.record(endpoint, Outcome::Ok, seconds);
            Ok(Json(value))
        }
        Err(error) => {
            state.metrics.record(endpoint, error.outcome(), seconds);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AreaItemsRequest, MusicRecommendRequest, resolve_event_type};
    use allium_deck::power::MultiUnitBonusMode;
    use allium_deck::types::EventType;
    use serde_json::json;

    #[test]
    fn area_item_evaluation_defaults_to_by_deck_and_accepts_both_spellings() {
        let body = json!({"user": {}, "cardIds": [1, 2, 3, 4, 5]});
        let default: AreaItemsRequest =
            serde_json::from_value(body.clone()).expect("default area item request parses");
        assert_eq!(
            default.multi_unit_bonus_evaluation,
            MultiUnitBonusMode::ByDeck
        );

        for key in ["multiUnitBonusEvaluation", "multi_unit_bonus_evaluation"] {
            for (value, expected) in [
                ("by_deck", MultiUnitBonusMode::ByDeck),
                ("force_on", MultiUnitBonusMode::ForceOn),
                ("force_off", MultiUnitBonusMode::ForceOff),
            ] {
                let mut request = body.clone();
                request[key] = json!(value);
                let parsed: AreaItemsRequest =
                    serde_json::from_value(request).expect("evaluation mode parses");
                assert_eq!(parsed.multi_unit_bonus_evaluation, expected);
            }
            for value in [json!("invalid"), json!(true), json!(1), json!(null)] {
                let mut request = body.clone();
                request[key] = value;
                assert!(serde_json::from_value::<AreaItemsRequest>(request).is_err());
            }
        }
    }

    #[test]
    fn music_recommend_event_type_handles_memory_event_unit_and_synthetic_ids() {
        assert_eq!(
            resolve_event_type(None, None, Some("light_sound"), &[])
                .expect("light_sound resolves to marathon"),
            EventType::Marathon
        );
        assert_eq!(
            resolve_event_type(Some("marathon"), None, Some("piapro"), &[])
                .expect("marathon + piapro resolves to marathon"),
            EventType::Marathon
        );
        for id in 4_000_001..=4_000_006 {
            assert_eq!(
                resolve_event_type(None, Some(id), None, &[])
                    .expect("synthetic memory event id resolves to marathon"),
                EventType::Marathon
            );
        }
        assert!(resolve_event_type(None, None, Some("invalid"), &[]).is_err());
        assert!(resolve_event_type(Some("cheerful_carnival"), None, Some("street"), &[]).is_err());
        assert!(resolve_event_type(None, Some(1), Some("street"), &[]).is_err());
        assert!(resolve_event_type(None, Some(4_000_007), None, &[]).is_err());

        let parsed: MusicRecommendRequest = serde_json::from_value(json!({
            "deck": { "totalPower": 100000, "cards": [] },
            "liveType": "solo",
            "eventMeUnit": "idol"
        }))
        .expect("music recommend request with eventMeUnit parses");
        assert_eq!(parsed.event_me_unit.as_deref(), Some("idol"));
    }
}
