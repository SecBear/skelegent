//! [`SecretRecorder`] — records secret resolution operations via [`SecretMiddleware`].

use crate::{Boundary, RecordEntry, RecordSink, context_from_otel};
use async_trait::async_trait;
use layer0::secret::SecretSource;
use skg_secret::{SecretError, SecretLease, SecretMiddleware, SecretNext};
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SECRET RECORDER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware that records every secret resolution operation to a [`RecordSink`].
///
/// Captures two entries per resolution:
/// - [`Phase::Pre`](crate::Phase::Pre) — before calling `next`, with the serialized [`SecretSource`]
/// - [`Phase::Post`](crate::Phase::Post) — after `next` returns, with duration and any error
///
/// The resolved [`SecretLease`] value is intentionally **not** recorded in the payload
/// (it carries no `Serialize` impl and must not appear in logs). Post payload records only
/// observable lease metadata: `renewable` and `has_expiry`.
///
/// Uses [`Boundary::Secret`] and [`context_from_otel`] to extract trace context
/// from the ambient OTel span when available, falling back to empty context.
pub struct SecretRecorder {
    sink: Arc<dyn RecordSink>,
}

impl SecretRecorder {
    /// Create a new secret recorder that sends entries to `sink`.
    pub fn new(sink: Arc<dyn RecordSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl SecretMiddleware for SecretRecorder {
    async fn resolve(
        &self,
        source: &SecretSource,
        next: &dyn SecretNext,
    ) -> Result<SecretLease, SecretError> {
        // Pre-phase: record the SecretSource (where the secret lives, not what it is).
        let pre_payload = serde_json::to_value(source)
            .unwrap_or_else(|e| serde_json::json!({"serialize_error": e.to_string()}));

        self.sink
            .record(RecordEntry::pre(
                Boundary::Secret,
                context_from_otel(),
                pre_payload,
            ))
            .await;

        let start = Instant::now();
        let result = next.resolve(source).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let (post_payload, error) = match &result {
            Ok(lease) => {
                // Record only non-secret lease metadata — never the value itself.
                let payload = serde_json::json!({
                    "status": "ok",
                    "renewable": lease.renewable,
                    "has_expiry": lease.expires_at.is_some(),
                });
                (payload, None)
            }
            Err(e) => (
                serde_json::json!({"error": e.to_string()}),
                Some(e.to_string()),
            ),
        };

        self.sink
            .record(RecordEntry::post(
                Boundary::Secret,
                context_from_otel(),
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
    use crate::{InMemorySink, Phase};
    use skg_secret::SecretValue;

    fn vault_source() -> SecretSource {
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/api-keys/test".into(),
        }
    }

    struct OkNext;

    #[async_trait]
    impl SecretNext for OkNext {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            Ok(SecretLease {
                value: SecretValue::new(b"s3cr3t".to_vec()),
                expires_at: None,
                renewable: false,
                lease_id: None,
            })
        }
    }

    struct ErrNext;

    #[async_trait]
    impl SecretNext for ErrNext {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            Err(SecretError::NotFound("test-key".into()))
        }
    }

    #[tokio::test]
    async fn secret_recorder_captures_pre_and_post_on_success() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = SecretRecorder::new(sink.clone());
        let source = vault_source();

        let result = recorder.resolve(&source, &OkNext).await;
        assert!(result.is_ok());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2, "expected pre + post");

        assert_eq!(entries[0].phase, Phase::Pre);
        assert_eq!(entries[0].boundary, Boundary::Secret);
        assert!(entries[0].duration_ms.is_none());
        assert!(entries[0].error.is_none());
        // Pre payload must describe the source, not any secret value.
        assert_eq!(entries[0].payload_json["type"], "vault");

        assert_eq!(entries[1].phase, Phase::Post);
        assert_eq!(entries[1].boundary, Boundary::Secret);
        assert!(entries[1].duration_ms.is_some());
        assert!(entries[1].error.is_none());
        assert_eq!(entries[1].payload_json["status"], "ok");
        // Secret value must NOT appear anywhere in the payload.
        assert!(
            entries[1].payload_json.get("value").is_none(),
            "secret value must never be recorded"
        );
    }

    #[tokio::test]
    async fn secret_recorder_captures_error() {
        let sink = Arc::new(InMemorySink::new());
        let recorder = SecretRecorder::new(sink.clone());
        let source = vault_source();

        let result = recorder.resolve(&source, &ErrNext).await;
        assert!(result.is_err());

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        let post = &entries[1];
        assert_eq!(post.phase, Phase::Post);
        assert!(post.error.is_some());
        assert!(post.payload_json.get("error").is_some());
        assert!(post.duration_ms.is_some());
    }
}
