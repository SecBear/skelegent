#![deny(missing_docs)]
//! Secret resolution for neuron.
//!
//! This crate defines the [`SecretResolver`] trait, the [`SecretValue`] in-memory
//! wrapper (no Serialize, no Display, no Clone — memory zeroed on drop), and the
//! [`SecretRegistry`] for composing multiple resolvers.
//!
//! ## Design
//!
//! - Resolvers resolve a [`SecretSource`] (from layer0), not a string name.
//!   The name->source mapping lives in `CredentialRef`.
//! - [`SecretValue`] uses scoped exposure (`with_bytes`) to prevent accidental leaks.
//! - [`SecretRegistry`] dispatches by [`SecretSource`] variant, following the same
//!   composition pattern as `ToolRegistry` and `HookRegistry`.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;
use zeroize::Zeroizing;

/// Errors from secret resolution (crate-local, not in layer0).
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum SecretError {
    /// The secret was not found in the backend.
    #[error("secret not found: {0}")]
    NotFound(String),

    /// Access denied by policy.
    #[error("access denied: {0}")]
    AccessDenied(String),

    /// Backend communication failure (network, timeout, etc.).
    #[error("backend error: {0}")]
    BackendError(String),

    /// The lease has expired and cannot be renewed.
    #[error("lease expired: {0}")]
    LeaseExpired(String),

    /// No resolver registered for this source type.
    /// The string is the source kind tag (from `SecretSource::kind()`).
    #[error("no resolver for source: {0}")]
    NoResolver(String),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// An opaque secret value. Cannot be logged, serialized, or cloned.
/// Memory is zeroed on drop via [`Zeroizing`].
///
/// The only way to access the bytes is through [`SecretValue::with_bytes`],
/// which enforces scoped exposure — the secret is only visible inside the closure.
pub struct SecretValue {
    inner: Zeroizing<Vec<u8>>,
}

impl SecretValue {
    /// Create a new secret value. The input vector is moved, not copied.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            inner: Zeroizing::new(bytes),
        }
    }

    /// Scoped exposure. The secret bytes are only accessible inside the closure.
    /// This is the ONLY way to read the value.
    pub fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(&self.inner)
    }

    /// Returns the length of the secret in bytes.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// Intentionally: no Display, no Clone, no Serialize, no PartialEq.

/// A resolved secret with optional lease information.
///
/// Leases allow time-bounded access to secrets. When a lease expires,
/// the secret must be re-resolved from the backend. Renewable leases
/// can be extended without re-authentication.
pub struct SecretLease {
    /// The resolved secret value.
    pub value: SecretValue,
    /// When this lease expires (None = no expiry).
    pub expires_at: Option<SystemTime>,
    /// Whether this lease can be renewed.
    pub renewable: bool,
    /// Opaque lease ID for renewal/revocation.
    pub lease_id: Option<String>,
}

impl SecretLease {
    /// Create a new lease with no expiry.
    pub fn permanent(value: SecretValue) -> Self {
        Self {
            value,
            expires_at: None,
            renewable: false,
            lease_id: None,
        }
    }

    /// Create a new lease with a TTL.
    pub fn with_ttl(value: SecretValue, ttl: std::time::Duration) -> Self {
        Self {
            value,
            expires_at: Some(SystemTime::now() + ttl),
            renewable: false,
            lease_id: None,
        }
    }

    /// Check if this lease has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| SystemTime::now() > exp)
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for SecretLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretLease")
            .field("value", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("renewable", &self.renewable)
            .field("lease_id", &self.lease_id)
            .finish()
    }
}

/// Resolve a secret from a specific backend.
///
/// Implementations are backend-specific: `VaultResolver` talks to Vault,
/// `AwsResolver` talks to AWS Secrets Manager, `KeystoreResolver` talks to
/// the OS keychain, etc.
///
/// Resolvers do NOT map names to sources. That mapping lives in
/// `CredentialRef.source`. The resolver receives the source directly
/// and knows how to fetch from that backend.
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Resolve a secret from the given source.
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError>;
}

