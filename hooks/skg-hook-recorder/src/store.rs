//! [`StoreRecorder`] — records state store operations via [`StoreMiddleware`].

use crate::{Boundary, RecordContext, RecordEntry, RecordSink};
use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::middleware::{StoreMiddleware, StoreReadNext, StoreWriteNext};
use layer0::state::StoreOptions;
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STORE RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every state read and write to a [`RecordSink`].
///
/// Captures two entries per operation:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with scope/key/value in payload
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
///
/// Writes are recorded with [`Boundary::StoreWrite`]; reads with [`Boundary::StoreRead`].
/// Both use [`RecordContext::empty`] since store operations don't carry a dispatch context.
pub struct StoreRecorder {
    sink: Arc<dyn RecordSink>,
}

impl StoreRecorder {
    /// Create a new store recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl StoreMiddleware for StoreRecorder {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        let payload = serde_json::json!({
            "scope": serde_json::to_value(scope).unwrap_or_default(),
            "key": key,
            "value": value,
        });

        self.sink
            .record(RecordEntry::pre(
                Boundary::StoreWrite,
                RecordContext::empty(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.write(scope, key, value, options).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let post_payload = match &result {
            Ok(()) => serde_json::json!({"status": "ok"}),
            Err(_) => serde_json::Value::Null,
        };
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::StoreWrite,
                RecordContext::empty(),
                post_payload,
                duration_ms,
                error,
            ))
            .await;

        result
    }

    async fn read(
        &self,
        scope: &Scope,
        key: &str,
        next: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let payload = serde_json::json!({
            "scope": serde_json::to_value(scope).unwrap_or_default(),
            "key": key,
        });

        self.sink
            .record(RecordEntry::pre(
                Boundary::StoreRead,
                RecordContext::empty(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.read(scope, key).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let post_payload = match &result {
            Ok(Some(value)) => value.clone(),
            Ok(None) => serde_json::json!({"found": false}),
            Err(_) => serde_json::Value::Null,
        };
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::StoreRead,
                RecordContext::empty(),
                post_payload,
                duration_ms,
                error,
            ))
            .await;

        result
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Boundary, InMemorySink, Phase};
    use layer0::id::{OperatorId, WorkflowId};

    struct NoOpWriteNext;

    #[async_trait]
    impl StoreWriteNext for NoOpWriteNext {
        async fn write(
            &self,
            _scope: &Scope,
            _key: &str,
            _value: serde_json::Value,
            _options: Option<&StoreOptions>,
        ) -> Result<(), StateError> {
            Ok(())
        }
    }

    struct EchoReadNext {
        value: serde_json::Value,
    }

    #[async_trait]
    impl StoreReadNext for EchoReadNext {
        async fn read(
            &self,
            _scope: &Scope,
            _key: &str,
        ) -> Result<Option<serde_json::Value>, StateError> {
            Ok(Some(self.value.clone()))
        }
    }

    fn test_scope() -> Scope {
        Scope::Operator {
            workflow: WorkflowId::from("wf-1"),
            operator: OperatorId::from("op-1"),
        }
    }

    #[tokio::test]
    async fn store_recorder_captures_write() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = StoreRecorder::new(sink.clone());

        let scope = test_scope();
        let result = recorder
            .write(
                &scope,
                "my-key",
                serde_json::json!({"x": 1}),
                None,
                &NoOpWriteNext,
            )
            .await;
        assert!(result.is_ok());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post entries");

        let pre = &entries[0];
        assert_eq!(pre.phase, Phase::Pre);
        assert_eq!(pre.boundary, Boundary::StoreWrite);
        assert_eq!(pre.payload_json["key"], "my-key");

        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::StoreWrite);
        assert!(post.duration_ms.is_some());
        assert!(post.error.is_none());
        assert_eq!(post.payload_json["status"], "ok");
    }

    #[tokio::test]
    async fn store_recorder_captures_read() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = StoreRecorder::new(sink.clone());

        let scope = test_scope();
        let next = EchoReadNext {
            value: serde_json::json!(42),
        };
        let result = recorder.read(&scope, "some-key", &next).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(serde_json::json!(42)));

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);

        let pre = &entries[0];
        assert_eq!(pre.phase, Phase::Pre);
        assert_eq!(pre.boundary, Boundary::StoreRead);
        assert_eq!(pre.payload_json["key"], "some-key");

        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::StoreRead);
        // Post payload should be the returned value (42 in this case).
        assert_eq!(post.payload_json, serde_json::json!(42));
    }
}
