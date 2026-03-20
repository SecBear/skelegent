//! Effect system — side-effects declared by operators for external execution.

use crate::content::Content;
use crate::context::Message;
use crate::dispatch::Artifact;
use crate::duration::DurationMs;
use crate::id::*;
use crate::state::{ContentKind, Lifetime, MemoryTier};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── ID generation ────────────────────────────────────────────────────────────

/// Monotonic counter for unique effect IDs within this process.
static EFFECT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_effect_id() -> String {
    let n = EFFECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("eff-{n}")
}


/// Monotonic counter for total ordering of effects within this process.
static EFFECT_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_seq() -> u64 {
    EFFECT_SEQ.fetch_add(1, Ordering::Relaxed)
}
// ── serde helpers for SystemTime ─────────────────────────────────────────────

mod serde_system_time {
    use super::*;

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let nanos = t
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos() as u64;
        nanos.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let nanos = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_nanos(nanos))
    }
}

// ── EffectMeta ────────────────────────────────────────────────────────────────

/// Causal metadata attached to every effect instance.
///
/// Follows the OTel three-ID pattern: `effect_id` (unique), `causation_id`
/// (immediate cause), `correlation_id` (root trace copied through the chain).
/// `seq` provides a total ordering within a dispatch run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectMeta {
    /// Unique identifier for this effect instance.
    pub effect_id: String,
    /// ID of the effect that caused this one (causal chain).
    /// `None` for root effects triggered by external input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    /// Correlation ID grouping related effects across dispatches.
    /// Copied unchanged through the entire causal chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Sequence number within the current dispatch run.
    /// Provides total ordering for replay.
    pub seq: u64,
    /// Wall-clock time when the effect was created.
    /// Stored as nanoseconds since UNIX epoch. Use `seq` for ordering, not this.
    #[serde(with = "serde_system_time")]
    pub timestamp: SystemTime,
}

impl Default for EffectMeta {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectMeta {
    /// Create new metadata with a generated `effect_id` and current timestamp.
    pub fn new() -> Self {
        Self {
            effect_id: next_effect_id(),
            causation_id: None,
            correlation_id: None,
            seq: next_seq(),
            timestamp: SystemTime::now(),
        }
    }

