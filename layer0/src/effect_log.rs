//! Append-only effect log for audit, replay, and debugging.

use crate::effect::Effect;
use std::future::Future;
use std::sync::RwLock;

// ── EffectLogError ────────────────────────────────────────────────────────────

/// Errors from effect log operations.
#[derive(Debug, thiserror::Error)]
pub enum EffectLogError {
    /// Storage backend error.
    #[error("storage error: {0}")]
    Storage(String),
}

// ── EffectLog ─────────────────────────────────────────────────────────────────

/// Append-only log of effects for audit, replay, and debugging.
///
/// Effects are appended during execution and can be read back for
/// replay, debugging, or projection into derived state.
///
/// This trait uses RPITIT (return-position `impl Trait`) rather than
/// `async-trait`, so it is **not** object-safe. Use a concrete generic
/// bound (`impl EffectLog` or `L: EffectLog`) rather than `dyn EffectLog`.
pub trait EffectLog: Send + Sync {
    /// Append an effect to the log.
    fn append(&self, effect: &Effect) -> impl Future<Output = Result<(), EffectLogError>> + Send;

    /// Read effects in append order, starting from `offset`.
    ///
    /// Returns up to `limit` effects. If `offset` exceeds the log length,
    /// returns an empty vec.
    fn read(
        &self,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Effect>, EffectLogError>> + Send;

    /// Return the current head position (number of effects logged).
    fn head(&self) -> impl Future<Output = Result<usize, EffectLogError>> + Send;

    /// Read effects filtered by `correlation_id`.
    ///
    /// Only effects whose `meta.correlation_id` matches the given string are
    /// returned. Applies `offset` and `limit` to the filtered set.
    fn read_by_correlation(
        &self,
        correlation_id: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Effect>, EffectLogError>> + Send;
}

// ── InMemoryEffectLog ─────────────────────────────────────────────────────────

/// In-memory effect log backed by a `Vec`.
///
/// Suitable for testing, development, and short-lived replay scenarios.
/// Not durable: all entries are lost when the instance is dropped.
///
/// Locking strategy: [`std::sync::RwLock`] is used intentionally. All lock
/// acquisitions are synchronous and brief (a single `push` or `clone`);
/// no lock is ever held across an `.await` point.
pub struct InMemoryEffectLog {
    entries: RwLock<Vec<Effect>>,
}

impl InMemoryEffectLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }

    /// Return a snapshot of all logged effects in append order.
    pub fn snapshot(&self) -> Vec<Effect> {
        self.entries.read().expect("rwlock poisoned").clone()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.write().expect("rwlock poisoned").clear();
    }
}

impl Default for InMemoryEffectLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectLog for InMemoryEffectLog {
    fn append(&self, effect: &Effect) -> impl Future<Output = Result<(), EffectLogError>> + Send {
        // Do all work synchronously: clone the effect, acquire the lock,
        // push, and drop the guard — nothing is held across an await point.
        let cloned = effect.clone();
        let result = self
            .entries
            .write()
            .map(|mut guard| guard.push(cloned))
            .map_err(|e| EffectLogError::Storage(e.to_string()));
        async move { result }
    }

