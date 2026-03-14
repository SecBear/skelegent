//! HTTP/JSON convenience adapter for the Runner service.
//!
//! Provides REST-style endpoints that delegate to the same
//! `RunnerServiceImpl` used by the gRPC path.
//!
//! Endpoints:
//! - `GET  /health`              — Docker healthcheck
//! - `POST /v1/execute`          — JSON execute (base64-encoded input/output)
//! - `POST /v1/execute/stream`   — SSE streaming variant
//!
//! # Effect interpretation
//!
//! The runner is a **deployment harness**, not an orchestrator.
//! Operator effects (tool calls, state mutations, etc.) are captured and
//! returned in the response body but **never interpreted** by the runner.
//! Callers that receive effects in the response are responsible for
//! deciding how and when to execute them.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;

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
    pub _idempotency_key: String,
    pub session_key: String,
}

/// Response from `POST /v1/execute`.
///
/// Contains the full serialized `OperatorOutput` (base64-encoded) plus an
/// explicit flag indicating whether the output contains unhandled effects.
/// The runner does **not** interpret effects — callers must inspect
/// `has_unhandled_effects` and the effects within the decoded output to
/// decide what action to take.
#[derive(Serialize)]
pub struct JsonExecuteResponse {
    /// Base64-encoded `OperatorOutput` JSON bytes.
    pub output: String,
    /// `true` when the output contains effects that were not handled by the
    /// runner. Callers should decode `output` and inspect the `effects`
    /// field to determine what action is required.
    pub has_unhandled_effects: bool,
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

/// Execute an operator and return the full output.
///
/// Effects are included in the response but **not interpreted** by the runner.
/// Callers must inspect `has_unhandled_effects` and handle effects externally.
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

    let output = runner.execute_operator(&req.operator, &input_bytes).await?;
    let has_unhandled = output.has_unhandled_effects();

    let output_bytes = serde_json::to_vec(&output)
        .map_err(|e| CoreError::Internal(format!("failed to serialize OperatorOutput: {e}")))?;

    Ok(Json(JsonExecuteResponse {
        output: BASE64_STANDARD.encode(&output_bytes),
        has_unhandled_effects: has_unhandled,
    }))
}

/// SSE streaming execute endpoint.
///
/// Accepts the same input as `POST /v1/execute` but returns an SSE stream.
/// The stream emits:
/// - `event: output` — JSON-serialized `OperatorOutput` (the final result)
/// - `event: done`   — signals completion (empty data)
/// - `event: error`  — JSON `{"error": "..."}` on failure
///
/// **Note:** This is a "streaming envelope" — the operator itself runs to
/// completion, then the output is streamed as a single event. True real-time
/// streaming (where partial results arrive during execution) requires
/// `EffectEmitter` integration.
///
/// # Effect interpretation
///
/// As with the non-streaming endpoint, effects are included in the response
/// but **not interpreted** by the runner. Callers must handle effects.
///
// TODO: Integrate `EffectEmitter` callback to stream partial effects and
// intermediate messages as they are produced during operator execution.
async fn execute_stream_handler(
    State(runner): State<Arc<RunnerServiceImpl>>,
    Json(req): Json<JsonExecuteRequest>,
) -> Result<impl IntoResponse, CoreError> {
    runner.validate_session_key(&req.session_key)?;

    let input_bytes = BASE64_STANDARD
        .decode(&req.input)
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 in `input`: {e}")))?;

    let _spec_bytes = BASE64_STANDARD
        .decode(&req.spec)
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 in `spec`: {e}")))?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);

    let operator_id = req.operator;
    tokio::spawn(async move {
        match runner.execute_operator(&operator_id, &input_bytes).await {
            Ok(output) => {
                let json = match serde_json::to_string(&output) {
                    Ok(j) => j,
                    Err(e) => {
                        let err_json =
                            serde_json::json!({ "error": format!("serialize failed: {e}") });
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("error")
                                .data(err_json.to_string())))
                            .await;
                        return;
                    }
                };

                // Send the full output as a single event.
                if tx
                    .send(Ok(Event::default().event("output").data(json)))
                    .await
                    .is_err()
                {
                    return; // client disconnected
                }

                // Signal completion.
                let _ = tx.send(Ok(Event::default().event("done").data(""))).await;
            }
            Err(core_err) => {
                let err_json = serde_json::json!({
                    "error": core_err.message(),
                    "code": match &core_err {
                        CoreError::Unauthenticated(_) => "unauthenticated",
                        CoreError::NotFound(_) => "not_found",
                        CoreError::InvalidArgument(_) => "invalid_argument",
                        CoreError::Internal(_) => "internal",
                    },
                });
                let _ = tx
                    .send(Ok(Event::default()
                        .event("error")
                        .data(err_json.to_string())))
                    .await;
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

/// Build the axum router with `/health`, `/v1/execute`, and `/v1/execute/stream`.
///
/// Body size is capped at 16 MiB. The streaming endpoint returns
/// `Content-Type: text/event-stream` with `Cache-Control: no-cache`
/// automatically via axum's [`Sse`] response type.
pub fn router(runner: Arc<RunnerServiceImpl>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/execute", post(execute_handler))
        .route("/v1/execute/stream", post(execute_stream_handler))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(runner)
}
