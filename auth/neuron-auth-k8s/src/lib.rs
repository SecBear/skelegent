#![deny(missing_docs)]
//! Stub Kubernetes ServiceAccount projected token auth provider.
//!
//! This crate provides the correct trait impl shape for a K8s auth provider.
//! The actual K8s integration is not implemented — all provide calls return
//! `AuthError::BackendError`.

use async_trait::async_trait;
use neuron_auth::{AuthError, AuthProvider, AuthRequest, AuthToken};

/// Stub for K8s ServiceAccount projected token.
pub struct K8sAuthProvider {
    _namespace: String,
}

impl K8sAuthProvider {
    /// Create a new K8s auth provider (stub).
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            _namespace: namespace.into(),
        }
    }
}

#[async_trait]
impl AuthProvider for K8sAuthProvider {
    async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
        Err(AuthError::BackendError("K8sAuthProvider is a stub".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn object_safety() {
        _assert_send_sync::<Box<dyn AuthProvider>>();
        _assert_send_sync::<Arc<dyn AuthProvider>>();
        let _: Arc<dyn AuthProvider> = Arc::new(K8sAuthProvider::new("default"));
    }

    #[tokio::test]
    async fn returns_stub_error() {
        let provider = K8sAuthProvider::new("default");
        let err = provider.provide(&AuthRequest::new()).await.unwrap_err();
        assert!(matches!(err, AuthError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }
}
