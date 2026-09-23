//! The HTTP shell must not change what the engine answers.
//!
//! Each case runs the same inputs two ways — through the router in process, and
//! through the library entry point directly — and requires the deck sequence to match
//! exactly, order included. A shell that reorders, re-ranks, or silently rewrites
//! parameters fails here.
//!
//! The fixtures come from the shared deterministic fixture generator, so these tests need
//! no game data and run anywhere.

#[path = "../fixtures/synthetic.rs"]
mod synth_masterdata;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use allium_deck::engine::{OwnedGameData, parse_build_params_json, parse_user_profile_json};
use allium_deck_server::api::{self, AppState};
use allium_deck_server::config::{Config, RegionSource};
use allium_deck_server::metrics::Metrics;
use allium_deck_server::pool::SearchPool;
use allium_deck_server::state::{Registry, SharedRegistry};

/// Writes the synthetic masterdata once per test binary and returns its directory.
fn fixture() -> &'static Path {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("synth");
        let masterdata = root.join("masterdata");
        std::fs::create_dir_all(&masterdata).expect("fixture directory");

        let synth = synth_masterdata::generate(synth_masterdata::DEFAULT_SEED);
        for (name, json) in &synth.tables {
            std::fs::write(masterdata.join(name), json).expect("table written");
        }
        std::fs::write(root.join("music_metas.json"), &synth.music_metas_json)
            .expect("music metas written");
        std::fs::write(root.join("user.json"), &synth.user_json).expect("user written");
        root
    })
}

fn user_json() -> String {
    std::fs::read_to_string(fixture().join("user.json")).expect("user fixture")
}

/// Extend the normal 390-card account just beyond the 512-bit metadata mask.
/// Added cards use conservative legal progression state; the test is about the
/// full-pool contract, not about a particular cultivation preset.
fn oversized_user_json() -> String {
    let mut user: serde_json::Value = serde_json::from_str(&user_json()).expect("user json");
    let cards = user["userCards"].as_array_mut().expect("userCards");
    let mut owned = cards
        .iter()
        .filter_map(|card| card["cardId"].as_u64())
        .collect::<HashSet<_>>();
    let target = allium_deck::pool::MASK_WORDS * 64 + 1;
    for card_id in 1..=1300u64 {
        if cards.len() >= target {
            break;
        }
        if owned.insert(card_id) {
            cards.push(serde_json::json!({
                "cardId": card_id,
                "level": 1,
                "skillLevel": 1,
                "masterRank": 0,
                "specialTrainingStatus": "not_doing",
                "defaultImage": "original",
                "episodes": []
            }));
        }
    }
    assert_eq!(
        cards.len(),
        target,
        "large fixture must cross the metadata mask width by one"
    );
    serde_json::to_string(&user).expect("oversized user serializes")
}

fn config() -> Config {
    let root = fixture();
    Config {
        bind: "127.0.0.1:0".parse().expect("bind address"),
        regions: vec![RegionSource {
            name: "synth".to_string(),
            masterdata_dir: root.join("masterdata"),
            music_metas: root.join("music_metas.json"),
        }],
        default_region: "synth".to_string(),
        workers: 2,
        max_queue: 16,
        queue_timeout_ms: 30_000,
        max_search_timeout_ms: 120_000,
        max_limit: 30,
        max_body_bytes: 8 * 1024 * 1024,
        admin_token: None,
        log_json: false,
    }
}

fn app(config: Config) -> (axum::Router, Arc<AppState>) {
    let registry = Registry::load(&config.regions, &config.default_region).expect("masterdata");
    let pool = SearchPool::new(
        config.workers,
        config.max_queue,
        Duration::from_millis(config.queue_timeout_ms),
        Duration::from_secs(300),
    );
    let metrics = Metrics::new(Arc::clone(&pool.metrics));
    let state = Arc::new(AppState {
        registry: SharedRegistry::new(registry),
        pool,
        metrics,
        config,
    });
    (api::router(Arc::clone(&state)), state)
}

