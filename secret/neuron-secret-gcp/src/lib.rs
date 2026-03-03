#![deny(missing_docs)]
//! Stub secret resolver for GCP Secret Manager.
//!
//! This crate provides the correct trait impl shape for a GCP Secret Manager resolver.
//! The actual GCP SDK integration is not implemented — all resolve calls return
//! `SecretError::BackendError`.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use neuron_auth::AuthProvider;
use neuron_secret::{SecretError, SecretLease, SecretResolver};
use std::sync::Arc;

/// Stub resolver for GCP Secret Manager.
pub struct GcpResolver {
    _auth: Arc<dyn AuthProvider>,
}

impl GcpResolver {
    /// Create a new GCP resolver (stub).
    pub fn new(auth: Arc<dyn AuthProvider>) -> Self {
        Self { _auth: auth }
    }
}

#[async_trait]
impl SecretResolver for GcpResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        match source {
            SecretSource::GcpSecretManager { project, secret_id } => {
                Err(SecretError::BackendError(format!(
                    "GcpResolver is a stub — would resolve {project}/{secret_id}"
                )))
            }
            _ => Err(SecretError::NoResolver("gcp".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_auth::{AuthError, AuthRequest, AuthToken};

    struct StubAuth;
    #[async_trait]
    impl AuthProvider for StubAuth {
        async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
            Ok(AuthToken::permanent(b"stub".to_vec()))
        }
    }

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn object_safety() {
        _assert_send_sync::<Box<dyn SecretResolver>>();
        _assert_send_sync::<Arc<dyn SecretResolver>>();
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let _: Arc<dyn SecretResolver> = Arc::new(GcpResolver::new(auth));
    }

    #[tokio::test]
    async fn matches_gcp_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = GcpResolver::new(auth);
        let source = SecretSource::GcpSecretManager {
            project: "my-project".into(),
            secret_id: "api-key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn rejects_wrong_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = GcpResolver::new(auth);
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }
}
