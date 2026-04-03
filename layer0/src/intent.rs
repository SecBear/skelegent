//! Executable intent model.

#![allow(missing_docs)]

use crate::duration::DurationMs;
use crate::effect::{HandoffContext, MemoryScope, Scope, SignalPayload};
use crate::id::{OperatorId, WorkflowId};
use crate::operator::OperatorInput;
use crate::state::{ContentKind, Lifetime, MemoryLink, MemoryTier};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static INTENT_COUNTER: AtomicU64 = AtomicU64::new(0);
static INTENT_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_intent_id() -> String {
    let n = INTENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("intent-{n}")
}

fn next_seq() -> u64 {
    INTENT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Executable intent envelope.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// Causal/replay metadata.
    pub meta: IntentMeta,
    /// Executable intent payload.
    pub kind: IntentKind,
}

/// Intent metadata used for replay ordering/correlation.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentMeta {
    /// Unique intent identifier.
    pub intent_id: String,
    /// Immediate cause identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    /// Correlation identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Deterministic sequence number.
    pub seq: u64,
}

impl Default for IntentMeta {
    fn default() -> Self {
        Self::new()
    }
}

impl IntentMeta {
    /// Create metadata with generated ID and monotonic sequence.
    pub fn new() -> Self {
        Self {
            intent_id: next_intent_id(),
            causation_id: None,
            correlation_id: None,
            seq: next_seq(),
        }
    }
}

/// Executable intent variants.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IntentKind {
    /// Write a value to state/memory.
    WriteMemory {
        scope: Scope,
        key: String,
        value: serde_json::Value,
        #[serde(default)]
        memory_scope: MemoryScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<MemoryTier>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<Lifetime>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_kind: Option<ContentKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        salience: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl: Option<DurationMs>,
    },
    /// Delete a value from state/memory.
    DeleteMemory { scope: Scope, key: String },
    /// Create a memory relationship.
    LinkMemory { scope: Scope, link: MemoryLink },
    /// Remove a memory relationship.
    UnlinkMemory {
        scope: Scope,
        from_key: String,
        to_key: String,
        relation: String,
    },
    /// Send a signal to a workflow.
    Signal {
        target: WorkflowId,
        payload: SignalPayload,
    },
    /// Delegate to another operator.
    Delegate {
        operator: OperatorId,
        input: Box<OperatorInput>,
    },
    /// Hand off control to another operator.
    Handoff {
        operator: OperatorId,
        context: HandoffContext,
    },
    /// Request human/policy approval.
    RequestApproval {
        tool_name: String,
        call_id: String,
        input: serde_json::Value,
    },
    /// Domain-specific executable intent.
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}

impl Intent {
    /// Create an intent with generated metadata.
    pub fn new(kind: IntentKind) -> Self {
        Self {
            meta: IntentMeta::new(),
            kind,
        }
    }
}
