#![deny(missing_docs)]
//! Static auth provider that always returns the same token.
//!
//! Intended for dev/test use only. Not suitable for production.

use async_trait::async_trait;
use neuron_auth::{AuthError, AuthProvider, AuthRequest, AuthToken};

/// A static auth provider that always returns the same token. Dev/test only.
pub struct StaticAuthProvider {
    token: Vec<u8>,
}

impl StaticAuthProvider {
    /// Create with a fixed token.
    pub fn new(token: impl Into<Vec<u8>>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl AuthProvider for StaticAuthProvider {
    async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
        Ok(AuthToken::permanent(self.token.clone()))
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
        let _: Arc<dyn AuthProvider> = Arc::new(StaticAuthProvider::new(b"token".to_vec()));
    }

    #[tokio::test]
    async fn returns_fixed_token() {
        let provider = StaticAuthProvider::new(b"my-secret-token".to_vec());
        let token = provider.provide(&AuthRequest::new()).await.unwrap();
        token.with_bytes(|b| assert_eq!(b, b"my-secret-token"));
    }

    #[tokio::test]
    async fn returns_same_token_every_time() {
        let provider = StaticAuthProvider::new(b"fixed".to_vec());
        let t1 = provider.provide(&AuthRequest::new()).await.unwrap();
        let t2 = provider.provide(&AuthRequest::new()).await.unwrap();
        t1.with_bytes(|b| assert_eq!(b, b"fixed"));
        t2.with_bytes(|b| assert_eq!(b, b"fixed"));
    }

    #[tokio::test]
    async fn ignores_request_context() {
        let provider = StaticAuthProvider::new(b"ignored".to_vec());
        let req = AuthRequest::new()
            .with_audience("https://vault.internal")
            .with_scope("admin");
        let token = provider.provide(&req).await.unwrap();
        token.with_bytes(|b| assert_eq!(b, b"ignored"));
    }
}
