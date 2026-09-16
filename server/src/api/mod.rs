//! HTTP surface: shared state, error mapping, and the router.

pub mod auxiliary;
pub mod ops;
pub mod recommend;

use std::sync::Arc;

use axum::Json;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::value::RawValue;

use allium_deck::engine::{parse_build_params_json, parse_user_profile_json};
use allium_deck::handler::{BuildParams, UserProfile};

use crate::config::Config;
use crate::metrics::{Metrics, Outcome};
use crate::pool::{Rejection, SearchPool};
use crate::state::{RegionSnapshot, SharedRegistry};

/// Everything a handler needs, shared across requests.
pub struct AppState {
    pub registry: SharedRegistry,
    pub pool: SearchPool,
    pub metrics: Metrics,
    pub config: Config,
}

impl AppState {
    /// Resolves the region named in a request, or the configured default.
    pub fn snapshot(&self, region: Option<&str>) -> Result<Arc<RegionSnapshot>, ApiError> {
        let registry = self.registry.current();
        registry.get(region).ok_or_else(|| {
            let known = registry.names().collect::<Vec<_>>().join(", ");
            let asked = region.unwrap_or(registry.default_region());
            ApiError::UnknownRegion(format!("unknown region {asked}; loaded regions: {known}"))
        })
    }

    /// Applies the service-side ceilings to parsed build parameters.
    ///
    /// The engine accepts `limit` up to 100 and `timeoutMs` up to 300000, which would
    /// let one request occupy a search thread for five minutes. The service lowers both
    /// to its configured ceilings instead of rejecting the request, so a caller that
    /// asks for more gets a smaller answer rather than an error.
    pub fn clamp(&self, params: &mut BuildParams) {
        params.limit = params.limit.min(self.config.max_limit);
        let ceiling = self.config.max_search_timeout_ms;
        params.timeout_ms = if params.timeout_ms == 0 {
            ceiling
        } else {
            params.timeout_ms.min(ceiling)
        };
    }
}

/// Errors that map onto a status code and a stable error body.
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    UnknownRegion(String),
    Overloaded,
    Timeout,
    Internal(String),
}

impl ApiError {
    pub fn outcome(&self) -> Outcome {
        match self {
            ApiError::BadRequest(_) => Outcome::BadRequest,
            ApiError::UnknownRegion(_) => Outcome::BadRequest,
            ApiError::Overloaded => Outcome::Overloaded,
            ApiError::Timeout => Outcome::Timeout,
            ApiError::Internal(_) => Outcome::Error,
        }
    }

    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            ApiError::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "invalid_request", message.clone())
            }
            ApiError::UnknownRegion(message) => {
                (StatusCode::NOT_FOUND, "unknown_region", message.clone())
            }
            ApiError::Overloaded => (
                StatusCode::SERVICE_UNAVAILABLE,
                "overloaded",
                "the search queue is full; retry shortly".to_string(),
            ),
            ApiError::Timeout => (
                StatusCode::GATEWAY_TIMEOUT,
                "queue_timeout",
                "the request waited longer than the configured budget".to_string(),
            ),
            ApiError::Internal(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                message.clone(),
            ),
        }
    }
}

impl From<Rejection> for ApiError {
    fn from(rejection: Rejection) -> Self {
        match rejection {
            Rejection::QueueFull => ApiError::Overloaded,
            Rejection::QueueTimeout => ApiError::Timeout,
            Rejection::Failed => ApiError::Internal("the search worker failed".to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        let body = serde_json::json!({ "error": { "code": code, "message": message } });
        let mut response = (status, Json(body)).into_response();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static("1"),
            );
        }
        response
    }
}

/// The user's card collection, accepted either as an object or as its JSON text.
///
/// The upload pipeline hands out the text form, so accepting it directly saves callers
/// a parse-and-reserialize round trip. This is a newtype over [`RawValue`] rather than
/// an untagged enum because untagged deserialization buffers its input, which the
/// raw-value passthrough does not survive.
#[derive(Debug, Deserialize)]
pub struct UserInput(Box<RawValue>);

impl UserInput {
    pub fn parse(&self) -> Result<UserProfile, ApiError> {
        let text = json_text(&self.0)?;
        parse_user_profile_json(&text)
            .map_err(|error| ApiError::BadRequest(format!("user data: {error}")))
    }
}

/// Returns the JSON text a raw value stands for: a JSON string is unescaped into the
/// document it carries, anything else is already the document.
pub fn json_text(raw: &RawValue) -> Result<String, ApiError> {
    let text = raw.get();
    if text.starts_with('"') {
        serde_json::from_str::<String>(text)
            .map_err(|error| ApiError::BadRequest(format!("expected JSON text: {error}")))
    } else {
        Ok(text.to_string())
    }
}

/// JSON body extractor that reports malformed input in the service's error shape
/// instead of axum's default plain-text rejection.
pub struct ApiJson<T>(pub T);

impl<T, S> axum::extract::FromRequest<S> for ApiJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| ApiError::BadRequest(rejection.body_text()))?;
        Ok(ApiJson(value))
    }
}

/// Parses build parameters straight from the request body.
///
/// The accepted keys are exactly the engine's own contract, documented in
/// `docs/parameters.md`; the service adds no parameter dialect of its own.
pub fn parse_params(raw: Option<&RawValue>) -> Result<BuildParams, ApiError> {
    let text = raw.map_or("{}", RawValue::get);
    parse_build_params_json(text).map_err(|error| ApiError::BadRequest(format!("params: {error}")))
}

/// Builds the router with every endpoint and the shared middleware stack.
pub fn router(state: Arc<AppState>) -> axum::Router {
    let body_limit = state.config.max_body_bytes;
    axum::Router::new()
        .route("/v1/recommend", post(recommend::recommend))
        .route(
            "/v1/recommend/challenge-all",
            post(recommend::challenge_all),
        )
        .route(
            "/v1/world-bloom/support-cards",
            post(auxiliary::world_bloom_support_cards),
        )
        .route("/v1/music/recommend", post(auxiliary::music_recommend))
        .route("/v1/live/exact-score", post(auxiliary::live_exact_score))
        .route(
            "/v1/area-items/recommend",
            post(auxiliary::area_items_recommend),
        )
        .route("/v1/regions", get(ops::regions))
        .route("/admin/reload", post(ops::reload))
        .route("/healthz", get(ops::healthz))
        .route("/readyz", get(ops::readyz))
        .route("/metrics", get(ops::metrics))
        .route("/openapi.json", get(ops::openapi))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}
