//! Executable intent model.

#![allow(missing_docs)]

use crate::duration::DurationMs;
use crate::id::{OperatorId, SessionId, WorkflowId};
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

// ── Shared scope and payload types ───────────────────────────────────────────

/// Where state lives. Scopes are hierarchical — a session scope
/// is narrower than a workflow scope, which is narrower than global.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Per-conversation.
    Session(SessionId),
    /// Per-workflow-execution.
    Workflow(WorkflowId),
    /// Per-operator within a workflow.
    Operator {
        /// The workflow this operator belongs to.
        workflow: WorkflowId,
        /// The operator within the workflow.
        operator: OperatorId,
    },
    /// Shared across all workflows.
    Global,
    /// Future scopes.
    Custom(String),
}

/// Memory lifetime scope for write-memory intents.
///
/// Controls how long a written value is expected to live. Backends may
/// interpret or enforce this differently — this is advisory, not a contract.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Discarded at end of the current turn.
    Turn,
    /// Lives for the session duration. This is the default.
    #[default]
    Session,
    /// Scoped to a named entity such as a user, project, or tenant.
    Entity(String),
    /// Persists globally across all sessions.
    Global,
}

/// Structured context passed to the receiving operator on handoff.
///
/// `task` is the primary input for the next operator — the message it should
/// act on. `history` optionally forwards prior conversation turns so the
/// receiving operator has context. `metadata` carries any unstructured
/// domain-specific data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffContext {
    /// The task/message to pass to the next operator.
    pub task: crate::content::Content,
    /// Optional conversation history to forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<crate::context::Message>>,
    /// Optional unstructured metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Payload for inter-operator/workflow signals.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalPayload {
    /// The type of signal being sent.
    pub signal_type: String,
    /// Signal data.
    pub data: serde_json::Value,
}

impl SignalPayload {
    /// Create a new signal payload.
    pub fn new(signal_type: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            signal_type: signal_type.into(),
            data,
        }
    }
}

// ── Intent envelope ───────────────────────────────────────────────────────────

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
        /// The scope to write into.
        scope: Scope,
        /// The key to write.
        key: String,
        /// The value to store.
        value: serde_json::Value,
        /// Memory lifetime scope.
        #[serde(default)]
        memory_scope: MemoryScope,
        /// Advisory storage tier hint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<MemoryTier>,
        /// Advisory persistence policy.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<Lifetime>,
        /// Cognitive category of the memory.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_kind: Option<ContentKind>,
        /// Write-time importance hint (0.0–1.0).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        salience: Option<f64>,
        /// Auto-expire after this duration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl: Option<DurationMs>,
    },
    /// Delete a value from state/memory.
    DeleteMemory {
        /// The scope to delete from.
        scope: Scope,
        /// The key to delete.
        key: String,
    },
    /// Create a memory relationship.
    LinkMemory {
        /// The scope for the link.
        scope: Scope,
        /// The link to create.
        link: MemoryLink,
    },
    /// Remove a memory relationship.
    UnlinkMemory {
        /// The scope for the unlink.
        scope: Scope,
        /// Source key.
        from_key: String,
        /// Target key.
        to_key: String,
        /// Relationship type.
        relation: String,
    },
    /// Send a signal to a workflow.
    Signal {
        /// The target workflow to signal.
        target: WorkflowId,
        /// The signal payload.
        payload: SignalPayload,
    },
    /// Delegate to another operator.
    Delegate {
        /// The operator to delegate to.
        operator: OperatorId,
        /// The input to pass to the delegated operator.
        input: Box<OperatorInput>,
    },
    /// Hand off control to another operator.
    Handoff {
        /// The operator to hand off to.
        operator: OperatorId,
        /// Structured context for the receiving operator.
        context: HandoffContext,
    },
    /// Request human/policy approval.
    RequestApproval {
        /// Name of the tool requesting approval.
        tool_name: String,
        /// Provider-assigned call ID for correlation.
        call_id: String,
        /// The input the model wants to send to the tool.
        input: serde_json::Value,
    },
    /// Domain-specific executable intent.
    Custom {
        /// The custom intent type identifier.
        name: String,
        /// Arbitrary payload.
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

    /// Write a value to state/memory.
    pub fn write_memory(
        scope: Scope,
        key: String,
        value: serde_json::Value,
        memory_scope: MemoryScope,
    ) -> Self {
        Self::new(IntentKind::WriteMemory {
            scope,
            key,
            value,
            memory_scope,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        })
    }

    /// Delete a value from state/memory.
    pub fn delete_memory(scope: Scope, key: String) -> Self {
        Self::new(IntentKind::DeleteMemory { scope, key })
    }

    /// Hand off control to another operator.
    pub fn handoff(operator: OperatorId, context: HandoffContext) -> Self {
        Self::new(IntentKind::Handoff { operator, context })
    }

    /// Send a signal to a workflow.
    pub fn signal(target: WorkflowId, payload: SignalPayload) -> Self {
        Self::new(IntentKind::Signal { target, payload })
    }

    /// Domain-specific executable intent.
    pub fn custom(name: String, payload: serde_json::Value) -> Self {
        Self::new(IntentKind::Custom { name, payload })
    }
}
