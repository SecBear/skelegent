//! Effect system — side-effects declared by operators for external execution.

use crate::dispatch::Artifact;
use crate::duration::DurationMs;
use crate::id::*;
use crate::state::{ContentKind, Lifetime, MemoryTier};
use serde::{Deserialize, Serialize};

/// A side-effect declared by an operator. NOT executed by the operator —
/// the calling layer decides when and how to execute it.
///
/// This is the key composability mechanism. An operator running in-process
/// has its effects executed by a simple loop. An operator running in Temporal
/// has its effects serialized into the workflow history. An operator running
/// in a test harness has its effects captured for assertions.
///
/// The Custom variant ensures future effect types can be represented
/// without changing the enum. When a new effect type stabilizes
/// (used by 3+ implementations), it graduates to a named variant.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    /// Write a value to persistent state.
    WriteMemory {
        /// The scope to write into.
        scope: Scope,
        /// The key to write.
        key: String,
        /// The value to store.
        value: serde_json::Value,
        /// Advisory storage tier hint. Backends may ignore this.
        /// Defaults to `None` (no hint).
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

    /// Delete a value from persistent state.
    DeleteMemory {
        /// The scope to delete from.
        scope: Scope,
        /// The key to delete.
        key: String,
    },

    /// Send a fire-and-forget signal to another operator or workflow.
    Signal {
        /// The target workflow to signal.
        target: WorkflowId,
        /// The signal payload.
        payload: SignalPayload,
    },

    /// Request that the orchestrator dispatch another operator.
    /// This is how delegation works — the operator doesn't call the
    /// other operator directly, it asks the orchestrator to do it.
    Delegate {
        /// The operator to delegate to.
        operator: OperatorId,
        /// The input to send to the delegated operator.
        input: Box<OperatorInput>,
    },

    /// Hand off the conversation to another operator. Unlike Delegate,
    /// the current operator is done — the next operator takes over.
    Handoff {
        /// The operator to hand off to.
        operator: OperatorId,
        /// State to pass to the next operator. This is NOT the full
        /// conversation — it's whatever the current operator thinks
        /// the next operator needs to continue.
        state: serde_json::Value,
    },

    /// Emit a log/trace event. Observers and telemetry consume these.
    Log {
        /// Severity level.
        level: LogLevel,
        /// Log message.
        message: String,
        /// Optional structured data.
        data: Option<serde_json::Value>,
    },

    /// Create a link between two memory entries.
    LinkMemory {
        /// The scope for the link.
        scope: Scope,
        /// The link to create.
        link: crate::state::MemoryLink,
    },

    /// Remove a link between two memory entries.
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

    /// A tool call requires human approval before execution.
    /// The operator loop should exit with [`ExitReason::AwaitingApproval`]
    /// and the calling layer decides whether to approve, deny, or modify.
    ToolApprovalRequired {
        /// Name of the tool requesting approval.
        tool_name: String,
        /// Provider-assigned call ID for correlation.
        call_id: String,
        /// The input the model wants to send to the tool.
        input: serde_json::Value,
    },

    /// Emit intermediate progress visible to the dispatch caller.
    ///
    /// The dispatch layer converts this into a
    /// [`DispatchEvent::Progress`](crate::dispatch::DispatchEvent::Progress)
    /// on the caller's handle. Use for reasoning traces, status updates,
    /// or partial outputs during long-running operations.
    Progress {
        /// Progress content.
        content: crate::content::Content,
    },

    /// Produce an intermediate deliverable during execution.
    ///
    /// The dispatch layer converts this into a
    /// [`DispatchEvent::ArtifactProduced`](crate::dispatch::DispatchEvent::ArtifactProduced)
    /// on the caller's handle. Use for files, structured data, or any
    /// named output produced before the operator finishes.
    Artifact {
        /// The artifact to emit.
        artifact: Artifact,
    },

    /// Future effect types. Named string + arbitrary payload.
    /// Use this for domain-specific effects that aren't general
    /// enough for a named variant.
    Custom {
        /// The custom effect type identifier.
        effect_type: String,
        /// Arbitrary payload.
        data: serde_json::Value,
    },
}

// Forward-declare OperatorInput usage for the Delegate variant.
use crate::operator::OperatorInput;

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

/// Log severity levels.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Finest-grained tracing.
    Trace,
    /// Debug-level detail.
    Debug,
    /// Informational messages.
    Info,
    /// Warnings.
    Warn,
    /// Errors.
    Error,
}
