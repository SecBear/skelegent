//! [`DispatchRecorder`] — records dispatch operations via [`DispatchMiddleware`].

use crate::{Boundary, RecordContext, RecordEntry, RecordSink, SCHEMA_VERSION};
use async_trait::async_trait;
use layer0::dispatch::{DispatchEvent, DispatchHandle};
use layer0::dispatch_context::DispatchContext;
use layer0::error::ProtocolError;
use layer0::middleware::{DispatchMiddleware, DispatchNext};
use layer0::operator::OperatorInput;
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every dispatch operation to a [`RecordSink`].
///
/// Captures two entries per dispatch:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with the serialized [`OperatorInput`]
/// - [`Phase::Post`](crate::Phase::Post) — recorded asynchronously via [`DispatchHandle::intercept`]
///   when the [`DispatchEvent::Completed`] terminal event arrives, carrying the actual
///   [`layer0::operator::OperatorOutput`] as its payload. For dispatches that return an
///   immediate error (before a handle is returned), Post is recorded synchronously with the
///   error information.
///
/// The [`RecordContext`] is populated from the [`DispatchContext`]:
/// `trace_id`, `operator_id`, and `dispatch_id`.
pub struct DispatchRecorder {
    sink: Arc<dyn RecordSink>,
}

impl DispatchRecorder {
    /// Create a new dispatch recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl DispatchMiddleware for DispatchRecorder {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, ProtocolError> {
        let record_ctx = RecordContext {
            trace_id: ctx.trace.trace_id.clone(),
            operator_id: ctx.operator_id.to_string(),
            dispatch_id: ctx.dispatch_id.to_string(),
        };

        // Pre-phase: serialize OperatorInput to JSON payload.
        let payload = serde_json::to_value(&input)
            .unwrap_or_else(|e| serde_json::json!({"serialize_error": e.to_string()}));
        self.sink
            .record(RecordEntry::pre(
                Boundary::Dispatch,
                record_ctx.clone(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.dispatch(ctx, input).await;

        match result {
            Err(e) => {
                // Immediate failure — record Post synchronously with the error.
                let duration_ms = start.elapsed().as_millis() as u64;
                let error_str = e.to_string();
                self.sink
                    .record(RecordEntry::post(
                        Boundary::Dispatch,
                        record_ctx,
                        serde_json::json!({"error": error_str}),
                        duration_ms,
                        Some(error_str),
                    ))
                    .await;
                Err(e)
            }
            Ok(handle) => {
                // Success — use intercept() to capture the terminal event asynchronously.
                // The Post-phase entry is written when Completed or Failed arrives on the handle.
                let sink = self.sink.clone();
                // `intercept` takes a sync `Fn(&DispatchEvent)` — the callback contract has no
                // async support. We cannot `.await` sink.record() directly here; a spawned task
                // is the only option. TODO: if DispatchHandle::intercept gains an async-callback
                // variant, replace `tokio::spawn` with a direct `.await` on `sink.record(entry)`.
                let intercepted = handle.intercept(move |event| {
                    let (payload, error) = match event {
                        DispatchEvent::Completed { output } => {
                            let p = serde_json::to_value(output).unwrap_or(serde_json::Value::Null);
                            (p, None)
                        }
                        DispatchEvent::Failed { error } => {
                            let msg = error.to_string();
                            (serde_json::json!({"error": msg}), Some(msg))
                        }
                        _ => return,
                    };
                    // Compute duration at terminal event time, not at handle-creation time.
                    let duration_ms = start.elapsed().as_millis() as u64;
                    let entry = RecordEntry {
                        boundary: Boundary::Dispatch,
                        phase: crate::Phase::Post,
                        context: record_ctx.clone(),
                        payload_json: payload,
                        duration_ms: Some(duration_ms),
                        error,
                        version: SCHEMA_VERSION,
                    };
                    let sink = sink.clone();
                    tokio::spawn(async move { sink.record(entry).await });
                });
                Ok(intercepted)
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InMemorySink, Phase};
    use layer0::content::Content;
    use layer0::dispatch::{DispatchEvent, DispatchHandle};
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{Outcome, OperatorOutput, TerminalOutcome, TriggerType};

    fn immediate_handle(output: OperatorOutput) -> DispatchHandle {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("test"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        handle
    }

    struct EchoNext;

    #[async_trait]
    impl DispatchNext for EchoNext {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            input: OperatorInput,
        ) -> Result<DispatchHandle, ProtocolError> {
            Ok(immediate_handle(OperatorOutput::new(
                input.message,
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed,
                },
            )))
        }
    }

    #[tokio::test]
    async fn dispatch_recorder_captures_pre_and_post() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = DispatchRecorder::new(sink.clone());

        let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::from("my-op"));
        let input = OperatorInput::new(Content::text("hello"), TriggerType::User);

        let handle = recorder.dispatch(&ctx, input, &EchoNext).await.unwrap();
        // Consume the handle so the intercept task flushes the Completed event.
        let _ = handle.collect().await.unwrap();
        // Yield to allow the spawned sink.record() task to complete.
        tokio::task::yield_now().await;

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post entries");

        assert_eq!(entries[0].phase, Phase::Pre);
        assert_eq!(entries[0].boundary, Boundary::Dispatch);
        assert_eq!(entries[0].context.operator_id, "my-op");
        assert_eq!(entries[0].context.dispatch_id, "d-001");
        assert!(entries[0].duration_ms.is_none());
        assert!(entries[0].error.is_none());

        assert_eq!(entries[1].phase, Phase::Post);
        assert_eq!(entries[1].boundary, Boundary::Dispatch);
        assert!(entries[1].duration_ms.is_some());
        assert!(entries[1].error.is_none());
        // Post-phase now carries the actual OperatorOutput instead of {"status": "dispatched"}.
        // The output message is "hello" echoed back by EchoNext.
        assert!(entries[1].payload_json.is_object());
        assert!(
            entries[1].payload_json.get("status").is_none(),
            "old stub payload must not appear"
        );
    }

    #[tokio::test]
    async fn dispatch_recorder_post_contains_operator_output() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = DispatchRecorder::new(sink.clone());

        let ctx = DispatchContext::new(DispatchId::new("d-002"), OperatorId::from("echo-op"));
        let input = OperatorInput::new(Content::text("world"), TriggerType::User);

        let handle = recorder.dispatch(&ctx, input, &EchoNext).await.unwrap();
        let output = handle.collect().await.unwrap();
        tokio::task::yield_now().await;

        assert_eq!(output.message.as_text().unwrap(), "world");

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        // The payload should be a serialized OperatorOutput (has "message" and "exit_reason" fields).
        assert!(
            post.payload_json.get("message").is_some()
                || post.payload_json.get("exit_reason").is_some(),
            "post payload should contain OperatorOutput fields, got: {}",
            post.payload_json
        );
    }

