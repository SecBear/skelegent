//! [`ExecRecorder`] — records environment execution operations via [`ExecMiddleware`].

use crate::{Boundary, RecordContext, RecordEntry, RecordSink};
use async_trait::async_trait;
use layer0::environment::EnvironmentSpec;
use layer0::error::EnvError;
use layer0::dispatch_context::DispatchContext;
use layer0::middleware::{ExecMiddleware, ExecNext};
use layer0::operator::{OperatorInput, OperatorOutput};
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXEC RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every environment execution to a [`RecordSink`].
///
/// Captures two entries per execution:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with the serialized [`OperatorInput`]
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
///
/// Uses [`Boundary::Exec`] and the passed [`DispatchContext`] to populate trace context,
/// giving accurate correlation without depending on an ambient OTel span.
pub struct ExecRecorder {
    sink: Arc<dyn RecordSink>,
}

impl ExecRecorder {
    /// Create a new exec recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl ExecMiddleware for ExecRecorder {
    async fn run(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError> {
        // Build record context from the dispatch context — accurate correlation without OTel.
        let record_ctx = RecordContext {
            trace_id: ctx.trace.trace_id.clone(),
            operator_id: ctx.operator_id.to_string(),
            dispatch_id: ctx.dispatch_id.to_string(),
        };
        // Pre-phase: serialize OperatorInput as payload.
        let payload = serde_json::to_value(&input)
            .unwrap_or_else(|e| serde_json::json!({"serialize_error": e.to_string()}));

        self.sink
            .record(RecordEntry::pre(
                Boundary::Exec,
                record_ctx.clone(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.run(ctx, input, spec).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let post_payload = match &result {
            Ok(output) => serde_json::to_value(output).unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        };
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::Exec,
                record_ctx,
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
    use layer0::ExitReason;
    use layer0::content::Content;
    use layer0::operator::TriggerType;

    struct EchoExec;

    #[async_trait]
    impl ExecNext for EchoExec {
        async fn run(
            &self,
            _ctx: &DispatchContext,
            input: OperatorInput,
            _spec: &EnvironmentSpec,
        ) -> Result<OperatorOutput, EnvError> {
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    #[tokio::test]
    async fn exec_recorder_captures_entry() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = ExecRecorder::new(sink.clone());

        let input = OperatorInput::new(Content::text("run this"), TriggerType::User);
        let spec = EnvironmentSpec::default();

        let result = recorder.run(&DispatchContext::new(layer0::id::DispatchId::new("test"), layer0::id::OperatorId::new("test")), input, &spec, &EchoExec).await;
        assert!(result.is_ok());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post entries");

        let pre = &entries[0];
        assert_eq!(pre.phase, Phase::Pre);
        assert_eq!(pre.boundary, Boundary::Exec);
        assert!(pre.duration_ms.is_none());
        assert!(pre.error.is_none());

        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::Exec);
        assert!(post.duration_ms.is_some());
        assert!(post.error.is_none());
        // Post payload should contain the serialized OperatorOutput.
        assert!(!post.payload_json.is_null());
        assert!(
            post.payload_json["exit_reason"].is_string()
                || post.payload_json["exit_reason"].is_object()
        );
    }

    #[tokio::test]
    async fn exec_recorder_captures_error() {
        struct FailExec;

        #[async_trait]
        impl ExecNext for FailExec {
            async fn run(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
                _spec: &EnvironmentSpec,
            ) -> Result<OperatorOutput, EnvError> {
                Err(EnvError::ProvisionFailed("sandbox exploded".into()))
            }
        }

        let sink = Arc::new(InMemorySink::new());
        let recorder = ExecRecorder::new(sink.clone());

        let input = OperatorInput::new(Content::text("fail"), TriggerType::User);
        let spec = EnvironmentSpec::default();

        let result = recorder.run(&DispatchContext::new(layer0::id::DispatchId::new("test"), layer0::id::OperatorId::new("test")), input, &spec, &FailExec).await;
        assert!(result.is_err());

        let entries = sink.entries().await;
        let post = &entries[1];
        assert!(post.error.is_some());
    }
}
