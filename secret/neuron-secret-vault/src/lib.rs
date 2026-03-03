#![deny(missing_docs)]
//! Stub secret resolver for HashiCorp Vault KV.
//!
//! This crate provides the correct trait impl shape for a Vault secret resolver.
//! The actual Vault SDK integration is not implemented — all resolve calls return
//! `SecretError::BackendError`.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use neuron_auth::AuthProvider;
use neuron_secret::{SecretError, SecretLease, SecretResolver};
use std::sync::Arc;

/// Stub resolver for HashiCorp Vault KV.
pub struct VaultResolver {
    _addr: String,
    _auth: Arc<dyn AuthProvider>,
}

impl VaultResolver {
    /// Create a new Vault resolver (stub).
    pub fn new(addr: impl Into<String>, auth: Arc<dyn AuthProvider>) -> Self {
        Self {
            _addr: addr.into(),
            _auth: auth,
        }
    }
}

#[async_trait]
impl SecretResolver for VaultResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        match source {
            SecretSource::Vault { mount, path } => Err(SecretError::BackendError(format!(
                "VaultResolver is a stub — would resolve {mount}/{path}"
            ))),
            _ => Err(SecretError::NoResolver("vault".into())),
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
        let resolver = VaultResolver::new("https://vault:8200", auth);
        let _: Box<dyn SecretResolver> = Box::new(resolver);
    }

    #[tokio::test]
    async fn matches_vault_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = VaultResolver::new("https://vault:8200", auth);
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn rejects_wrong_source() {
        let auth: Arc<dyn AuthProvider> = Arc::new(StubAuth);
        let resolver = VaultResolver::new("https://vault:8200", auth);
        let source = SecretSource::OsKeystore {
            service: "test".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }
}
