//! Semantic execution event model.

#![allow(missing_docs)]

use crate::approval::ApprovalRequest;
use crate::content::Content;
use crate::dispatch::Artifact;
use crate::dispatch_context::DispatchContext;
use crate::error::ProtocolError;
use crate::id::{DispatchId, OperatorId};
use crate::intent::Intent;
use crate::operator::OperatorOutput;
use crate::wait::WaitState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);
static EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_event_id() -> String {
    let n = EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("event-{n}")
}

fn next_seq() -> u64 {
    EVENT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Semantic execution event envelope.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    /// Event metadata.
    pub meta: EventMeta,
    /// Event payload.
    pub kind: EventKind,
}

/// Metadata attached to every semantic event.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMeta {
    /// Unique event identifier.
    pub event_id: String,
    /// Dispatch this event belongs to.
    pub dispatch_id: DispatchId,
    /// Parent dispatch for delegated invocations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_dispatch_id: Option<DispatchId>,
    /// Correlation identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Deterministic event ordering sequence.
    pub seq: u64,
    /// Wall-clock timestamp in unix milliseconds.
    pub timestamp_unix_ms: u64,
    /// Semantic source classification.
    pub source: EventSource,
}

impl EventMeta {
    /// Build metadata using dispatch context and source.
    pub fn from_context(ctx: &DispatchContext, source: EventSource) -> Self {
        Self {
            event_id: next_event_id(),
            dispatch_id: ctx.dispatch_id.clone(),
            parent_dispatch_id: ctx.parent_id.clone(),
            correlation_id: Some(ctx.trace.trace_id.clone()).filter(|s| !s.is_empty()),
            seq: next_seq(),
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            source,
        }
    }
}

/// High-level semantic source of an event.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// Runtime-originated semantic event.
    Runtime,
    /// Runtime projection from provider transport.
    ProviderProjection,
    /// Intent execution path.
    IntentExecutor,
    /// Orchestrator-originated semantic event.
    Orchestrator,
    /// Domain-specific source.
    Custom(String),
}

/// Semantic event variants.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// Invocation started.
    InvocationStarted { operator: OperatorId },
    /// Inference started.
    InferenceStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// A full tool call was assembled.
    ToolCallAssembled {
        call_id: String,
        capability_id: String,
        input: serde_json::Value,
    },
    /// Tool execution result materialized.
    ToolResultReceived {
        call_id: String,
        capability_id: String,
        output: serde_json::Value,
    },
    /// Intent was declared.
    IntentDeclared { intent: Intent },
    /// Progress content.
    Progress { content: Content },
    /// Artifact produced.
    ArtifactProduced { artifact: Artifact },
    /// Log message.
    Log { level: String, message: String },
    /// Observation key/value payload.
    Observation {
        key: String,
        value: serde_json::Value,
    },
    /// Metric sample.
    Metric {
        name: String,
        value: f64,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
    /// Invocation suspended.
    Suspended {
        wait: WaitState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_request: Option<ApprovalRequest>,
    },
    /// Terminal success.
    Completed { output: OperatorOutput },
    /// Terminal failure.
    Failed { error: ProtocolError },
}

impl ExecutionEvent {
    /// Create an event with metadata derived from context.
    pub fn new(ctx: &DispatchContext, source: EventSource, kind: EventKind) -> Self {
        Self {
            meta: EventMeta::from_context(ctx, source),
            kind,
        }
    }
}