    fn read(
        &self,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Effect>, EffectLogError>> + Send {
        let result = self
            .entries
            .read()
            .map(|guard| guard.iter().skip(offset).take(limit).cloned().collect())
            .map_err(|e| EffectLogError::Storage(e.to_string()));
        async move { result }
    }

    fn head(&self) -> impl Future<Output = Result<usize, EffectLogError>> + Send {
        let result = self
            .entries
            .read()
            .map(|guard| guard.len())
            .map_err(|e| EffectLogError::Storage(e.to_string()));
        async move { result }
    }

    fn read_by_correlation(
        &self,
        correlation_id: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Effect>, EffectLogError>> + Send {
        // Own the string so the future is 'static (doesn't borrow self).
        let cid = correlation_id.to_owned();
        let result = self
            .entries
            .read()
            .map(|guard| {
                guard
                    .iter()
                    .filter(|e| e.meta.correlation_id.as_deref() == Some(cid.as_str()))
                    .skip(offset)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .map_err(|e| EffectLogError::Storage(e.to_string()));
        async move { result }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Effect, EffectKind, MemoryScope, Scope};
    use crate::id::{SessionId, WorkflowId};
    use serde_json::json;

    fn make_write(key: &str) -> Effect {
        Effect::write_memory(
            Scope::Session(SessionId::new("s1")),
            key.to_owned(),
            json!(1),
            MemoryScope::Session,
        )
    }

    #[tokio::test]
    async fn append_and_read() {
        let log = InMemoryEffectLog::new();
        log.append(&make_write("a")).await.unwrap();
        log.append(&make_write("b")).await.unwrap();
        log.append(&make_write("c")).await.unwrap();

        let entries = log.read(0, 10).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Verify append order is preserved.
        assert!(entries[0].meta.seq < entries[1].meta.seq);
        assert!(entries[1].meta.seq < entries[2].meta.seq);
    }

    #[tokio::test]
    async fn read_with_offset_and_limit() {
        let log = InMemoryEffectLog::new();
        for i in 0..5 {
            log.append(&make_write(&format!("k{i}"))).await.unwrap();
        }

        let slice = log.read(2, 2).await.unwrap();
        assert_eq!(slice.len(), 2);
        assert!(slice[0].meta.seq < slice[1].meta.seq);
    }

    #[tokio::test]
    async fn head_tracks_count() {
        let log = InMemoryEffectLog::new();
        assert_eq!(log.head().await.unwrap(), 0);
        log.append(&make_write("a")).await.unwrap();
        assert_eq!(log.head().await.unwrap(), 1);
        log.append(&make_write("b")).await.unwrap();
        assert_eq!(log.head().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn read_by_correlation() {
        let log = InMemoryEffectLog::new();

        let mut e1 = make_write("a");
        e1.meta.correlation_id = Some("trace-1".to_owned());
        let mut e2 = make_write("b");
        e2.meta.correlation_id = Some("trace-2".to_owned());
        let mut e3 = make_write("c");
        e3.meta.correlation_id = Some("trace-1".to_owned());

        log.append(&e1).await.unwrap();
        log.append(&e2).await.unwrap();
        log.append(&e3).await.unwrap();

        let trace1 = log.read_by_correlation("trace-1", 0, 10).await.unwrap();
        assert_eq!(trace1.len(), 2);
        assert!(
            trace1[0].meta.seq < trace1[1].meta.seq,
            "trace1 entries should be in order"
        );

        let trace2 = log.read_by_correlation("trace-2", 0, 10).await.unwrap();
        assert_eq!(trace2.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_returns_all() {
        let log = InMemoryEffectLog::new();
        log.append(&make_write("a")).await.unwrap();
        log.append(&make_write("b")).await.unwrap();

        let snap = log.snapshot();
        assert_eq!(snap.len(), 2);
    }

    #[test]
    fn is_durable_classification() {
        use crate::content::Content;
        use crate::effect::{HandoffContext, SignalPayload};
        use crate::id::OperatorId;
        use crate::operator::{OperatorInput, TriggerType};

        // ── Durable variants ──────────────────────────────────────────────────
        let durable_cases: &[(&str, EffectKind)] = &[
            (
                "WriteMemory",
                EffectKind::WriteMemory {
                    scope: Scope::Global,
                    key: "k".into(),
                    value: json!(1),
                    memory_scope: MemoryScope::Session,
                    tier: None,
                    lifetime: None,
                    content_kind: None,
                    salience: None,
                    ttl: None,
                },
            ),
            (
                "DeleteMemory",
                EffectKind::DeleteMemory {
                    scope: Scope::Global,
                    key: "k".into(),
                },
            ),
            (
                "Signal",
                EffectKind::Signal {
                    target: WorkflowId::new("wf"),
                    payload: SignalPayload::new("s", json!({})),
                },
            ),
            (
                "Delegate",
                EffectKind::Delegate {
                    operator: OperatorId::new("op"),
                    input: Box::new(OperatorInput::new(Content::text("hi"), TriggerType::Task)),
                },
            ),
            (
                "Handoff",
                EffectKind::Handoff {
                    operator: OperatorId::new("op"),
                    context: HandoffContext {
                        task: Content::text("go"),
                        history: None,
                        metadata: None,
                    },
                },
            ),
            (
                "ToolApprovalRequired",
                EffectKind::ToolApprovalRequired {
                    tool_name: "bash".into(),
                    call_id: "c1".into(),
                    input: json!({}),
                },
            ),
            (
                "Custom",
                EffectKind::Custom {
                    name: "my.event".into(),
                    payload: json!({}),
                },
            ),
        ];

        for (name, kind) in durable_cases {
            assert!(kind.is_durable(), "{name} should be durable");
        }

        // ── Ephemeral variants ────────────────────────────────────────────────
        use crate::dispatch::Artifact;
        let ephemeral_cases: &[(&str, EffectKind)] = &[
            (
                "Log",
                EffectKind::Log {
                    level: "info".into(),
                    message: "msg".into(),
                },
            ),
            (
                "Progress",
                EffectKind::Progress {
                    content: Content::text("50%"),
                },
            ),
            (
                "Observation",
                EffectKind::Observation {
                    key: "latency".into(),
                    value: json!(42),
                },
            ),
            (
                "Metric",
                EffectKind::Metric {
                    name: "tokens".into(),
                    value: 100.0,
                    tags: Default::default(),
                },
            ),
            (
                "Artifact",
                EffectKind::Artifact {
                    artifact: Artifact {
                        id: "art-1".into(),
                        name: None,
                        description: None,
                        parts: vec![Content::text("result")],
                        metadata: None,
                        append: false,
                        last_chunk: true,
                    },
                },
            ),
        ];

        for (name, kind) in ephemeral_cases {
            assert!(!kind.is_durable(), "{name} should not be durable");
        }
    }
}
