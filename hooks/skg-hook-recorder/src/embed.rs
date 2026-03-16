//! [`EmbedRecorder`] — records embedding operations via [`EmbedMiddleware`].

use crate::{Boundary, RecordContext, RecordEntry, RecordSink};
use async_trait::async_trait;
use skg_turn::embedding::{EmbedRequest, EmbedResponse};
use skg_turn::infer_middleware::{EmbedMiddleware, EmbedNext};
use skg_turn::provider::ProviderError;
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EMBED RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every embedding operation to a [`RecordSink`].
///
/// Captures two entries per embed call:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with the serialized [`EmbedRequest`]
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
///
/// Uses [`Boundary::Embed`] and [`RecordContext::empty`] since `EmbedMiddleware`
/// does not receive a dispatch context.
pub struct EmbedRecorder {
    sink: Arc<dyn RecordSink>,
}

impl EmbedRecorder {
    /// Create a new embed recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl EmbedMiddleware for EmbedRecorder {
    async fn embed(
        &self,
        request: EmbedRequest,
        next: &dyn EmbedNext,
    ) -> Result<EmbedResponse, ProviderError> {
        // Pre-phase: serialize EmbedRequest to JSON payload.
        let payload = serde_json::to_value(&request)
            .unwrap_or_else(|e| serde_json::json!({"serialize_error": e.to_string()}));

        self.sink
            .record(RecordEntry::pre(
                Boundary::Embed,
                RecordContext::empty(),
                payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.embed(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Post-phase: record outcome, including the full EmbedResponse on success.
        let post_payload = match &result {
            Ok(response) => serde_json::to_value(response).unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        };
        let error = result.as_ref().err().map(|e| e.to_string());
        self.sink
            .record(RecordEntry::post(
                Boundary::Embed,
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
    use skg_turn::embedding::Embedding;
    use skg_turn::types::TokenUsage;

    fn make_embed_request() -> EmbedRequest {
        EmbedRequest::new(vec!["hello".into(), "world".into()])
    }

    fn make_embed_response() -> EmbedResponse {
        EmbedResponse {
            embeddings: vec![
                Embedding {
                    vector: vec![0.1, 0.2, 0.3],
                },
                Embedding {
                    vector: vec![0.4, 0.5, 0.6],
                },
            ],
            model: "test-model".into(),
            usage: TokenUsage::default(),
        }
    }

    struct EchoNext;

    #[async_trait]
    impl EmbedNext for EchoNext {
        async fn embed(&self, _request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
            Ok(make_embed_response())
        }
    }

    #[tokio::test]
    async fn embed_recorder_captures_pre_and_post() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = EmbedRecorder::new(sink.clone());

        let request = make_embed_request();
        let result = recorder.embed(request, &EchoNext).await;
        assert!(result.is_ok());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post entries");

        let pre = &entries[0];
        assert_eq!(pre.phase, Phase::Pre);
        assert_eq!(pre.boundary, Boundary::Embed);
        assert!(pre.duration_ms.is_none());
        assert!(pre.error.is_none());

        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert_eq!(post.boundary, Boundary::Embed);
        assert!(post.duration_ms.is_some());
        assert!(post.error.is_none());
        // Post payload should contain the serialized EmbedResponse.
        assert!(!post.payload_json.is_null());
        assert_eq!(post.payload_json["model"], "test-model");
    }

    #[tokio::test]
    async fn embed_recorder_captures_error() {
        struct FailNext;

        #[async_trait]
        impl EmbedNext for FailNext {
            async fn embed(&self, _request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
                Err(ProviderError::ContentBlocked {
                    message: "blocked".into(),
                })
            }
        }

        let sink = Arc::new(InMemorySink::new());
        let recorder = EmbedRecorder::new(sink.clone());

        let result = recorder.embed(make_embed_request(), &FailNext).await;
        assert!(result.is_err());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert!(post.error.is_some());
    }
}
