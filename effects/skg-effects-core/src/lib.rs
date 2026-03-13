#![deny(missing_docs)]
//! Core effect execution primitives: trait and error types.

use async_trait::async_trait;
use layer0::effect::Effect;
use layer0::error::{OrchError, StateError};
use layer0::id::WorkflowId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for effect execution.
#[derive(Debug, Error)]
pub enum Error {
    /// Dispatch error.
    #[error("orchestrator error: {0}")]
    Dispatch(#[from] OrchError),
    /// State backend error.
    #[error("state error: {0}")]
    State(#[from] StateError),
    /// An unknown/unhandled effect was encountered and policy is Error.
    #[error("unknown or unsupported effect encountered")]
    UnknownEffect,
}

/// Policy for handling unknown/custom effects.
#[derive(Debug, Clone, Copy, Default)]
pub enum UnknownEffectPolicy {
    /// Ignore unknown/custom effects and log a warning via `tracing::warn`.
    #[default]
    IgnoreAndWarn,
    /// Treat unknown/custom effects as an error.
    Error,
}

/// Execute Layer 0 effects in a deterministic, ordered fashion.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute a list of effects in order. Implementations must preserve
    /// the provided order and short-circuit on the first error.
    async fn execute(&self, effects: &[Effect]) -> Result<(), Error>;
}

// ── Capability traits ────────────────────────────────────────────────────────

/// Fire-and-forget signal delivery to a running workflow.
///
/// This is an independent capability — not all dispatchers support it.
/// Local (non-durable) implementations may journal signals in-memory;
/// durable implementations forward them to the workflow engine.
#[async_trait]
pub trait Signalable: Send + Sync {
    /// Send a signal to a running workflow.
    ///
    /// Returns `Ok(())` when the signal is accepted (not when processed).
    async fn signal(
        &self,
        target: &WorkflowId,
        signal: layer0::effect::SignalPayload,
    ) -> Result<(), OrchError>;
}

/// Read-only query of a running workflow's state.
///
/// This is an independent capability — not all dispatchers support it.
/// The query schema is workflow-defined; the trait provides transport only.
#[async_trait]
pub trait Queryable: Send + Sync {
    /// Query a running workflow's state.
    ///
    /// Returns a JSON value whose schema depends on the workflow.
    async fn query(
        &self,
        target: &WorkflowId,
        query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError>;
}

/// Payload for querying a running workflow.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPayload {
    /// The type of query to execute.
    pub query_type: String,
    /// Query parameters.
    pub params: serde_json::Value,
}

impl QueryPayload {
    /// Create a new query payload.
    pub fn new(query_type: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            query_type: query_type.into(),
            params,
        }
    }
}
