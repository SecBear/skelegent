//! Provider trait for LLM backends.
//!
//! The [`Provider`] trait uses RPITIT (return-position `impl Trait` in traits)
//! and is intentionally NOT object-safe. The object-safe boundary is
//! `layer0::Turn` — NeuronTurn<P: Provider> implements Turn.

use crate::types::{ProviderRequest, ProviderResponse};
use std::future::Future;
use thiserror::Error;

/// Errors from LLM providers.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ProviderError {
    /// HTTP or network request failed.
    #[error("request failed: {0}")]
    RequestFailed(String),

    /// Provider rate-limited the request.
    #[error("rate limited")]
    RateLimited,

    /// Authentication/authorization failed.
    #[error("auth failed: {0}")]
    AuthFailed(String),

    /// Could not parse the provider's response.
    #[error("invalid response: {0}")]
    InvalidResponse(String),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl ProviderError {
    /// Whether retrying this request might succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited | ProviderError::RequestFailed(_)
        )
    }
}

/// LLM provider interface.
///
/// Each provider (Anthropic, OpenAI, Ollama) implements this trait.
/// Provider-native features (truncation, caching, thinking blocks)
/// are handled by the provider impl using `ProviderRequest.extra`.
///
/// This trait uses RPITIT and is NOT object-safe. That's intentional —
/// `NeuronTurn<P: Provider>` is generic, and the object-safe boundary
/// is `layer0::Turn`.
pub trait Provider: Send + Sync {
    /// Send a completion request to the provider.
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl Future<Output = Result<ProviderResponse, ProviderError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_display() {
        assert_eq!(
            ProviderError::RequestFailed("timeout".into()).to_string(),
            "request failed: timeout"
        );
        assert_eq!(ProviderError::RateLimited.to_string(), "rate limited");
        assert_eq!(
            ProviderError::AuthFailed("bad key".into()).to_string(),
            "auth failed: bad key"
        );
        assert_eq!(
            ProviderError::InvalidResponse("bad json".into()).to_string(),
            "invalid response: bad json"
        );
    }

    #[test]
    fn provider_error_retryable() {
        assert!(ProviderError::RateLimited.is_retryable());
        assert!(ProviderError::RequestFailed("timeout".into()).is_retryable());
        assert!(!ProviderError::AuthFailed("bad key".into()).is_retryable());
        assert!(!ProviderError::InvalidResponse("x".into()).is_retryable());
    }

    #[test]
    fn provider_error_from_boxed() {
        let err: Box<dyn std::error::Error + Send + Sync> = "some error".into();
        let provider_err = ProviderError::from(err);
        assert!(matches!(provider_err, ProviderError::Other(_)));
        assert!(!provider_err.is_retryable());
    }

    #[test]
    fn provider_error_other_display() {
        let err: Box<dyn std::error::Error + Send + Sync> = "custom error".into();
        let provider_err = ProviderError::from(err);
        assert_eq!(provider_err.to_string(), "custom error");
    }
}
