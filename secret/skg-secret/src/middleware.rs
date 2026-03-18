//! Middleware traits for secret resolution using the continuation pattern.
//!
//! [`SecretMiddleware`] wraps [`crate::SecretResolver`]`::resolve` with
//! a continuation-passing chain, enabling policy guards, audit observers,
//! and caching layers without modifying resolver implementations.
//!
//! ## Stacking order
//!
//! Observers (outermost) → Transformers → Guards (innermost).
//!
//! Observers always run (even if a guard halts) because they are the
//! outermost layer. Guards see transformed input because transformers
//! sit between observers and guards.

use crate::{SecretError, SecretLease};
use async_trait::async_trait;
use layer0::secret::SecretSource;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SECRET NEXT (continuation)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in a secret middleware chain.
///
/// Call `resolve()` to pass control to the inner layer.
/// Don't call it to short-circuit (guardrail halt, cached response).
#[async_trait]
pub trait SecretNext: Send + Sync {
    /// Forward the resolve request to the next layer.
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SECRET MIDDLEWARE (wraps SecretResolver::resolve)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Middleware wrapping `SecretResolver::resolve`.
///
/// Code before `next.resolve()` = pre-processing (source mutation, logging).
/// Code after `next.resolve()` = post-processing (response mutation, metrics).
/// Not calling `next.resolve()` = short-circuit (guardrail halt, cached response).
///
/// Use for: policy enforcement, audit logging, caching, secret rotation,
/// access rate limiting.
#[async_trait]
pub trait SecretMiddleware: Send + Sync {
    /// Intercept a secret resolve call.
    async fn resolve(
        &self,
        source: &SecretSource,
        next: &dyn SecretNext,
    ) -> Result<SecretLease, SecretError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SECRET STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for secret resolution operations.
///
/// Built via [`SecretStack::builder()`]. Stacking order:
/// Observers (outermost) → Transformers → Guards (innermost).
///
/// Observers always run (even if a guard halts) because they're
/// the outermost layer. Guards see transformed input because
/// transformers are between observers and guards.
pub struct SecretStack {
    /// Middleware layers in call order (outermost first).
    layers: Vec<Arc<dyn SecretMiddleware>>,
}

/// Builder for [`SecretStack`].
pub struct SecretStackBuilder {
    observers: Vec<Arc<dyn SecretMiddleware>>,
    transformers: Vec<Arc<dyn SecretMiddleware>>,
    guards: Vec<Arc<dyn SecretMiddleware>>,
}

impl SecretStack {
    /// Start building a secret middleware stack.
    pub fn builder() -> SecretStackBuilder {
        SecretStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Resolve through the middleware chain, ending at `terminal`.
    pub async fn resolve_with(
        &self,
        source: &SecretSource,
        terminal: &dyn SecretNext,
    ) -> Result<SecretLease, SecretError> {
        if self.layers.is_empty() {
            return terminal.resolve(source).await;
        }
        let chain = SecretChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.resolve(source).await
    }
}

impl SecretStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn SecretMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware (mutates input/output, always calls next).
    pub fn transform(mut self, mw: Arc<dyn SecretMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit by not calling next).
    pub fn guard(mut self, mw: Arc<dyn SecretMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> SecretStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        SecretStack { layers }
    }
}

struct SecretChain<'a> {
    layers: &'a [Arc<dyn SecretMiddleware>],
    index: usize,
    terminal: &'a dyn SecretNext,
}

#[async_trait]
impl SecretNext for SecretChain<'_> {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        if self.index >= self.layers.len() {
            return self.terminal.resolve(source).await;
        }
        let next = SecretChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].resolve(source, &next).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretValue;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ━━━ Helpers ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// A terminal that always succeeds with a known secret.
    struct StubTerminal {
        value: &'static [u8],
    }

    #[async_trait]
    impl SecretNext for StubTerminal {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            Ok(SecretLease::permanent(SecretValue::new(
                self.value.to_vec(),
            )))
        }
    }

    /// A terminal that tracks how many times it was called.
    struct CountingTerminal {
        value: &'static [u8],
        count: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl SecretNext for CountingTerminal {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(SecretLease::permanent(SecretValue::new(
                self.value.to_vec(),
            )))
        }
    }

    fn vault_source() -> SecretSource {
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/api-key".into(),
        }
    }

    fn keystore_source() -> SecretSource {
        SecretSource::OsKeystore {
            service: "my-app".into(),
        }
    }

    // ━━━ Test 1: Policy guard denies access ━━━━━━━━━━━━━━━━━

    /// A guard middleware that denies Vault sources.
    struct DenyVaultGuard;

    #[async_trait]
    impl SecretMiddleware for DenyVaultGuard {
        async fn resolve(
            &self,
            source: &SecretSource,
            next: &dyn SecretNext,
        ) -> Result<SecretLease, SecretError> {
            if matches!(source, SecretSource::Vault { .. }) {
                return Err(SecretError::AccessDenied(
                    "vault access blocked by policy".into(),
                ));
            }
            next.resolve(source).await
        }
    }

    #[tokio::test]
    async fn guard_denies_vault_source() {
        let stack = SecretStack::builder()
            .guard(Arc::new(DenyVaultGuard))
            .build();

        let terminal = StubTerminal {
            value: b"should-not-reach",
        };

        let result = stack.resolve_with(&vault_source(), &terminal).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SecretError::AccessDenied(_)),
            "expected AccessDenied, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn guard_allows_non_vault_source() {
        let stack = SecretStack::builder()
            .guard(Arc::new(DenyVaultGuard))
            .build();

        let terminal = StubTerminal {
            value: b"keystore-value",
        };

        let result = stack.resolve_with(&keystore_source(), &terminal).await;
        assert!(result.is_ok());
        result
            .unwrap()
            .value
            .with_bytes(|b| assert_eq!(b, b"keystore-value"));
    }

    // ━━━ Test 2: Audit observer records all resolutions ━━━━━

    /// An observer that records all resolve calls.
    struct AuditObserver {
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl SecretMiddleware for AuditObserver {
        async fn resolve(
            &self,
            source: &SecretSource,
            next: &dyn SecretNext,
        ) -> Result<SecretLease, SecretError> {
            self.log.lock().unwrap().push(source.kind().to_string());
            next.resolve(source).await
        }
    }

    #[tokio::test]
    async fn observer_records_all_resolutions() {
        let log = Arc::new(Mutex::new(Vec::<String>::new()));

        let stack = SecretStack::builder()
            .observe(Arc::new(AuditObserver { log: log.clone() }))
            .build();

        let terminal = StubTerminal { value: b"secret" };

        let sources = [
            vault_source(),
            keystore_source(),
            SecretSource::AwsSecretsManager {
                secret_id: "my-secret".into(),
                region: None,
            },
        ];

        for source in &sources {
            let _ = stack.resolve_with(source, &terminal).await;
        }

        let entries = log.lock().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], "vault");
        assert_eq!(entries[1], "os_keystore");
        assert_eq!(entries[2], "aws");
    }

    // ━━━ Test 3: Caching middleware returns cached lease ━━━━

    /// A caching middleware that stores resolved secret bytes and
    /// returns them on subsequent calls for the same source kind+key.
    ///
    /// Caches raw bytes (not `SecretLease`) because `SecretValue`
    /// does not implement `Clone`.
    struct CachingMiddleware {
        cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    impl CachingMiddleware {
        fn cache_key(source: &SecretSource) -> String {
            format!("{source:?}")
        }
    }

    #[async_trait]
    impl SecretMiddleware for CachingMiddleware {
        async fn resolve(
            &self,
            source: &SecretSource,
            next: &dyn SecretNext,
        ) -> Result<SecretLease, SecretError> {
            let key = Self::cache_key(source);

            // Check cache
            {
                let cache = self.cache.lock().unwrap();
                if let Some(bytes) = cache.get(&key) {
                    return Ok(SecretLease::permanent(SecretValue::new(bytes.clone())));
                }
            }

            // Cache miss — resolve and store
            let lease = next.resolve(source).await?;
            let bytes = lease.value.with_bytes(|b| b.to_vec());
            self.cache.lock().unwrap().insert(key, bytes);
            Ok(lease)
        }
    }

    #[tokio::test]
    async fn caching_middleware_skips_backend_on_hit() {
        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let stack = SecretStack::builder()
            .transform(Arc::new(CachingMiddleware {
                cache: Arc::new(Mutex::new(HashMap::new())),
            }))
            .build();

        let terminal = CountingTerminal {
            value: b"cached-secret",
            count: call_count.clone(),
        };

        let source = keystore_source();

        // First resolve — cache miss, hits backend
        let lease1 = stack.resolve_with(&source, &terminal).await.unwrap();
        lease1.value.with_bytes(|b| assert_eq!(b, b"cached-secret"));
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "first call should hit backend"
        );

        // Second resolve — cache hit, no backend call
        let lease2 = stack.resolve_with(&source, &terminal).await.unwrap();
        lease2.value.with_bytes(|b| assert_eq!(b, b"cached-secret"));
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "second call should use cache, not hit backend"
        );
    }

    // ━━━ Test 4: Stack composes correctly (guard + observer) ━

    #[tokio::test]
    async fn stack_composes_guard_and_observer() {
        let log = Arc::new(Mutex::new(Vec::<String>::new()));

        let stack = SecretStack::builder()
            .observe(Arc::new(AuditObserver { log: log.clone() }))
            .guard(Arc::new(DenyVaultGuard))
            .build();

        let terminal = StubTerminal {
            value: b"the-secret",
        };

        // Attempt Vault — blocked by guard, but observer sees it
        let result = stack.resolve_with(&vault_source(), &terminal).await;
        assert!(matches!(result.unwrap_err(), SecretError::AccessDenied(_)));

        // Attempt Keystore — succeeds, observer sees it
        let result = stack.resolve_with(&keystore_source(), &terminal).await;
        assert!(result.is_ok());
        result
            .unwrap()
            .value
            .with_bytes(|b| assert_eq!(b, b"the-secret"));

        // Observer saw both attempts
        let entries = log.lock().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], "vault");
        assert_eq!(entries[1], "os_keystore");
    }

    // ━━━ Test 5: Empty stack passes through to terminal ━━━━━

    #[tokio::test]
    async fn empty_stack_passthrough() {
        let stack = SecretStack::builder().build();

        let terminal = StubTerminal {
            value: b"direct-value",
        };

        let result = stack.resolve_with(&keystore_source(), &terminal).await;
        assert!(result.is_ok());
        result
            .unwrap()
            .value
            .with_bytes(|b| assert_eq!(b, b"direct-value"));
    }

    // ━━━ Test 6: Object safety ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn secret_middleware_is_object_safe() {
        fn _assert_send_sync<T: Send + Sync>() {}
        _assert_send_sync::<Box<dyn SecretMiddleware>>();
        _assert_send_sync::<Box<dyn SecretNext>>();
    }
}
