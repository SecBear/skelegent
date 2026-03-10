//! HTTP/JSON convenience adapter for the Runner service.
//!
//! Provides REST-style endpoints that delegate to the same
//! `RunnerServiceImpl` used by the gRPC path.
//!
//! Endpoints:
//! - `GET  /health`       — Docker healthcheck
//! - `POST /v1/execute`   — JSON execute (base64-encoded input/output)
//!
//! TODO: `POST /v1/execute/stream` — SSE streaming variant

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{CoreError, RunnerServiceImpl};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct JsonExecuteRequest {
    pub operator: String,
    /// Base64-encoded input bytes (same payload as protobuf `bytes input`).
    pub input: String,
    /// Base64-encoded environment spec bytes.
    pub spec: String,
    pub idempotency_key: String,
    pub session_key: String,
}

#[derive(Serialize)]
pub struct JsonExecuteResponse {
    /// Base64-encoded output bytes.
    pub output: String,
}

#[derive(Serialize)]
pub struct JsonHealthResponse {
    pub ready: bool,
    pub version: String,
}

#[derive(Serialize)]
pub struct JsonError {
    pub error: String,
    pub code: String,
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

impl CoreError {
    fn to_http(&self) -> (StatusCode, Json<JsonError>) {
        let (status, code) = match self {
            CoreError::Unauthenticated(_) => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            CoreError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            CoreError::InvalidArgument(_) => (StatusCode::BAD_REQUEST, "invalid_argument"),
            CoreError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        (
            status,
            Json(JsonError {
                error: self.message().to_string(),
                code: code.to_string(),
            }),
        )
    }
}

impl IntoResponse for CoreError {
    fn into_response(self) -> Response {
        let (status, body) = self.to_http();
        (status, body).into_response()
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler() -> Json<JsonHealthResponse> {
    Json(JsonHealthResponse {
        ready: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn execute_handler(
    State(runner): State<Arc<RunnerServiceImpl>>,
    Json(req): Json<JsonExecuteRequest>,
) -> Result<Json<JsonExecuteResponse>, CoreError> {
    runner.validate_session_key(&req.session_key)?;

    let input_bytes = BASE64_STANDARD
        .decode(&req.input)
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 in `input`: {e}")))?;

    // `spec` is accepted for forward-compat but unused by execute_core today.
    let _spec_bytes = BASE64_STANDARD
        .decode(&req.spec)
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 in `spec`: {e}")))?;

    let output_bytes = runner.execute_core(&req.operator, &input_bytes).await?;

    Ok(Json(JsonExecuteResponse {
        output: BASE64_STANDARD.encode(&output_bytes),
    }))
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

/// Build the axum router with `/health` and `/v1/execute`.
///
/// Body size is capped at 16 MiB.
pub fn router(runner: Arc<RunnerServiceImpl>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/execute", post(execute_handler))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(runner)
}
