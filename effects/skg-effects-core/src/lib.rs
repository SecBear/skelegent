#![deny(missing_docs)]
//! Core effect handling primitives.
//!
//! This crate defines the [`EffectHandler`] trait — the single interface between
//! effect declarations ([`Effect`]) and their execution. Modeled after the
//! Elm Cmd pattern: handlers interpret effects and return structured
//! [`EffectOutcome`]s. The caller decides what to do with dispatch outcomes.
//!
//! Implementations:
//! - `LocalEffectHandler` (in `skg-effects-local`) — in-process state + signaling
//! - `TemporalEffectHandler` (in `skg-effects-temporal`) — durable via Temporal

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::Intent;
use layer0::dispatch::Dispatcher;
use layer0::error::{ProtocolError, StateError};
use layer0::id::{DispatchId, OperatorId, WorkflowId};
use layer0::operator::OperatorInput;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for effect handling.
#[derive(Debug, Error)]
pub enum Error {
    /// Protocol error.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
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

// ── Effect handling ─────────────────────────────────────────────────────────

/// Outcome of handling a single effect.
///
/// Inspired by the Elm `Cmd` pattern and algebraic effect handlers:
/// the handler interprets an effect declaration and returns what happened.
/// The caller decides what to do with dispatch outcomes — enqueue for
/// depth-limited execution, dispatch immediately, or capture for testing.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum EffectOutcome {
    /// The effect was executed successfully (state written, signal sent, etc.).
    Applied,
    /// The effect was intentionally skipped (middleware guard suppressed a write,
    /// backend doesn't support this effect type, or unknown-effect policy is ignore).
    Skipped,
    /// A follow-up delegation dispatch is needed.
    /// The handler does NOT dispatch — the caller decides when and how.
    Delegate {
        /// The operator to delegate to.
        operator: OperatorId,
        /// The input for the delegated operator.
        input: OperatorInput,
    },
    /// A follow-up handoff dispatch is needed.
    /// Semantically distinct from [`Delegate`](Self::Delegate): the current
    /// operator is done and the next operator takes over the conversation.
    Handoff {
        /// The operator to hand off to.
        operator: OperatorId,
        /// The input for the handoff target (constructed from handoff state).
        input: OperatorInput,
    },
}

/// Handle a single intent and return what happened.
///
/// This is the single interface between intent declarations ([`Intent`] from
/// `layer0`) and their execution. Inspired by algebraic effect handlers:
/// the handler interprets an intent, performs any side effects (state writes,
/// signal delivery), and returns a structured outcome.
///
/// Dispatch intents (`Delegate`, `Handoff`) are NEVER executed by the handler.
/// They are returned as [`EffectOutcome::Delegate`] / [`EffectOutcome::Handoff`]
/// so the caller can control the dispatch loop (depth limiting, tracing,
/// durable scheduling).
///
/// For fire-and-forget callers, [`execute_intents`] dispatches immediately.
#[async_trait]
pub trait EffectHandler: Send + Sync {
    /// Handle a single intent. Returns the outcome for the caller to act on.
    async fn handle(&self, intent: &Intent, ctx: &DispatchContext) -> Result<EffectOutcome, Error>;
}

/// Execute all intents, dispatching followups immediately.
///
/// Convenience for callers that don't need depth limiting or trace recording.
/// Calls [`EffectHandler::handle`] for each intent and dispatches any
/// [`EffectOutcome::Delegate`] / [`EffectOutcome::Handoff`] outcomes through
/// the provided dispatcher.
pub async fn execute_intents(
    handler: &dyn EffectHandler,
    intents: &[Intent],
    ctx: &DispatchContext,
    dispatcher: &dyn Dispatcher,
) -> Result<(), Error> {
    for effect in intents {
        match handler.handle(effect, ctx).await? {
            EffectOutcome::Applied | EffectOutcome::Skipped => {}
            EffectOutcome::Delegate { operator, input }
            | EffectOutcome::Handoff { operator, input } => {
                let child_ctx = ctx.child(DispatchId::new(operator.as_str()), operator);
                dispatcher
                    .dispatch(&child_ctx, input)
                    .await
                    .map_err(Error::Protocol)?
                    .collect()
                    .await
                    .map_err(Error::Protocol)?;
            }
        }
    }
    Ok(())
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
        signal: layer0::SignalPayload,
    ) -> Result<(), ProtocolError>;
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
    ) -> Result<serde_json::Value, ProtocolError>;
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