    #[tokio::test]
    async fn dispatch_recorder_captures_error() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = DispatchRecorder::new(sink.clone());

        struct FailNext;
        #[async_trait]
        impl DispatchNext for FailNext {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
            ) -> Result<DispatchHandle, ProtocolError> {
                Err(ProtocolError::unavailable("boom"))
            }
        }

        let ctx = DispatchContext::new(DispatchId::new("d-err"), OperatorId::from("op"));
        let input = OperatorInput::new(Content::text("fail"), TriggerType::User);
        let result = recorder.dispatch(&ctx, input, &FailNext).await;
        assert!(result.is_err());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert!(post.error.is_some());
        assert!(
            post.payload_json.get("error").is_some(),
            "post payload must contain error field"
        );
    }

    /// A `DispatchNext` that returns a handle which emits `DispatchEvent::Failed`.
    struct HandleFailNext;

    #[async_trait]
    impl DispatchNext for HandleFailNext {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            _input: OperatorInput,
        ) -> Result<DispatchHandle, ProtocolError> {
            let (handle, sender) = DispatchHandle::channel(DispatchId::new("hf-001"));
            tokio::spawn(async move {
                let _ = sender
                    .send(DispatchEvent::Failed {
                        error: ProtocolError::unavailable("stream failed"),
                    })
                    .await;
            });
            Ok(handle)
        }
    }

    #[tokio::test]
    async fn dispatch_recorder_captures_handle_failed_event() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = DispatchRecorder::new(sink.clone());

        let ctx = DispatchContext::new(DispatchId::new("d-hf"), OperatorId::from("op"));
        let input = OperatorInput::new(Content::text("x"), TriggerType::User);

        let handle = recorder
            .dispatch(&ctx, input, &HandleFailNext)
            .await
            .unwrap();
        // collect() drains events; the Failed terminal event triggers the intercept.
        let result = handle.collect().await;
        assert!(result.is_err(), "expected Err from Failed event");
        // Yield to allow the spawned sink.record() task to flush.
        tokio::task::yield_now().await;

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post");
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::Dispatch);
        assert!(
            post.error.is_some(),
            "post error must be set for Failed event"
        );
        assert!(
            post.payload_json.get("error").is_some(),
            "post payload must contain error field, got: {}",
            post.payload_json
        );
        assert!(post.duration_ms.is_some(), "duration must be recorded");
    }
}