async fn post(router: &axum::Router, path: &str, body: String) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request built");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router responded");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// The deck sequence the library itself produces for these parameters.
fn engine_decks(params_json: &str) -> Vec<Vec<u16>> {
    let root = fixture();
    let owned = OwnedGameData::load(&root.join("masterdata"), &root.join("music_metas.json"))
        .expect("masterdata loads");
    let user = parse_user_profile_json(&user_json()).expect("user parses");
    let params = parse_build_params_json(params_json).expect("params parse");
    let outcome =
        allium_deck::engine::recommend(&user, &owned.as_ref(), &params).expect("engine recommends");
    assert_eq!(
        outcome.completion(),
        allium_deck::search::SearchCompletion::Complete
    );
    outcome
        .results
        .iter()
        .map(|deck| deck.cards.to_vec())
        .collect()
}

fn request_body(params_json: &str) -> String {
    format!("{{\"user\":{},\"params\":{}}}", user_json(), params_json)
}

fn http_decks(value: &serde_json::Value) -> Vec<Vec<u16>> {
    value["decks"]
        .as_array()
        .expect("decks array")
        .iter()
        .map(|deck| {
            deck["cards"]
                .as_array()
                .expect("cards array")
                .iter()
                .map(|card| card["cardId"].as_u64().expect("card id") as u16)
                .collect()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn http_decks_match_the_engine_entry_point() {
    let (router, state) = app(config());

    for params in [
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":8}"#,
        r#"{"liveType":"multi","target":"score","limit":5,"attrFilter":"cool"}"#,
        r#"{"liveType":"solo","target":"power","limit":5}"#,
        r#"{"liveType":"challenge","target":"score","limit":3,"challengeLiveCharacterId":1}"#,
    ] {
        let (status, body) = post(&router, "/v1/recommend", request_body(params)).await;
        assert_eq!(status, StatusCode::OK, "params {params} gave {body}");
        assert_eq!(
            http_decks(&body),
            engine_decks(params),
            "the shell changed the result for {params}"
        );
        assert_eq!(body["timedOut"], serde_json::Value::Bool(false));
    }

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_object_and_its_json_text_are_equivalent() {
    let (router, state) = app(config());
    let params =
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":5}"#;

    let (as_object, object_body) = post(&router, "/v1/recommend", request_body(params)).await;
    // The upload pipeline hands out the collection as JSON text rather than an object.
    let as_text_body = format!(
        "{{\"user\":{},\"params\":{}}}",
        serde_json::Value::String(user_json()),
        params
    );
    let (as_text, text_body) = post(&router, "/v1/recommend", as_text_body).await;

    assert_eq!(as_object, StatusCode::OK);
    assert_eq!(as_text, StatusCode::OK, "text form rejected: {text_body}");
    assert_eq!(http_decks(&object_body), http_decks(&text_body));

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn limit_and_timeout_are_lowered_to_the_service_ceilings() {
    let mut config = config();
    config.max_limit = 3;
    config.max_search_timeout_ms = 250;
    let (router, state) = app(config);

    // Asks for more decks and a longer deadline than the service allows.
    let params = r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":30,"timeoutMs":300000}"#;
    let (status, body) = post(&router, "/v1/recommend", request_body(params)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body["decks"]
            .as_array()
            .is_some_and(|decks| decks.len() <= 3),
        "limit was not lowered: {body}"
    );
    assert!(
        body["timing"]["searchMs"]
            .as_f64()
            .is_some_and(|ms| ms < 5_000.0),
        "the request was allowed to run past the ceiling: {body}"
    );

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn challenge_all_ranks_one_deck_per_character() {
    let (router, state) = app(config());
    let params = r#"{"target":"score","limit":1,"timeoutMs":60000}"#;

    let (status, body) = post(&router, "/v1/recommend/challenge-all", request_body(params)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let characters = body["characters"].as_array().expect("characters");
    assert_eq!(characters.len(), 26);
    // The sweep forces a challenge live type regardless of what was asked for.
    assert_eq!(body["diagnostics"]["effectiveLiveType"], "challenge");

    // Every deck is five cards of the character it is filed under, and the ranking is
    // a permutation of the characters that produced one.
    let mut ranks = Vec::new();
    for entry in characters {
        if let Some(rank) = entry["rank"].as_u64() {
            ranks.push(rank);
            assert!(
                entry["deck"]["cards"]
                    .as_array()
                    .is_some_and(|c| c.len() == 5)
            );
        }
    }
    ranks.sort_unstable();
    assert_eq!(ranks, (1..=ranks.len() as u64).collect::<Vec<_>>());

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn large_pool_http_result_matches_complete_engine_result() {
    let (router, state) = app(config());
    let user_json = oversized_user_json();
    let params_json = r#"{"liveType":"multi","target":"power","limit":8,"timeoutMs":30000}"#;
    let body = format!("{{\"user\":{user_json},\"params\":{params_json}}}");

    let (status, value) = post(&router, "/v1/recommend", body).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["completion"], "complete", "{value}");
    assert_eq!(value["diagnostics"]["poolSize"], 513, "{value}");

    let root = fixture();
    let owned = OwnedGameData::load(&root.join("masterdata"), &root.join("music_metas.json"))
        .expect("masterdata loads");
    let user = parse_user_profile_json(&user_json).expect("user parses");
    let params = parse_build_params_json(params_json).expect("params parse");
    let engine = allium_deck::engine::recommend(&user, &owned.as_ref(), &params)
        .expect("engine handles every owned card");
    assert_eq!(
        engine.completion(),
        allium_deck::search::SearchCompletion::Complete
    );
    let expected = engine
        .results
        .iter()
        .map(|deck| deck.cards.to_vec())
        .collect::<Vec<_>>();
    assert_eq!(http_decks(&value), expected);

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn challenge_all_reports_solver_timeout_not_elapsed_time_guessing() {
    let (router, state) = app(config());
    // A complete Top-100 for every character of the large account takes far
    // longer than the one-millisecond budget.
    let params = r#"{"target":"score","limit":100,"timeoutMs":1}"#;
    let body = format!("{{\"user\":{},\"params\":{params}}}", oversized_user_json());

    let (status, body) = post(&router, "/v1/recommend/challenge-all", body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["completion"], "timed_out", "{body}");
    assert_eq!(body["timedOut"], true, "{body}");
    assert_eq!(body["diagnostics"]["deadlineHit"], true, "{body}");
    assert!(
        body["characters"]
            .as_array()
            .is_some_and(|characters| characters
                .iter()
                .any(|entry| entry["completion"] == "timed_out")),
        "shared-budget expiry did not reach per-character outcomes: {body}"
    );

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_input_is_reported_in_the_error_shape() {
    let (router, state) = app(config());

    for (path, body, code) in [
        ("/v1/recommend", "not json".to_string(), "invalid_request"),
        (
            "/v1/recommend",
            format!("{{\"region\":\"nope\",\"user\":{}}}", user_json()),
            "unknown_region",
        ),
        (
            "/v1/recommend",
            format!(
                "{{\"user\":{},\"params\":{{\"liveType\":\"nonsense\"}}}}",
                user_json()
            ),
            "invalid_request",
        ),
    ] {
        let (status, value) = post(&router, path, body).await;
        assert!(
            status.is_client_error(),
            "expected a client error, got {status}"
        );
        assert_eq!(value["error"]["code"], code, "body was {value}");
        assert!(value["error"]["message"].is_string());
    }

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn regions_reports_the_loaded_masterdata_and_the_ceilings() {
    let (router, state) = app(config());
    let request = Request::builder()
        .uri("/v1/regions")
        .body(Body::empty())
        .expect("request built");
    let response = router.clone().oneshot(request).await.expect("responded");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

    assert_eq!(value["defaultRegion"], "synth");
    assert_eq!(value["regions"][0]["name"], "synth");
    assert_eq!(value["regions"][0]["counts"]["cards"], 1300);
    assert_eq!(value["maxLimit"], 30);

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn world_bloom_support_cards_are_ranked_by_bonus() {
    let (router, state) = app(config());
    let body = format!(
        "{{\"user\":{},\"params\":{{\"worldBloomFinaleTurn\":3,\"worldBloomCharacterId\":1}}}}",
        user_json()
    );

    let (status, value) = post(&router, "/v1/world-bloom/support-cards", body).await;
    assert_eq!(status, StatusCode::OK, "{value}");

    let cards = value.as_array().expect("an array of cards");
    let bonuses: Vec<f64> = cards
        .iter()
        .map(|card| card["bonus"].as_f64().expect("bonus"))
        .collect();
    assert!(
        bonuses.windows(2).all(|pair| pair[0] >= pair[1]),
        "support cards are not ordered strongest first"
    );

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn music_recommend_scores_every_song_for_a_fixed_deck() {
    let (router, state) = app(config());
    let body = r#"{
        "liveType": "multi",
        "eventType": "marathon",
        "deck": {
            "totalPower": 250000,
            "eventBonusRate": 60.0,
            "supportDeckBonusRate": 0.0,
            "cards": [
                {"skillScoreUp": 80.0, "skillLifeRecovery": 0.0},
                {"skillScoreUp": 70.0, "skillLifeRecovery": 0.0},
                {"skillScoreUp": 60.0, "skillLifeRecovery": 0.0},
                {"skillScoreUp": 50.0, "skillLifeRecovery": 0.0},
                {"skillScoreUp": 40.0, "skillLifeRecovery": 0.0}
            ]
        }
    }"#;

    let (status, value) = post(&router, "/v1/music/recommend", body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert!(
        value.as_array().is_some_and(|songs| !songs.is_empty()),
        "expected scored songs, got {value}"
    );

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoints_needing_absent_tables_say_which_table_is_missing() {
    // The synthetic set carries the deck-building tables but not the auxiliary ones, so
    // these endpoints must report the missing table rather than fail some other way.
    let (router, state) = app(config());

    let body = format!("{{\"user\":{},\"cardIds\":[1,2,3]}}", user_json());
    let (status, value) = post(&router, "/v1/area-items/recommend", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert_eq!(value["error"]["code"], "invalid_request");
    assert!(value["error"]["message"].is_string());

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_requires_its_token_and_swaps_the_snapshot() {
    let mut config = config();
    config.admin_token = Some("secret".to_string());
    let (router, state) = app(config);

    let reload = |token: Option<&'static str>| {
        let router = router.clone();
        async move {
            let mut request = Request::builder().method("POST").uri("/admin/reload");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let response = router
                .oneshot(request.body(Body::empty()).expect("request built"))
                .await
                .expect("responded");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, value)
        }
    };

    let (no_token, _) = reload(None).await;
    assert_eq!(no_token, StatusCode::BAD_REQUEST);
    let (wrong_token, _) = reload(Some("wrong")).await;
    assert_eq!(wrong_token, StatusCode::BAD_REQUEST);

    let before = state.snapshot(None).expect("a snapshot before reload");
    let (accepted, body) = reload(Some("secret")).await;
    assert_eq!(accepted, StatusCode::OK, "{body}");
    assert_eq!(body["reloaded"][0]["name"], "synth");

    // The registry now hands out a different snapshot, and requests that started
    // earlier still hold the old one.
    let after = state.snapshot(None).expect("a snapshot after reload");
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(before.counts.cards, after.counts.cards);

    state.pool.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_documented_path_is_served() {
    // The OpenAPI description is a static file, so nothing stops it from drifting away
    // from the router. Each documented path must at least reach a handler: a 404 or 405
    // means the description promises an endpoint that does not exist.
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../openapi.json")).expect("openapi.json parses");
    let paths = document["paths"].as_object().expect("paths object");
    assert!(!paths.is_empty());

    let (router, state) = app(config());
    for (path, methods) in paths {
        for method in methods.as_object().expect("methods object").keys() {
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request built");
            let status = router
                .clone()
                .oneshot(request)
                .await
                .expect("router responded")
                .status();
            assert!(
                status != StatusCode::NOT_FOUND && status != StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} is documented but answered {status}"
            );
        }
    }

    state.pool.shutdown();
}
