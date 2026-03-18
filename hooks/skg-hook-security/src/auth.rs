//! Inbound authentication primitives for HTTP-serving deployments.
//!
//! These types validate incoming bearer tokens or API keys at the dispatch level,
//! independent of any HTTP framework. [`AuthGuard`] extracts a token from a raw
//! `Authorization` header value and delegates to a [`TokenValidator`].

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use thiserror::Error;

// ---------------------------------------------------------------------------
// AuthIdentity
// ---------------------------------------------------------------------------

/// Identity established after successful token validation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthIdentity {
    /// Unique identifier for the authenticated principal (user-id, service name, etc.).
    pub id: String,
    /// Permission scopes granted to this identity.
    pub scopes: Vec<String>,
    /// Arbitrary key-value metadata attached by the validator.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// AuthError
// ---------------------------------------------------------------------------

/// Errors produced during token validation.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum AuthError {
    /// The token is syntactically invalid or not recognized.
    #[error("invalid token")]
    InvalidToken,
    /// The token has expired.
    #[error("token expired")]
    Expired,
    /// The token is valid but lacks a required scope.
    #[error("insufficient scope: {required} is required")]
    InsufficientScope {
        /// The scope that was required but not present.
        required: String,
    },
    /// An internal error during validation (backend unavailable, etc.).
    #[error("internal auth error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// TokenValidator
// ---------------------------------------------------------------------------

/// Async trait for validating bearer tokens or API keys.
///
/// Implementations may hit a database, call an external IdP, or check a static
/// lookup table. The validator receives the raw token (without the `Bearer `
/// scheme prefix) and returns an [`AuthIdentity`] on success.
#[async_trait]
pub trait TokenValidator: Send + Sync + fmt::Debug {
    /// Validate `token` and return the associated identity.
    async fn validate(&self, token: &str) -> Result<AuthIdentity, AuthError>;
}

// ---------------------------------------------------------------------------
// AuthGuard
// ---------------------------------------------------------------------------

/// Extracts a bearer token from a raw `Authorization` header value and
/// delegates validation to a [`TokenValidator`].
///
/// Framework-agnostic: accepts a `&str` header value, not an HTTP request.
#[derive(Debug)]
pub struct AuthGuard {
    validator: Box<dyn TokenValidator>,
}

impl AuthGuard {
    /// Create a new guard backed by the given validator.
    pub fn new(validator: Box<dyn TokenValidator>) -> Self {
        Self { validator }
    }

    /// Parse the `Authorization` header value and validate the token.
    ///
    /// Accepts `Bearer <token>` (case-insensitive scheme). Returns
    /// [`AuthError::InvalidToken`] for missing, malformed, or empty tokens.
    pub async fn authenticate(&self, header_value: &str) -> Result<AuthIdentity, AuthError> {
        let token = Self::extract_bearer_token(header_value)?;
        self.validator.validate(token).await
    }

    /// Extract the token portion from a `Bearer <token>` header value.
    fn extract_bearer_token(header_value: &str) -> Result<&str, AuthError> {
        let trimmed = header_value.trim();
        // RFC 7235: scheme is case-insensitive.
        if trimmed.len() < 7 || !trimmed[..7].eq_ignore_ascii_case("bearer ") {
            return Err(AuthError::InvalidToken);
        }
        let token = trimmed[7..].trim();
        if token.is_empty() {
            return Err(AuthError::InvalidToken);
        }
        Ok(token)
    }
}

// ---------------------------------------------------------------------------
// StaticKeyValidator
// ---------------------------------------------------------------------------

/// A [`TokenValidator`] that checks tokens against a fixed set of known API keys.
///
/// Useful for development, testing, or simple deployments where keys are
/// configured at startup.
#[derive(Clone)]
pub struct StaticKeyValidator {
    keys: HashMap<String, AuthIdentity>,
}

impl StaticKeyValidator {
    /// Create a validator from a map of `token → identity`.
    pub fn new(keys: HashMap<String, AuthIdentity>) -> Self {
        Self { keys }
    }

