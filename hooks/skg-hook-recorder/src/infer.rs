//! [`InferRecorder`] — records inference operations via [`InferMiddleware`].

use crate::{Boundary, RecordEntry, RecordSink, context_from_otel};
use async_trait::async_trait;
use skg_turn::infer_middleware::{InferMiddleware, InferNext};
use skg_turn::provider::ProviderError;
use skg_turn::{InferRequest, InferResponse};
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// INFER RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every inference operation to a [`RecordSink`].
///
/// Captures two entries per inference call:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with the serialized [`InferRequest`]
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
///
/// Uses [`Boundary::Infer`] and [`context_from_otel`] to extract trace context
/// from the ambient OTel span when available, falling back to empty context.
pub struct InferRecorder {
    sink: Arc<dyn RecordSink>,
}

impl InferRecorder {
    /// Create a new infer recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl InferMiddleware for InferRecorder {
    async fn infer(
        &self,
        request: InferRequest,
        next: &dyn InferNext,
    ) -> Result<InferResponse, ProviderError> {
        // Pre-phase: serialize InferRequest to JSON payload.
        let payload = serde_json::to_value(InferRequestSummary::from(&request))
            .unwrap_or_else(|e| serde_json::json!({"serialize_error": e.to_string()}));

        self.sink
            .record(RecordEntry::pre(
                Boundary::Infer,
                context_from_otel(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.infer(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Post-phase: record outcome, including the full InferResponse on success.
        let post_payload = match &result {
            Ok(response) => serde_json::to_value(response).unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        };
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::Infer,
                context_from_otel(),
                post_payload,
                duration_ms,
                error,
            ))
            .await;

        result
    }
}

/// A serializable summary of an [`InferRequest`] for recording purposes.
///
/// [`InferRequest`] itself is not `Serialize` because it contains layer0 types
/// that may not implement `Serialize`. This summary captures the key fields.
#[derive(serde::Serialize)]
struct InferRequestSummary<'a> {
    model: Option<&'a str>,
    message_count: usize,
    tool_count: usize,
    max_tokens: Option<u32>,
    system: Option<&'a str>,
}

impl<'a> From<&'a InferRequest> for InferRequestSummary<'a> {
    fn from(req: &'a InferRequest) -> Self {
        Self {
            model: req.model.as_deref(),
            message_count: req.messages.len(),
            tool_count: req.tools.len(),
            max_tokens: req.max_tokens,
            system: req.system.as_deref(),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Boundary, InMemorySink, Phase};
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use skg_turn::types::{StopReason, TokenUsage};

    fn make_infer_request() -> InferRequest {
        InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
    }

    fn make_infer_response() -> InferResponse {
        InferResponse {
            content: Content::text("response"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test-model".into(),
            cost: None,
            truncated: None,
        }
    }

    struct EchoNext;

    #[async_trait]
    impl InferNext for EchoNext {
        async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
            Ok(make_infer_response())
        }
    }

    #[tokio::test]
    async fn infer_recorder_captures_pre_and_post() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = InferRecorder::new(sink.clone());

        let request = make_infer_request();
        let result = recorder.infer(request, &EchoNext).await;
        assert!(result.is_ok());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post entries");

        let pre = &entries[0];
        assert_eq!(pre.phase, Phase::Pre);
        assert_eq!(pre.boundary, Boundary::Infer);
        assert!(pre.duration_ms.is_none());
        assert!(pre.error.is_none());

        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::Infer);
        assert!(post.duration_ms.is_some());
        assert!(post.error.is_none());
        // Post payload should contain the serialized InferResponse.
        assert!(!post.payload_json.is_null());
        assert_eq!(post.payload_json["model"], "test-model");
    }

    #[tokio::test]
    async fn infer_recorder_captures_error() {
        struct FailNext;

        #[async_trait]
        impl InferNext for FailNext {
            async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
                Err(ProviderError::ContentBlocked {
                    message: "blocked".into(),
                })
            }
        }

        let sink = Arc::new(InMemorySink::new());
        let recorder = InferRecorder::new(sink.clone());

        let result = recorder.infer(make_infer_request(), &FailNext).await;
        assert!(result.is_err());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert!(post.error.is_some());
    }
}
