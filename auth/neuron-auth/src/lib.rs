#![deny(missing_docs)]
//! Authentication providers for neuron.
//!
//! This crate defines the [`AuthProvider`] trait for obtaining authentication
//! credentials to access secret backends. It also provides [`AuthProviderChain`]
//! for composing multiple providers (try in order until one succeeds, like
//! AWS DefaultCredentialsChain).
//!
//! ## Separation of Concerns
//!
//! Auth providers produce credentials (tokens). Secret resolvers consume them.
//! A `VaultResolver` takes an `Arc<dyn AuthProvider>` and uses it to authenticate
//! before fetching secrets. This separation follows the pattern established by
//! AWS SDK (`ProvideCredentials` vs `SecretsManagerClient`), vaultrs
//! (`auth::*` vs `kv2::*`), and Google Cloud SDK.

use async_trait::async_trait;
use neuron_secret::SecretValue;
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

/// Errors from authentication providers (crate-local, not in layer0).
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum AuthError {
    /// Authentication failed (bad credentials, expired token, etc.).
    #[error("auth failed: {0}")]
    AuthFailed(String),

    /// The requested scope or audience is not available.
    #[error("scope unavailable: {0}")]
    ScopeUnavailable(String),

    /// Backend communication failure.
    #[error("backend error: {0}")]
    BackendError(String),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Context for an authentication request.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct AuthRequest {
    /// Target audience (OIDC audience, API identifier).
    pub audience: Option<String>,
    /// Requested scopes (OIDC scopes, OAuth2 scopes).
    pub scopes: Vec<String>,
    /// Target resource identifier (e.g., Vault path, AWS region).
    pub resource: Option<String>,
    /// Actor identity for audit (workflow ID, agent ID).
    pub actor: Option<String>,
}

impl AuthRequest {
    /// Create an empty auth request (no specific context).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target audience.
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Add a scope.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scopes.push(scope.into());
        self
    }

    /// Set the target resource.
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Set the actor identity.
    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }
}

/// An opaque authentication token with expiry.
/// Uses [`SecretValue`] internally for in-memory protection.
pub struct AuthToken {
    inner: SecretValue,
    expires_at: Option<SystemTime>,
}

impl AuthToken {
    /// Create a new auth token.
    pub fn new(bytes: Vec<u8>, expires_at: Option<SystemTime>) -> Self {
        Self {
            inner: SecretValue::new(bytes),
            expires_at,
        }
    }

    /// Create a token that never expires (for dev/test).
    pub fn permanent(bytes: Vec<u8>) -> Self {
        Self::new(bytes, None)
    }

    /// Scoped exposure of the token bytes.
    pub fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        self.inner.with_bytes(f)
    }

    /// Check if this token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| SystemTime::now() > exp)
            .unwrap_or(false)
    }

    /// Returns when this token expires, if known.
    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

impl std::fmt::Debug for AuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthToken")
            .field("value", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Provide authentication credentials for accessing a secret backend.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Provide an authentication token for the given request context.
    async fn provide(&self, request: &AuthRequest) -> Result<AuthToken, AuthError>;
}

/// Tries providers in order until one succeeds.
pub struct AuthProviderChain {
    providers: Vec<Arc<dyn AuthProvider>>,
}

impl AuthProviderChain {
    /// Create a new empty chain.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a provider to the end of the chain.
    pub fn with_provider(mut self, provider: Arc<dyn AuthProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Add a provider to the end of the chain (mutable).
    pub fn add(&mut self, provider: Arc<dyn AuthProvider>) {
        self.providers.push(provider);
    }
}

impl Default for AuthProviderChain {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthProvider for AuthProviderChain {
    async fn provide(&self, request: &AuthRequest) -> Result<AuthToken, AuthError> {
        let mut last_err = None;
        for provider in &self.providers {
            match provider.provide(request).await {
                Ok(token) => return Ok(token),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AuthError::AuthFailed("no providers configured".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn auth_provider_is_object_safe_send_sync() {
        _assert_send_sync::<Box<dyn AuthProvider>>();
        _assert_send_sync::<Arc<dyn AuthProvider>>();
    }

    #[test]
    fn auth_token_debug_is_redacted() {
        let token = AuthToken::permanent(b"secret-token".to_vec());
        let debug = format!("{:?}", token);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret-token"));
    }

    #[test]
    fn auth_token_with_bytes_exposes_content() {
        let token = AuthToken::permanent(b"my-token".to_vec());
        token.with_bytes(|bytes| {
            assert_eq!(bytes, b"my-token");
        });
    }

    #[test]
    fn auth_token_permanent_never_expires() {
        let token = AuthToken::permanent(b"token".to_vec());
        assert!(!token.is_expired());
        assert!(token.expires_at().is_none());
    }

    #[test]
    fn auth_request_builder() {
        let req = AuthRequest::new()
            .with_audience("https://vault.internal")
            .with_scope("read:secrets")
            .with_scope("write:audit")
            .with_resource("secret/data/api-key")
            .with_actor("workflow-001");
        assert_eq!(req.audience.as_deref(), Some("https://vault.internal"));
        assert_eq!(req.scopes.len(), 2);
        assert_eq!(req.resource.as_deref(), Some("secret/data/api-key"));
        assert_eq!(req.actor.as_deref(), Some("workflow-001"));
    }

    struct AlwaysFailProvider;
    #[async_trait]
    impl AuthProvider for AlwaysFailProvider {
        async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
            Err(AuthError::AuthFailed("always fails".into()))
        }
    }

    struct StaticTokenProvider {
        token: Vec<u8>,
    }
    #[async_trait]
    impl AuthProvider for StaticTokenProvider {
        async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
            Ok(AuthToken::permanent(self.token.clone()))
        }
    }

    #[tokio::test]
    async fn chain_empty_returns_error() {
        let chain = AuthProviderChain::new();
        assert!(chain.provide(&AuthRequest::new()).await.is_err());
    }

    #[tokio::test]
    async fn chain_first_success_wins() {
        let chain = AuthProviderChain::new()
            .with_provider(Arc::new(StaticTokenProvider {
                token: b"first".to_vec(),
            }))
            .with_provider(Arc::new(StaticTokenProvider {
                token: b"second".to_vec(),
            }));
        let token = chain.provide(&AuthRequest::new()).await.unwrap();
        token.with_bytes(|b| assert_eq!(b, b"first"));
    }

    #[tokio::test]
    async fn chain_skips_failures() {
        let chain = AuthProviderChain::new()
            .with_provider(Arc::new(AlwaysFailProvider))
            .with_provider(Arc::new(StaticTokenProvider {
                token: b"fallback".to_vec(),
            }));
        let token = chain.provide(&AuthRequest::new()).await.unwrap();
        token.with_bytes(|b| assert_eq!(b, b"fallback"));
    }

    #[tokio::test]
    async fn chain_all_fail_returns_last_error() {
        let chain = AuthProviderChain::new()
            .with_provider(Arc::new(AlwaysFailProvider))
            .with_provider(Arc::new(AlwaysFailProvider));
        let result = chain.provide(&AuthRequest::new()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "auth failed: always fails");
    }

    #[test]
    fn auth_error_display_all_variants() {
        assert_eq!(
            AuthError::AuthFailed("bad token".into()).to_string(),
            "auth failed: bad token"
        );
        assert_eq!(
            AuthError::ScopeUnavailable("admin".into()).to_string(),
            "scope unavailable: admin"
        );
        assert_eq!(
            AuthError::BackendError("connection refused".into()).to_string(),
            "backend error: connection refused"
        );
    }
}