/// How to match a [`SecretSource`] variant to a resolver.
#[derive(Debug, Clone)]
pub enum SourceMatcher {
    /// Match all `SecretSource::Vault` variants.
    Vault,
    /// Match all `SecretSource::AwsSecretsManager` variants.
    Aws,
    /// Match all `SecretSource::GcpSecretManager` variants.
    Gcp,
    /// Match all `SecretSource::AzureKeyVault` variants.
    Azure,
    /// Match all `SecretSource::OsKeystore` variants.
    OsKeystore,
    /// Match all `SecretSource::Kubernetes` variants.
    Kubernetes,
    /// Match all `SecretSource::Hardware` variants.
    Hardware,
    /// Match a specific `SecretSource::Custom` provider name.
    Custom(String),
}

impl SourceMatcher {
    /// Check if this matcher matches the given source.
    pub fn matches(&self, source: &SecretSource) -> bool {
        match (self, source) {
            (SourceMatcher::Vault, SecretSource::Vault { .. }) => true,
            (SourceMatcher::Aws, SecretSource::AwsSecretsManager { .. }) => true,
            (SourceMatcher::Gcp, SecretSource::GcpSecretManager { .. }) => true,
            (SourceMatcher::Azure, SecretSource::AzureKeyVault { .. }) => true,
            (SourceMatcher::OsKeystore, SecretSource::OsKeystore { .. }) => true,
            (SourceMatcher::Kubernetes, SecretSource::Kubernetes { .. }) => true,
            (SourceMatcher::Hardware, SecretSource::Hardware { .. }) => true,
            (SourceMatcher::Custom(name), SecretSource::Custom { provider, .. }) => {
                name == provider
            }
            _ => false,
        }
    }
}

/// Composes multiple resolvers, routing by [`SecretSource`] variant.
///
/// When `resolve()` is called, the registry matches the source to a registered
/// resolver and delegates. If no resolver matches, returns `SecretError::NoResolver`.
///
/// Optionally emits [`SecretAccessEvent`](layer0::secret::SecretAccessEvent)s through a [`SecretEventSink`] for audit logging.
/// Use [`resolve_named`](SecretRegistry::resolve_named) (not the trait's `resolve()`) when you have
/// a credential name for proper audit events.
pub struct SecretRegistry {
    resolvers: Vec<(SourceMatcher, Arc<dyn SecretResolver>)>,
    event_sink: Option<Arc<dyn SecretEventSink>>,
}

impl SecretRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            resolvers: Vec::new(),
            event_sink: None,
        }
    }

    /// Register a resolver for sources matching the given pattern.
    pub fn with_resolver(
        mut self,
        matcher: SourceMatcher,
        resolver: Arc<dyn SecretResolver>,
    ) -> Self {
        self.resolvers.push((matcher, resolver));
        self
    }

    /// Set the event sink for audit logging.
    pub fn with_event_sink(mut self, sink: Arc<dyn SecretEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// Add a resolver for sources matching the given pattern.
    pub fn add(&mut self, matcher: SourceMatcher, resolver: Arc<dyn SecretResolver>) {
        self.resolvers.push((matcher, resolver));
    }
}

impl Default for SecretRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Optional event sink for audit logging of secret access.
///
/// The SecretRegistry emits [`SecretAccessEvent`](layer0::secret::SecretAccessEvent)s through this sink.
/// Implementations can forward to an event bus, write to audit logs,
/// or feed anomaly detection systems.
///
/// If no sink is provided to SecretRegistry, events are silently dropped.
pub trait SecretEventSink: Send + Sync {
    /// Emit a secret access event.
    fn emit(&self, event: layer0::secret::SecretAccessEvent);
}

