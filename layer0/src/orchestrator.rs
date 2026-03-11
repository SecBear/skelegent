//! The Orchestrator protocol — how operators from different agents compose.

use crate::dispatch::Dispatcher;
use crate::{error::OrchError, id::*, operator::OperatorInput, operator::OperatorOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Protocol ② — Orchestration
///
/// How operators compose, and how execution survives failures.
/// Durability and composition are inseparable — Temporal replay IS
/// orchestration IS crash recovery. They're the same system.
///
/// Extends [`Dispatcher`] — every orchestrator IS-A dispatcher.
/// One invocation primitive, used everywhere: by the top-level
/// caller, by composing operators (via `Arc<dyn Dispatcher>`),
/// and by the framework itself.
///
/// Implementations:
/// - LocalOrchestrator: in-process, tokio tasks, no durability
/// - TemporalOrchestrator: Temporal workflows, full durability
/// - RestateOrchestrator: Restate, durable execution
/// - HttpOrchestrator: dispatch over HTTP (microservice pattern)
///
/// The key property: calling code doesn't know which implementation
/// is behind the trait. `dispatch()` might be a function call or a
/// network hop to another continent. The trait is transport-agnostic.
#[async_trait]
pub trait Orchestrator: Dispatcher {
    /// Dispatch multiple operator invocations in parallel.
    ///
    /// The implementation decides whether this is tokio::join!,
    /// Temporal child workflows, parallel HTTP requests, or something else.
    ///
    /// Returns results in the same order as the input tasks.
    /// Individual tasks may fail independently.
    async fn dispatch_many(
        &self,
        tasks: Vec<(OperatorId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>>;

    /// Fire-and-forget signal to a running workflow.
    ///
    /// Used for: inter-agent messaging, user feedback injection,
    /// budget adjustments, cancellation.
    ///
    /// Returns Ok(()) when the signal is accepted (not when it's
    /// processed — that's async by nature).
    async fn signal(
        &self,
        target: &WorkflowId,
        signal: crate::effect::SignalPayload,
    ) -> Result<(), OrchError>;

    /// Read-only query of a running workflow's state.
    ///
    /// Used for: dashboards, status checks, budget queries.
    ///
    /// Returns a JSON value — the schema depends on the workflow.
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
