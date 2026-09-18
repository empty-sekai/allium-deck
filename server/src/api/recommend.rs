//! Deck building endpoints.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use allium_deck::LiveType;
use allium_deck::handler::{BuildError, PreparedGameData, UserProfile, build_card_pool_prepared};
use allium_deck::pool::CardPool;
use allium_deck::search::{
    DeckResult, SearchCompletion, SearchContext, SearchParams, SearchStats, challenge_search,
    compare_deck_results, search_targets, summarize_deck,
};

use super::{ApiError, ApiJson, AppState, UserInput, parse_params};
use crate::metrics::Outcome;
use crate::state::RegionSnapshot;

/// Music used for challenge scoring when a request does not name one.
const DEFAULT_CHALLENGE_MUSIC_ID: i32 = 104;
const DEFAULT_CHALLENGE_MUSIC_DIFF: &str = "master";
/// Game character ids, as used by the challenge-all sweep.
const CHARACTER_IDS: std::ops::RangeInclusive<i32> = 1..=26;

#[derive(Debug, Deserialize)]
pub struct RecommendRequest {
    /// Region to serve from. Defaults to the service's default region.
    pub region: Option<String>,
    /// The player's collection, as an object or as its JSON text.
    pub user: UserInput,
    /// Build parameters; see `docs/parameters.md`.
    pub params: Option<Box<RawValue>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardOut {
    pub card_id: u16,
    pub power_total: Option<i32>,
    pub event_bonus: Option<f64>,
    pub skill_score_up: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckOut {
    pub rank: usize,
    /// The search objective's value for this deck. Comparable only within one response.
    pub target_value: u64,
    pub cards: Vec<CardOut>,
    pub total_power: Option<i32>,
    pub live_score: Option<i32>,
    pub event_point: Option<i32>,
    pub multi_live_score_up: Option<f64>,
    pub event_bonus_total: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub pool_size: usize,
    pub visited_nodes: u64,
    pub bound_prunes: u64,
    pub feasibility_prunes: u64,
    pub dominance_prunes: u64,
    pub deadline_hit: bool,
    pub phases: allium_deck::search::SearchDiagnostics,
    pub effective_live_type: &'static str,
    pub leaf_nodes: u64,
    pub ub_prunes: u64,
    pub leader_prunes: u64,
    pub ep_explored: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub queue_wait_ms: f64,
    pub build_pool_ms: f64,
    pub search_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendResponse {
    pub region: String,
    pub decks: Vec<DeckOut>,
    pub diagnostics: Diagnostics,
    pub timing: Timing,
    /// True when the search hit its deadline. The decks are then the best found so far
    /// rather than a proven optimum; see the exactness matrix in `docs/parameters.md`.
    pub completion: SearchCompletion,
    pub timed_out: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeCharacterOut {
    pub rank: Option<usize>,
    pub character_id: i32,
    pub candidate_count: usize,
    pub completion: SearchCompletion,
    pub search_ms: f64,
    pub deck: Option<DeckOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeAllResponse {
    pub region: String,
    pub characters: Vec<ChallengeCharacterOut>,
    pub diagnostics: Diagnostics,
    pub timing: Timing,
    pub completion: SearchCompletion,
    pub timed_out: bool,
}

/// `POST /v1/recommend`
pub async fn recommend(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<RecommendRequest>,
) -> Result<Json<RecommendResponse>, ApiError> {
    let started = Instant::now();
    let outcome = run_recommend(&state, request).await;
    finish(&state, "recommend", started, outcome)
}

async fn run_recommend(
    state: &Arc<AppState>,
    request: RecommendRequest,
) -> Result<(RecommendResponse, f64, f64), ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let user = request.user.parse()?;
    let mut params = parse_params(request.params.as_deref())?;
    state.clamp(&mut params);

    let region = snapshot.name.clone();
    let job = state
        .pool
        .execute(move || search_one(&snapshot, &user, params))
        .await?;

    let (decks, diagnostics, build_pool_ms, search_ms, completion) = job.value?;
    let queue_wait_ms = job.queue_wait.as_secs_f64() * 1000.0;
    Ok((
        RecommendResponse {
            region,
            decks,
            diagnostics,
            timing: Timing {
                queue_wait_ms,
                build_pool_ms,
                search_ms,
                total_ms: 0.0,
            },
            completion,
            timed_out: completion == SearchCompletion::TimedOut,
        },
        build_pool_ms,
        search_ms,
    ))
}

type SearchOutput = (Vec<DeckOut>, Diagnostics, f64, f64, SearchCompletion);

/// Builds the pool and searches it. Runs on a search thread, never on the runtime.
fn search_one(
    snapshot: &RegionSnapshot,
    user: &UserProfile,
    params: allium_deck::handler::BuildParams,
) -> Result<SearchOutput, ApiError> {
    let game = snapshot.game();
    let prepared = PreparedGameData::with_indexes(game, snapshot.indexes());

    let build_started = Instant::now();
    let built = build_card_pool_prepared(user, &prepared, &params);
    let build_pool_ms = elapsed_ms(build_started);

    let (pool, ctx) = match built {
        Ok(built) => built,
        // Matches `engine::recommend`: for tiered bonus search an empty pool means
        // every requested tier is unreachable, which is an empty answer rather than
        // an error.
        Err(BuildError::EmptyPool) if !params.target_bonus_list.is_empty() => {
            return Ok((
                Vec::new(),
                diagnostics_for(0, params.live_type, &SearchStats::default()),
                build_pool_ms,
                0.0,
                SearchCompletion::Complete,
            ));
        }
        Err(error) => return Err(ApiError::BadRequest(error.to_string())),
    };

    let search_params = SearchParams {
        top_k: params.limit,
        timeout_ms: params.timeout_ms,
    };
    let search_started = Instant::now();
    let outcome = search_targets(&pool, &ctx, &search_params, &params.target_bonus_list);
    let completion = outcome.completion();
    let results = outcome.results;
    let stats = outcome.stats;
    let search_ms = elapsed_ms(search_started);

    let decks = results
        .iter()
        .enumerate()
        .map(|(index, result)| deck_out(index + 1, &pool, &ctx, result))
        .collect();
    Ok((
        decks,
        diagnostics(&pool, &ctx, &stats),
        build_pool_ms,
        search_ms,
        completion,
    ))
}

/// `POST /v1/recommend/challenge-all`
pub async fn challenge_all(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<RecommendRequest>,
) -> Result<Json<ChallengeAllResponse>, ApiError> {
    let started = Instant::now();
    let outcome = run_challenge_all(&state, request).await;
    finish(&state, "challenge_all", started, outcome)
}

async fn run_challenge_all(
    state: &Arc<AppState>,
    request: RecommendRequest,
) -> Result<(ChallengeAllResponse, f64, f64), ApiError> {
    let snapshot = state.snapshot(request.region.as_deref())?;
    let user = request.user.parse()?;
    let mut params = parse_params(request.params.as_deref())?;
    state.clamp(&mut params);

    // A challenge deck is five cards of one character, so the sweep is per character.
    if !matches!(
        params.live_type,
        LiveType::Challenge | LiveType::ChallengeAuto
    ) {
        params.live_type = LiveType::Challenge;
    }
    // The shared pool keeps every character; the search filters per character, so the
    // sweep builds one pool instead of twenty-six.
    params.challenge_live_character_id = None;
    if params.music_id.is_none() {
        params.music_id = Some(DEFAULT_CHALLENGE_MUSIC_ID);
    }
    if params.music_diff.is_none() {
        params.music_diff = Some(DEFAULT_CHALLENGE_MUSIC_DIFF.to_string());
    }

    let region = snapshot.name.clone();
    let job = state
        .pool
        .execute(move || sweep_characters(&snapshot, &user, params))
        .await?;

    let (characters, diagnostics, build_pool_ms, search_ms, completion) = job.value?;
    let queue_wait_ms = job.queue_wait.as_secs_f64() * 1000.0;
    Ok((
        ChallengeAllResponse {
            region,
            characters,
            diagnostics,
            timing: Timing {
                queue_wait_ms,
                build_pool_ms,
                search_ms,
                total_ms: 0.0,
            },
            completion,
            timed_out: completion == SearchCompletion::TimedOut,
        },
        build_pool_ms,
        search_ms,
    ))
}

type SweepOutput = (
    Vec<ChallengeCharacterOut>,
    Diagnostics,
    f64,
    f64,
    SearchCompletion,
);

fn sweep_characters(
    snapshot: &RegionSnapshot,
    user: &UserProfile,
    params: allium_deck::handler::BuildParams,
) -> Result<SweepOutput, ApiError> {
    let game = snapshot.game();
    let prepared = PreparedGameData::with_indexes(game, snapshot.indexes());

    let build_started = Instant::now();
    let (pool, ctx) = build_card_pool_prepared(user, &prepared, &params)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let build_pool_ms = elapsed_ms(build_started);

    // One deck per character: the ranking across characters is what the sweep is for.
    let search_params = SearchParams {
        top_k: 1,
        timeout_ms: params.timeout_ms,
    };

    let search_started = Instant::now();
    let ids = CHARACTER_IDS.map(|id| id as u8).collect::<Vec<_>>();
    let batch = challenge_search::search_characters_outcome(&pool, &ctx, &search_params, &ids);
    let search_ms = elapsed_ms(search_started);
    let completion = batch.completion();
    let mut characters = batch
        .results
        .iter()
        .map(|entry| {
            let candidate_count = pool
                .indices()
                .filter(|&card| pool.char_id(card) == entry.character_id)
                .count();
            ChallengeCharacterOut {
                rank: None,
                character_id: i32::from(entry.character_id),
                candidate_count,
                completion: entry.outcome.completion(),
                search_ms: entry.search_time.as_secs_f64() * 1000.0,
                deck: entry
                    .outcome
                    .results
                    .first()
                    .map(|result| deck_out(1, &pool, &ctx, result)),
            }
        })
        .collect::<Vec<_>>();

    // Rank exact public-card-set witnesses, rather than duplicating a score-only
    // ordering in the HTTP layer. Partial rankings remain explicitly timed out.
    let mut order = batch
        .results
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.outcome.results.first().map(|deck| (index, deck)))
        .collect::<Vec<_>>();
    order.sort_by(|left, right| compare_deck_results(&pool, &ctx, left.1, right.1));
    for (rank, (index, _)) in order.into_iter().enumerate() {
        characters[index].rank = Some(rank + 1);
    }

    Ok((
        characters,
        diagnostics(&pool, &ctx, &batch.stats),
        build_pool_ms,
        search_ms,
        completion,
    ))
}

/// Shapes one search result into the response, including the display panel.
fn deck_out(rank: usize, pool: &CardPool, ctx: &SearchContext, result: &DeckResult) -> DeckOut {
    let summary = summarize_deck(pool, ctx, &result.cards);
    let ordered = summary.map_or(result.cards, |summary| summary.ordered_cards);
    let cards = (0..5)
        .map(|position| {
            let card = ordered[position];
            CardOut {
                card_id: pool.game_id(card),
                power_total: summary.map(|summary| summary.card_power_total[position]),
                event_bonus: summary.map(|summary| summary.card_event_bonus_rates[position]),
                skill_score_up: summary.map(|summary| summary.card_skill_score_up[position]),
            }
        })
        .collect();

    DeckOut {
        rank,
        target_value: result.score,
        cards,
        total_power: summary.map(|summary| summary.total_power),
        live_score: summary.map(|summary| summary.live_score),
        event_point: summary.and_then(|summary| summary.event_point),
        multi_live_score_up: summary.map(|summary| summary.multi_live_score_up),
        event_bonus_total: summary.and_then(|summary| summary.event_bonus_total),
    }
}

fn diagnostics(pool: &CardPool, ctx: &SearchContext, stats: &SearchStats) -> Diagnostics {
    diagnostics_for(pool.count(), ctx.effective_live_type(), stats)
}

fn diagnostics_for(pool_size: usize, live_type: LiveType, stats: &SearchStats) -> Diagnostics {
    Diagnostics {
        pool_size,
        effective_live_type: live_type_name(live_type),
        visited_nodes: stats.visited_nodes,
        bound_prunes: stats.bound_prunes,
        feasibility_prunes: stats.feasibility_prunes,
        dominance_prunes: stats.dominance_prunes,
        deadline_hit: stats.deadline_hit,
        phases: stats.diagnostics.clone(),
        leaf_nodes: stats.leaf_nodes,
        ub_prunes: stats.ub_prunes,
        leader_prunes: stats.leader_prunes,
        ep_explored: stats.ep_explored,
    }
}

/// The same spelling the request contract uses, so responses round-trip into requests.
fn live_type_name(live_type: LiveType) -> &'static str {
    match live_type {
        LiveType::Solo => "solo",
        LiveType::Auto => "auto",
        LiveType::Multi => "multi",
        LiveType::Cheerful => "cheerful",
        LiveType::Challenge => "challenge",
        LiveType::ChallengeAuto => "challenge_auto",
        LiveType::Mysekai => "mysekai",
    }
}

fn elapsed_ms(from: Instant) -> f64 {
    from.elapsed().as_secs_f64() * 1000.0
}

/// Records metrics and stamps the total wall time onto the response.
fn finish<T: HasTiming>(
    state: &Arc<AppState>,
    endpoint: &str,
    started: Instant,
    outcome: Result<(T, f64, f64), ApiError>,
) -> Result<Json<T>, ApiError> {
    let total_ms = elapsed_ms(started);
    match outcome {
        Ok((mut response, build_pool_ms, search_ms)) => {
            let timing = response.timing_mut();
            timing.total_ms = total_ms;
            let queue_wait_ms = timing.queue_wait_ms;
            state
                .metrics
                .record(endpoint, Outcome::Ok, total_ms / 1000.0);
            state.metrics.record_stages(
                endpoint,
                queue_wait_ms / 1000.0,
                build_pool_ms / 1000.0,
                search_ms / 1000.0,
            );
            if response.timed_out() {
                state.metrics.record_search_timeout();
            }
            Ok(Json(response))
        }
        Err(error) => {
            state
                .metrics
                .record(endpoint, error.outcome(), total_ms / 1000.0);
            Err(error)
        }
    }
}

/// Lets `finish` stamp the total time onto either response shape.
pub trait HasTiming {
    fn timing_mut(&mut self) -> &mut Timing;
    fn timed_out(&self) -> bool;
}

impl HasTiming for RecommendResponse {
    fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }
    fn timed_out(&self) -> bool {
        self.timed_out
    }
}

impl HasTiming for ChallengeAllResponse {
    fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }
    fn timed_out(&self) -> bool {
        self.timed_out
    }
}
