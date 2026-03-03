#![deny(missing_docs)]
//! Stub secret resolver for OS keystore (macOS Keychain, Windows DPAPI, Linux Secret Service).
//!
//! This crate provides the correct trait impl shape for an OS keystore resolver.
//! The actual platform keychain integration is not implemented — all resolve calls
//! return `SecretError::BackendError`.
//!
//! No auth dependency — the OS keystore uses system-level authentication.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use neuron_secret::{SecretError, SecretLease, SecretResolver};

/// Stub resolver for OS keystore.
pub struct KeystoreResolver;

impl KeystoreResolver {
    /// Create a new OS keystore resolver (stub).
    pub fn new() -> Self {
        Self
    }
}

impl Default for KeystoreResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretResolver for KeystoreResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        match source {
            SecretSource::OsKeystore { service } => Err(SecretError::BackendError(format!(
                "KeystoreResolver is a stub — would resolve service {service:?}"
            ))),
            _ => Err(SecretError::NoResolver("os_keystore".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn object_safety() {
        _assert_send_sync::<Box<dyn SecretResolver>>();
        _assert_send_sync::<Arc<dyn SecretResolver>>();
        let _: Arc<dyn SecretResolver> = Arc::new(KeystoreResolver::new());
    }

    #[tokio::test]
    async fn matches_keystore_source() {
        let resolver = KeystoreResolver::new();
        let source = SecretSource::OsKeystore {
            service: "my-app".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn rejects_wrong_source() {
        let resolver = KeystoreResolver::new();
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }
}
