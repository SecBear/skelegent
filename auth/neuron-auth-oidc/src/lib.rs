#![deny(missing_docs)]
//! Stub OIDC client credentials / token exchange auth provider.
//!
//! This crate provides the correct trait impl shape for an OIDC auth provider.
//! The actual OIDC integration is not implemented — all provide calls return
//! `AuthError::BackendError`.

use async_trait::async_trait;
use neuron_auth::{AuthError, AuthProvider, AuthRequest, AuthToken};

/// Stub for OIDC client credentials / token exchange.
pub struct OidcAuthProvider {
    _issuer_url: String,
}

impl OidcAuthProvider {
    /// Create a new OIDC auth provider (stub).
    pub fn new(issuer_url: impl Into<String>) -> Self {
        Self {
            _issuer_url: issuer_url.into(),
        }
    }
}

#[async_trait]
impl AuthProvider for OidcAuthProvider {
    async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
        Err(AuthError::BackendError("OidcAuthProvider is a stub".into()))
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
        let _: Arc<dyn AuthProvider> =
            Arc::new(OidcAuthProvider::new("https://issuer.example.com"));
    }

    #[tokio::test]
    async fn returns_stub_error() {
        let provider = OidcAuthProvider::new("https://issuer.example.com");
        let err = provider.provide(&AuthRequest::new()).await.unwrap_err();
        assert!(matches!(err, AuthError::BackendError(_)));
        assert!(err.to_string().contains("stub"));
    }
}