impl SecretRegistry {
    /// Resolve a secret by credential name and source, emitting an audit event.
    ///
    /// This is the primary entry point for Environment implementations.
    /// Use this instead of `resolve()` when you have a `CredentialRef` --
    /// the credential name flows into the `SecretAccessEvent` for audit logging.
    ///
    /// ```rust,ignore
    /// let lease = registry.resolve_named(&cred.name, &cred.source).await?;
    /// ```
    pub async fn resolve_named(
        &self,
        credential_name: &str,
        source: &SecretSource,
    ) -> Result<SecretLease, SecretError> {
        let result = self.resolve(source).await;
        // Emit audit event if sink is configured
        if let Some(sink) = &self.event_sink {
            use layer0::secret::{SecretAccessEvent, SecretAccessOutcome};
            let outcome = if result.is_ok() {
                SecretAccessOutcome::Resolved
            } else {
                SecretAccessOutcome::Failed
            };
            let event = SecretAccessEvent::new(
                credential_name,
                source.clone(),
                outcome,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
            sink.emit(event);
        }
        result
    }
}

#[async_trait]
impl SecretResolver for SecretRegistry {
    /// Route to the matching resolver. No audit event -- use `resolve_named()`
    /// when you have a credential name for proper audit logging.
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        for (matcher, resolver) in &self.resolvers {
            if matcher.matches(source) {
                return resolver.resolve(source).await;
            }
        }
        Err(SecretError::NoResolver(source.kind().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_value_debug_is_redacted() {
        let secret = SecretValue::new(b"super-secret-key".to_vec());
        let debug = format!("{:?}", secret);
        assert_eq!(debug, "[REDACTED]");
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn secret_value_with_bytes_exposes_content() {
        let secret = SecretValue::new(b"my-api-key".to_vec());
        secret.with_bytes(|bytes| {
            assert_eq!(bytes, b"my-api-key");
        });
    }

    #[test]
    fn secret_value_len() {
        let secret = SecretValue::new(b"12345".to_vec());
        assert_eq!(secret.len(), 5);
        assert!(!secret.is_empty());

        let empty = SecretValue::new(vec![]);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn secret_lease_permanent_never_expires() {
        let lease = SecretLease::permanent(SecretValue::new(b"key".to_vec()));
        assert!(!lease.is_expired());
        assert!(lease.expires_at.is_none());
        assert!(!lease.renewable);
    }

    #[test]
    fn secret_lease_debug_redacts_value() {
        let lease = SecretLease::permanent(SecretValue::new(b"secret".to_vec()));
        let debug = format!("{:?}", lease);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn source_matcher_vault() {
        let matcher = SourceMatcher::Vault;
        assert!(matcher.matches(&SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        }));
        assert!(!matcher.matches(&SecretSource::OsKeystore {
            service: "test".into(),
        }));
    }

    #[test]
    fn source_matcher_custom() {
        let matcher = SourceMatcher::Custom("1password".into());
        assert!(matcher.matches(&SecretSource::Custom {
            provider: "1password".into(),
            config: serde_json::json!({}),
        }));
        assert!(!matcher.matches(&SecretSource::Custom {
            provider: "bitwarden".into(),
            config: serde_json::json!({}),
        }));
    }

    // Object safety
    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn secret_resolver_is_object_safe_send_sync() {
        _assert_send_sync::<Box<dyn SecretResolver>>();
        _assert_send_sync::<Arc<dyn SecretResolver>>();
    }

    #[tokio::test]
    async fn registry_no_resolver_returns_error() {
        let registry = SecretRegistry::new();
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let result = registry.resolve(&source).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecretError::NoResolver(_)));
    }

    // Test registry dispatches to correct resolver
    struct StubResolver {
        value: &'static [u8],
    }

    #[async_trait]
    impl SecretResolver for StubResolver {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            Ok(SecretLease::permanent(SecretValue::new(
                self.value.to_vec(),
            )))
        }
    }

    #[tokio::test]
    async fn registry_dispatches_to_matching_resolver() {
        let registry = SecretRegistry::new()
            .with_resolver(
                SourceMatcher::Vault,
                Arc::new(StubResolver {
                    value: b"vault-secret",
                }),
            )
            .with_resolver(
                SourceMatcher::OsKeystore,
                Arc::new(StubResolver {
                    value: b"keystore-secret",
                }),
            );

        let vault_source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let lease = registry.resolve(&vault_source).await.unwrap();
        lease.value.with_bytes(|b| assert_eq!(b, b"vault-secret"));

        let keystore_source = SecretSource::OsKeystore {
            service: "test".into(),
        };
        let lease = registry.resolve(&keystore_source).await.unwrap();
        lease
            .value
            .with_bytes(|b| assert_eq!(b, b"keystore-secret"));
    }

    #[test]
    fn secret_error_display_all_variants() {
        assert_eq!(
            SecretError::NotFound("api-key".into()).to_string(),
            "secret not found: api-key"
        );
        assert_eq!(
            SecretError::AccessDenied("no policy".into()).to_string(),
            "access denied: no policy"
        );
        assert_eq!(
            SecretError::BackendError("timeout".into()).to_string(),
            "backend error: timeout"
        );
        assert_eq!(
            SecretError::LeaseExpired("lease-123".into()).to_string(),
            "lease expired: lease-123"
        );
        assert_eq!(
            SecretError::NoResolver("vault".into()).to_string(),
            "no resolver for source: vault"
        );
    }
}