    /// Create an empty validator (all tokens will be rejected).
    pub fn empty() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    /// Add a key-identity mapping. Returns `self` for chaining.
    pub fn with_key(mut self, token: impl Into<String>, identity: AuthIdentity) -> Self {
        self.keys.insert(token.into(), identity);
        self
    }
}

#[async_trait]
impl TokenValidator for StaticKeyValidator {
    async fn validate(&self, token: &str) -> Result<AuthIdentity, AuthError> {
        self.keys.get(token).cloned().ok_or(AuthError::InvalidToken)
    }
}

impl fmt::Debug for StaticKeyValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Deliberately omit key contents — they are secrets.
        // Only the count is printed so debug output remains diagnostic.
        f.debug_struct("StaticKeyValidator")
            .field("keys", &format_args!("<{} keys>", self.keys.len()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity(id: &str) -> AuthIdentity {
        AuthIdentity {
            id: id.to_owned(),
            scopes: vec!["read".to_owned()],
            metadata: HashMap::new(),
        }
    }

    fn test_validator() -> StaticKeyValidator {
        StaticKeyValidator::new(HashMap::new())
            .with_key("valid-key-1", test_identity("user-1"))
            .with_key("valid-key-2", test_identity("user-2"))
    }

    // -- StaticKeyValidator -------------------------------------------------

    #[tokio::test]
    async fn static_validator_accepts_known_key() {
        let v = test_validator();
        let id = v.validate("valid-key-1").await.unwrap();
        assert_eq!(id.id, "user-1");
        assert_eq!(id.scopes, vec!["read"]);
    }

    #[tokio::test]
    async fn static_validator_rejects_unknown_key() {
        let v = test_validator();
        let err = v.validate("bad-key").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn static_validator_empty_rejects_all() {
        let v = StaticKeyValidator::empty();
        assert!(v.validate("anything").await.is_err());
    }

    #[test]
    fn static_validator_debug_redacts_keys() {
        let v = test_validator();
        let s = format!("{v:?}");
        // Must show key count for diagnostics.
        assert!(s.contains("<2 keys>"), "expected count in debug: {s}");
        // Must not leak any actual key material.
        assert!(!s.contains("valid-key-1"), "key material leaked in debug: {s}");
        assert!(!s.contains("valid-key-2"), "key material leaked in debug: {s}");
    }

    // -- AuthGuard header parsing -------------------------------------------

    #[tokio::test]
    async fn guard_extracts_bearer_token() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let id = guard.authenticate("Bearer valid-key-1").await.unwrap();
        assert_eq!(id.id, "user-1");
    }

    #[tokio::test]
    async fn guard_case_insensitive_scheme() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let id = guard.authenticate("bearer valid-key-1").await.unwrap();
        assert_eq!(id.id, "user-1");

        let id = guard.authenticate("BEARER valid-key-2").await.unwrap();
        assert_eq!(id.id, "user-2");
    }

    #[tokio::test]
    async fn guard_trims_whitespace() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let id = guard
            .authenticate("  Bearer   valid-key-1  ")
            .await
            .unwrap();
        assert_eq!(id.id, "user-1");
    }

    #[tokio::test]
    async fn guard_rejects_missing_scheme() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let err = guard.authenticate("valid-key-1").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn guard_rejects_wrong_scheme() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let err = guard.authenticate("Basic dXNlcjpwYXNz").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn guard_rejects_empty_token() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let err = guard.authenticate("Bearer ").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn guard_rejects_empty_string() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let err = guard.authenticate("").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn guard_rejects_bearer_only_whitespace() {
        let guard = AuthGuard::new(Box::new(test_validator()));
        let err = guard.authenticate("Bearer    ").await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    // -- AuthError Display --------------------------------------------------

    #[test]
    fn auth_error_display() {
        assert_eq!(AuthError::InvalidToken.to_string(), "invalid token");
        assert_eq!(AuthError::Expired.to_string(), "token expired");
        assert_eq!(
            AuthError::InsufficientScope {
                required: "admin".into()
            }
            .to_string(),
            "insufficient scope: admin is required"
        );
        assert_eq!(
            AuthError::Internal("db down".into()).to_string(),
            "internal auth error: db down"
        );
    }
}