    /// Create with an explicit causation chain.
    pub fn with_cause(causation_id: String, correlation_id: Option<String>) -> Self {
        Self {
            effect_id: next_effect_id(),
            causation_id: Some(causation_id),
            correlation_id,
            seq: next_seq(),
            timestamp: SystemTime::now(),
        }
    }
}

// ── MemoryScope ───────────────────────────────────────────────────────────────

/// Memory lifetime scope for [`EffectKind::WriteMemory`].
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

// ── HandoffContext ───────────────────────────────────────────────────────────

/// Structured context passed to the receiving operator on handoff.
///
/// `task` is the primary input for the next operator — the message it should
/// act on. `history` optionally forwards prior conversation turns so the
/// receiving operator has context. `metadata` carries any unstructured
/// domain-specific data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffContext {
    /// The task/message to pass to the next operator.
    pub task: Content,
    /// Optional conversation history to forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Message>>,
    /// Optional unstructured metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// ── EffectKind ────────────────────────────────────────────────────────────────

/// The payload of a declared side-effect.
///
/// Renamed from the previous `Effect` enum. The variants are identical
/// except:
/// - [`WriteMemory`](EffectKind::WriteMemory) gains a `memory_scope` field.
/// - [`Handoff`](EffectKind::Handoff) replaces `metadata: Option<Value>` with
///   a structured [`HandoffContext`] carrying `task`, `history`, and `metadata`.
/// - [`Custom`](EffectKind::Custom) renames `effect_type`/`data` to
///   `name`/`payload`.
/// - New variants: [`Log`](EffectKind::Log), [`Observation`](EffectKind::Observation),
///   [`Metric`](EffectKind::Metric).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectKind {
    /// Write a value to persistent state.
    WriteMemory {
        /// The scope to write into.
        scope: Scope,
        /// The key to write.
        key: String,
        /// The value to store.
        value: serde_json::Value,
        /// Memory lifetime scope. Defaults to [`MemoryScope::Session`].
        #[serde(default)]
        memory_scope: MemoryScope,
        /// Advisory storage tier hint. Backends may ignore this.
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
    ///
    /// The operator does not call the other operator directly — it asks
    /// the orchestrator to do it. This is the delegation mechanism.
    Delegate {
        /// The operator to delegate to.
        operator: OperatorId,
        /// The input to send to the delegated operator.
        input: Box<OperatorInput>,
    },

    /// Hand off the conversation to another operator.
    ///
    /// Unlike [`Delegate`](EffectKind::Delegate), the current operator is
    /// done — the next operator takes over entirely.
    Handoff {
        /// The operator to hand off to.
        operator: OperatorId,
        /// Structured context for the receiving operator.
        context: HandoffContext,
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
    ///
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
    /// on the caller's handle.
    Progress {
        /// Progress content.
        content: crate::content::Content,
    },

    /// Produce an intermediate deliverable during execution.
    ///
    /// The dispatch layer converts this into a
    /// [`DispatchEvent::ArtifactProduced`](crate::dispatch::DispatchEvent::ArtifactProduced)
    /// on the caller's handle.
    Artifact {
        /// The artifact to emit.
        artifact: Artifact,
    },

    /// Emit a structured log entry.
    Log {
        /// Log severity level (e.g., `"info"`, `"warn"`, `"error"`).
        level: String,
        /// Human-readable log message.
        message: String,
    },

    /// Record an observation key-value pair for debugging or tracing.
    Observation {
        /// Observation key.
        key: String,
        /// Observation value.
        value: serde_json::Value,
    },

    /// Emit a numeric metric for monitoring.
    Metric {
        /// Metric name.
        name: String,
        /// Numeric value.
        value: f64,
        /// Metric tags for dimensionality.
        #[serde(default)]
        tags: std::collections::HashMap<String, String>,
    },

    /// Future effect types. Named string + arbitrary payload.
    ///
    /// Use this for domain-specific effects that aren't general enough
    /// for a named variant. When a type stabilizes (3+ implementations),
    /// it graduates to a named variant.
    Custom {
        /// The custom effect type identifier.
        name: String,
        /// Arbitrary payload.
        payload: serde_json::Value,
    },
}

impl EffectKind {
    /// Whether this effect kind should be durably logged.
    ///
    /// Durable effects represent observable state changes or control-flow
    /// decisions that must survive a restart for correct replay.
    /// Ephemeral effects (Log, Progress, Metric, Observation, Artifact) are
    /// informational and do not need to be preserved.
    pub fn is_durable(&self) -> bool {
        matches!(
            self,
            EffectKind::WriteMemory { .. }
                | EffectKind::DeleteMemory { .. }
                | EffectKind::Signal { .. }
                | EffectKind::Delegate { .. }
                | EffectKind::Handoff { .. }
                | EffectKind::ToolApprovalRequired { .. }
                | EffectKind::Custom { .. }
        )
    }
}

// ── Effect ────────────────────────────────────────────────────────────────────

/// A side-effect declared by an operator. NOT executed by the operator —
/// the calling layer decides when and how to execute it.
///
/// This is the key composability mechanism. An operator running in-process
/// has its effects executed by a simple loop. An operator running in Temporal
/// has its effects serialized into the workflow history. An operator running
/// in a test harness has its effects captured for assertions.
///
/// `meta` carries causal metadata (effect ID, causation chain, sequence number,
/// timestamp) that is uniform across all effect types without per-variant
/// boilerplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Effect {
    /// Causal metadata for this effect instance.
    pub meta: EffectMeta,
    /// The effect payload.
    pub kind: EffectKind,
}

impl Effect {
    /// Create an effect with auto-generated metadata.
    pub fn new(kind: EffectKind) -> Self {
        Self {
            meta: EffectMeta::new(),
            kind,
        }
    }

    /// Create an effect with explicit causation metadata.
    pub fn with_cause(
        kind: EffectKind,
        causation_id: String,
        correlation_id: Option<String>,
    ) -> Self {
        Self {
            meta: EffectMeta::with_cause(causation_id, correlation_id),
            kind,
        }
    }

