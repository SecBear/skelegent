//! [`DispatchRecorder`] — records dispatch operations via [`DispatchMiddleware`].

use crate::{Boundary, RecordContext, RecordEntry, RecordSink};
use async_trait::async_trait;
use layer0::dispatch::DispatchHandle;
use layer0::dispatch_context::DispatchContext;
use layer0::error::OrchError;
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
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
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
    ) -> Result<DispatchHandle, OrchError> {
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
        let duration_ms = start.elapsed().as_millis() as u64;

        // Post-phase: record outcome.
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::Dispatch,
                record_ctx,
                serde_json::Value::Null,
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
    use crate::{InMemorySink, Phase};
    use layer0::ExitReason;
    use layer0::content::Content;
    use layer0::dispatch::{DispatchEvent, DispatchHandle};
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{OperatorOutput, TriggerType};

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
        ) -> Result<DispatchHandle, OrchError> {
            Ok(immediate_handle(OperatorOutput::new(
                input.message,
                ExitReason::Complete,
            )))
        }
    }

    #[tokio::test]
    async fn dispatch_recorder_captures_pre_and_post() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = DispatchRecorder::new(sink.clone());

        let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::from("my-op"));
        let input = OperatorInput::new(Content::text("hello"), TriggerType::User);

        let result = recorder.dispatch(&ctx, input, &EchoNext).await;
        assert!(result.is_ok());

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
            ) -> Result<DispatchHandle, OrchError> {
                Err(OrchError::DispatchFailed("boom".into()))
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
    }
}
