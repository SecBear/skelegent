#![deny(missing_docs)]
//! Secret resolver that reads from process environment variables.
//!
//! Uses `SecretSource::Custom { provider: "env", config: { "var_name": "..." } }`.

use async_trait::async_trait;
use layer0::secret::SecretSource;
use neuron_secret::{SecretError, SecretLease, SecretResolver, SecretValue};

/// Resolves secrets from process environment variables.
///
/// Uses `SecretSource::Custom { provider: "env", config: { "var_name": "..." } }`.
///
/// Config schema:
/// ```json
/// { "type": "custom", "provider": "env", "config": { "var_name": "ANTHROPIC_API_KEY" } }
/// ```
///
/// Required config field: `var_name` (string) — the environment variable name to read.
pub struct EnvResolver;

#[async_trait]
impl SecretResolver for EnvResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, SecretError> {
        match source {
            SecretSource::Custom { provider, config } if provider == "env" => {
                let var_name =
                    config
                        .get("var_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            SecretError::NotFound("env source requires config.var_name".into())
                        })?;
                match std::env::var(var_name) {
                    Ok(val) => Ok(SecretLease::permanent(SecretValue::new(val.into_bytes()))),
                    Err(_) => Err(SecretError::NotFound(format!("env var {var_name} not set"))),
                }
            }
            _ => Err(SecretError::NoResolver("env".into())),
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
        let _: Box<dyn SecretResolver> = Box::new(EnvResolver);
        let _: Arc<dyn SecretResolver> = Arc::new(EnvResolver);
    }

    #[tokio::test]
    async fn resolves_set_env_var() {
        // SAFETY: test-only; unique var name avoids cross-test interference.
        unsafe { std::env::set_var("NEURON_TEST_SECRET_ENV", "test-value-42") };
        let resolver = EnvResolver;
        let source = SecretSource::Custom {
            provider: "env".into(),
            config: serde_json::json!({ "var_name": "NEURON_TEST_SECRET_ENV" }),
        };
        let lease = resolver.resolve(&source).await.unwrap();
        lease.value.with_bytes(|b| assert_eq!(b, b"test-value-42"));
        // SAFETY: test-only cleanup.
        unsafe { std::env::remove_var("NEURON_TEST_SECRET_ENV") };
    }

    #[tokio::test]
    async fn rejects_missing_env_var() {
        // SAFETY: test-only; unique var name avoids cross-test interference.
        unsafe { std::env::remove_var("NEURON_TEST_MISSING_VAR") };
        let resolver = EnvResolver;
        let source = SecretSource::Custom {
            provider: "env".into(),
            config: serde_json::json!({ "var_name": "NEURON_TEST_MISSING_VAR" }),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
        assert!(err.to_string().contains("NEURON_TEST_MISSING_VAR"));
    }

    #[tokio::test]
    async fn rejects_non_custom_source() {
        let resolver = EnvResolver;
        let source = SecretSource::Vault {
            mount: "secret".into(),
            path: "data/key".into(),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }

    #[tokio::test]
    async fn rejects_custom_with_wrong_provider() {
        let resolver = EnvResolver;
        let source = SecretSource::Custom {
            provider: "1password".into(),
            config: serde_json::json!({}),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NoResolver(_)));
    }

    #[tokio::test]
    async fn rejects_missing_var_name_config() {
        let resolver = EnvResolver;
        let source = SecretSource::Custom {
            provider: "env".into(),
            config: serde_json::json!({}),
        };
        let err = resolver.resolve(&source).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
        assert!(err.to_string().contains("var_name"));
    }
}
