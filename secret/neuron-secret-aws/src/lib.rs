#![deny(missing_docs)]
//! Stub secret resolver for AWS Secrets Manager.
//!
//! This crate provides the correct trait impl shape for an AWS Secrets Manager resolver.
//! The actual AWS SDK integration is not implemented — all resolve calls return
//! `SecretError::BackendError`.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use neuron_auth::AuthProvider;
use neuron_secret::{SecretError, SecretLease, SecretResolver};
use std::sync::Arc;

/// Stub resolver for AWS Secrets Manager.
pub struct AwsResolver {
    _auth: Arc<dyn AuthProvider>,
}

impl AwsResolver {
    /// Create a new AWS resolver (stub).
    pub fn new(auth: Arc<dyn AuthProvider>) -> Self {
        Self { _auth: auth }
    }
}

#[async_trait]
impl SecretResolver for AwsResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        match source {
            SecretSource::AwsSecretsManager { secret_id, region } => {
                Err(SecretError::BackendError(format!(
                    "AwsResolver is a stub — would resolve {secret_id} in region {region:?}"
                )))
            }
            _ => Err(SecretError::NoResolver("aws".into())),
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
        let _: Arc<dyn SecretResolver> = Arc::new(AwsResolver::new(auth));
    }

    #[tokio::test]
    async fn matches_aws_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = AwsResolver::new(auth);
        let source = SecretSource::AwsSecretsManager {
            secret_id: "my-secret".into(),
            region: Some("us-east-1".into()),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn rejects_wrong_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = AwsResolver::new(auth);
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }
}
