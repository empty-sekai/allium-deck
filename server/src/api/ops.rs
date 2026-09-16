//! Operational endpoints: health, readiness, metrics, region inventory, reload.

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use super::{ApiError, AppState};
use crate::state::{Registry, TableCounts};

/// The OpenAPI description is served verbatim so it can be diffed like any other file.
const OPENAPI: &str = include_str!("../../openapi.json");

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionOut {
    pub name: String,
    pub is_default: bool,
    pub counts: TableCounts,
    /// Unix seconds at which this snapshot was loaded.
    pub loaded_at: u64,
    pub load_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionsResponse {
    pub default_region: String,
    pub regions: Vec<RegionOut>,
    pub workers: usize,
    pub max_queue: usize,
    pub max_search_timeout_ms: u64,
    pub max_limit: usize,
}

/// `GET /v1/regions`
pub async fn regions(State(state): State<Arc<AppState>>) -> Json<RegionsResponse> {
    let registry = state.registry.current();
    Json(RegionsResponse {
        default_region: registry.default_region().to_string(),
        regions: describe(&registry),
        workers: state.pool.workers_configured,
        max_queue: state.pool.max_queue,
        max_search_timeout_ms: state.config.max_search_timeout_ms,
        max_limit: state.config.max_limit,
    })
}

fn describe(registry: &Registry) -> Vec<RegionOut> {
    registry
        .snapshots()
        .map(|snapshot| RegionOut {
            name: snapshot.name.clone(),
            is_default: snapshot.name == registry.default_region(),
            counts: snapshot.counts,
            loaded_at: snapshot
                .loaded_at
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0),
            load_ms: snapshot.load_ms,
        })
        .collect()
}

/// `GET /healthz` — the process is up.
pub async fn healthz() -> &'static str {
    "ok"
}

/// `GET /readyz` — masterdata is loaded and the service can answer requests.
pub async fn readyz(State(state): State<Arc<AppState>>) -> Response {
    let registry = state.registry.current();
    if registry.names().next().is_some() {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "no masterdata loaded").into_response()
    }
}

/// `GET /metrics` — Prometheus text exposition.
pub async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}

/// `GET /openapi.json`
pub async fn openapi() -> Response {
    ([(header::CONTENT_TYPE, "application/json")], OPENAPI).into_response()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadResponse {
    pub reloaded: Vec<RegionOut>,
}

/// `POST /admin/reload` — re-reads every configured region from disk.
///
/// The new snapshot replaces the old one atomically; requests already in flight keep
/// the snapshot they started with. A failed reload leaves the running snapshot in place.
pub async fn reload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ReloadResponse>, ApiError> {
    let Some(expected) = state.config.admin_token.as_deref() else {
        return Err(ApiError::BadRequest(
            "reload is disabled; start the service with --admin-token to enable it".to_string(),
        ));
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if presented != Some(expected) {
        return Err(ApiError::BadRequest("invalid admin token".to_string()));
    }

    let sources = state.config.regions.clone();
    let default_region = state.config.default_region.clone();
    // Loading parses every masterdata table, so it runs off the runtime — but on the
    // blocking pool rather than the search threads, which stay free for requests.
    let loaded = tokio::task::spawn_blocking(move || Registry::load(&sources, &default_region))
        .await
        .map_err(|error| ApiError::Internal(format!("reload task failed: {error}")))?
        .map_err(ApiError::BadRequest)?;

    let described = describe(&loaded);
    state.registry.replace(loaded);
    state.metrics.record_reload();
    tracing::info!(regions = described.len(), "masterdata reloaded");
    Ok(Json(ReloadResponse {
        reloaded: described,
    }))
}
