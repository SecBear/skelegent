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
//!
//! # Streaming
//!
//! The `/v1/execute/stream` endpoint returns real-time SSE events as the
//! operator executes. Progress updates, artifacts, and effects are streamed
//! incrementally — not buffered to completion.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::prelude::*;
use layer0::dispatch::{DispatchEvent, DispatchHandle, EffectEmitter};
use layer0::{DispatchContext, DispatchId, OperatorId};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tracing::error;

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

/// Classify an `OrchError` into a structured SSE error payload.
///
/// Returns a JSON value with `error` (human-readable message), `code`
/// (machine-readable variant), and `retryable` (whether the caller can retry).
fn classify_error(error: &layer0::error::OrchError) -> serde_json::Value {
    use layer0::error::{OperatorError, OrchError};

    let (code, retryable) = match error {
        OrchError::OperatorNotFound(_) => ("operator_not_found", false),
        OrchError::WorkflowNotFound(_) => ("workflow_not_found", false),
        OrchError::DispatchFailed(_) => ("dispatch_failed", true),
        OrchError::SignalFailed(_) => ("signal_failed", true),
        OrchError::OperatorError(op_err) => match op_err {
            OperatorError::Model { retryable, .. } => {
                if *retryable {
                    ("model_error_retryable", true)
                } else {
                    ("model_error", false)
                }
            }
            OperatorError::SubDispatch { .. } => ("tool_error", false),
            OperatorError::ContextAssembly { .. } => ("context_error", false),
            OperatorError::Retryable { .. } => ("retryable_error", true),
            OperatorError::NonRetryable { .. } => ("non_retryable_error", false),
            OperatorError::Halted { .. } => ("halted", false),
            _ => ("operator_error", false),
        },
        OrchError::EnvironmentError(_) => ("environment_error", false),
        _ => ("internal_error", false),
    };

    serde_json::json!({
        "error": error.to_string(),
        "code": code,
        "retryable": retryable,
    })
}

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
/// Events are streamed in real-time as the operator executes:
///
/// - `event: progress`  — intermediate progress content (reasoning, status)
/// - `event: artifact`  — intermediate artifact produced during execution
/// - `event: effect`    — side-effect emitted by the operator
/// - `event: output`    — JSON-serialized `OperatorOutput` (final result)
/// - `event: done`      — signals completion (empty data)
/// - `event: error`     — JSON `{"error": "..."}` on failure
///
/// # Effect interpretation
///
/// Effects are included in the stream but **not interpreted** by the runner.
/// Callers must handle effects.
async fn execute_stream_handler(
    State(runner): State<Arc<RunnerServiceImpl>>,
    Json(req): Json<JsonExecuteRequest>,
) -> Result<impl IntoResponse, CoreError> {
    runner.validate_session_key(&req.session_key)?;

    let input = runner.deserialize_input_from_b64(&req.input)?;
    let operator = runner.resolve_operator(&req.operator)?;
    let _spec_bytes = BASE64_STANDARD
        .decode(&req.spec)
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 in `spec`: {e}")))?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    let op_id = req.operator;
    tokio::spawn(async move {
        // Create a dispatch channel so the operator can emit real-time events.
        let dispatch_id = DispatchId::new("runner-sse");
        let (mut handle, sender) = DispatchHandle::channel(dispatch_id);
        let emitter = EffectEmitter::new(sender.clone());

        // Spawn the operator execution, then send the terminal event.
        let op_id_inner = op_id.clone();
        tokio::spawn(async move {
            let ctx =
                DispatchContext::new(DispatchId::new("runner-sse"), OperatorId::new(op_id_inner));
            match operator.execute(input, &ctx, &emitter).await {
                Ok(output) => {
                    let _ = sender.send(DispatchEvent::Completed { output }).await;
                }
                Err(op_err) => {
                    let _ = sender
                        .send(DispatchEvent::Failed {
                            error: op_err.into(),
                        })
                        .await;
                }
            }
            // Drop sender to close the channel.
        });

        // Forward dispatch events as SSE events.
        while let Some(event) = handle.recv().await {
            let sse_event = match &event {
                DispatchEvent::Progress { content } => match serde_json::to_string(content) {
                    Ok(json) => Event::default().event("progress").data(json),
                    Err(e) => {
                        error!("failed to serialize progress: {e}");
                        continue;
                    }
                },
                DispatchEvent::ArtifactProduced { artifact } => {
                    match serde_json::to_string(artifact) {
                        Ok(json) => Event::default().event("artifact").data(json),
                        Err(e) => {
                            error!("failed to serialize artifact: {e}");
                            continue;
                        }
                    }
                }
                DispatchEvent::EffectEmitted { effect } => match serde_json::to_string(effect) {
                    Ok(json) => Event::default().event("effect").data(json),
                    Err(e) => {
                        error!("failed to serialize effect: {e}");
                        continue;
                    }
                },
                DispatchEvent::Completed { output } => match serde_json::to_string(output) {
                    Ok(json) => Event::default().event("output").data(json),
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
                },
                DispatchEvent::Failed { error } => {
                    let err_json = classify_error(error);
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("error")
                            .data(err_json.to_string())))
                        .await;
                    return;
                }
                _ => continue, // Future variants — skip gracefully.
            };

            if tx.send(Ok(sse_event)).await.is_err() {
                return; // client disconnected
            }

            // If we just sent the terminal output, signal done.
            if matches!(event, DispatchEvent::Completed { .. }) {
                let _ = tx.send(Ok(Event::default().event("done").data(""))).await;
                return;
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