    /// Convenience: create a [`EffectKind::WriteMemory`] effect with `Session` scope.
    pub fn write_memory(
        scope: Scope,
        key: String,
        value: serde_json::Value,
        memory_scope: MemoryScope,
    ) -> Self {
        Self::new(EffectKind::WriteMemory {
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

    /// Convenience: create a [`EffectKind::DeleteMemory`] effect.
    pub fn delete_memory(scope: Scope, key: String) -> Self {
        Self::new(EffectKind::DeleteMemory { scope, key })
    }

    /// Convenience: create a [`EffectKind::Handoff`] effect.
    pub fn handoff(operator: OperatorId, context: HandoffContext) -> Self {
        Self::new(EffectKind::Handoff { operator, context })
    }

    /// Convenience: create a [`EffectKind::Signal`] effect.
    pub fn signal(target: WorkflowId, payload: SignalPayload) -> Self {
        Self::new(EffectKind::Signal { target, payload })
    }

    /// Convenience: create a [`EffectKind::Custom`] effect.
    pub fn custom(name: String, payload: serde_json::Value) -> Self {
        Self::new(EffectKind::Custom { name, payload })
    }

    /// Convenience: create a [`EffectKind::Log`] effect.
    pub fn log(level: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(EffectKind::Log {
            level: level.into(),
            message: message.into(),
        })
    }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effect_meta_auto_generated() {
        let meta1 = EffectMeta::new();
        let meta2 = EffectMeta::new();
        // IDs must be non-empty and distinct.
        assert!(!meta1.effect_id.is_empty());
        assert_ne!(meta1.effect_id, meta2.effect_id);
        // Timestamps should be set (not the epoch).
        assert!(meta1.timestamp >= UNIX_EPOCH);
        assert!(meta1.seq < meta2.seq, "seq must be monotonically increasing");
        // causation and correlation are None by default.
        assert!(meta1.causation_id.is_none());
        assert!(meta1.correlation_id.is_none());
    }

    #[test]
    fn effect_kind_serde_round_trip() {
        fn round_trip(kind: EffectKind) {
            let effect = Effect::new(kind);
            let json = serde_json::to_string(&effect).expect("serialize");
            let back: Effect = serde_json::from_str(&json).expect("deserialize");
            // Re-serialize and compare — not comparing structs directly
            // since SystemTime precision may differ.
            let json2 = serde_json::to_string(&back).expect("re-serialize");
            assert_eq!(json, json2);
        }

        round_trip(EffectKind::WriteMemory {
            scope: Scope::Global,
            key: "k".into(),
            value: json!(42),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        });
        round_trip(EffectKind::DeleteMemory {
            scope: Scope::Global,
            key: "k".into(),
        });
        round_trip(EffectKind::Signal {
            target: WorkflowId::new("wf"),
            payload: SignalPayload::new("sig", json!({})),
        });
        round_trip(EffectKind::Handoff {
            operator: OperatorId::new("op"),
            context: HandoffContext {
                task: Content::text("done"),
                history: None,
                metadata: Some(json!({"reason": "done"})),
            },
        });
        round_trip(EffectKind::ToolApprovalRequired {
            tool_name: "my_tool".into(),
            call_id: "c1".into(),
            input: json!({"x": 1}),
        });
        round_trip(EffectKind::Log {
            level: "info".into(),
            message: "hello".into(),
        });
        round_trip(EffectKind::Observation {
            key: "latency_ms".into(),
            value: json!(42),
        });
        round_trip(EffectKind::Metric {
            name: "tokens".into(),
            value: 100.0,
            tags: std::collections::HashMap::from([("model".into(), "gpt-4".into())]),
        });
        round_trip(EffectKind::Custom {
            name: "my.event".into(),
            payload: json!({"foo": "bar"}),
        });
    }

    #[test]
    fn memory_scope_default_is_session() {
        assert_eq!(MemoryScope::default(), MemoryScope::Session);
        // Verify round-trip for the default value.
        let scope = MemoryScope::default();
        let json = serde_json::to_string(&scope).expect("serialize");
        let back: MemoryScope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, MemoryScope::Session);
    }

    #[test]
    fn effect_convenience_constructors() {
        // write_memory
        let e = Effect::write_memory(
            Scope::Global,
            "key".into(),
            json!({"v": 1}),
            MemoryScope::Session,
        );
        assert!(matches!(e.kind, EffectKind::WriteMemory { ref key, .. } if key == "key"));

        // delete_memory
        let e = Effect::delete_memory(Scope::Global, "key2".into());
        assert!(matches!(e.kind, EffectKind::DeleteMemory { ref key, .. } if key == "key2"));

        // handoff
        let e = Effect::handoff(
            OperatorId::new("target"),
            HandoffContext {
                task: Content::text("do the next thing"),
                history: None,
                metadata: None,
            },
        );
        assert!(matches!(
            e.kind,
            EffectKind::Handoff { ref context, .. } if context.metadata.is_none()
        ));

        // signal
        let e = Effect::signal(WorkflowId::new("wf"), SignalPayload::new("t", json!({})));
        assert!(matches!(e.kind, EffectKind::Signal { .. }));

        // custom
        let e = Effect::custom("my.event".into(), json!({}));
        assert!(matches!(e.kind, EffectKind::Custom { ref name, .. } if name == "my.event"));

        // log
        let e = Effect::log("warn", "disk full");
        assert!(matches!(
            e.kind,
            EffectKind::Log { ref message, .. } if message == "disk full"
        ));
    }

    #[test]
    fn seq_auto_increments() {
        let e1 = Effect::new(EffectKind::Log { level: "info".into(), message: "a".into() });
        let e2 = Effect::new(EffectKind::Log { level: "info".into(), message: "b".into() });
        let e3 = Effect::new(EffectKind::Log { level: "info".into(), message: "c".into() });
        assert!(e1.meta.seq < e2.meta.seq, "seq must increase");
        assert!(e2.meta.seq < e3.meta.seq, "seq must increase");
    }
}
