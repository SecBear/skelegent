//! Provider trait for LLM backends.
//!
//! The [`Provider`] trait uses RPITIT (return-position `impl Trait` in traits)
//! and is intentionally NOT object-safe. The object-safe boundary is
//! `layer0::Turn` — NeuronTurn<P: Provider> implements Turn.
//!
//! [`Message`]: layer0::context::Message

use crate::infer::{InferRequest, InferResponse};
use std::future::Future;
use thiserror::Error;

/// Errors from LLM providers.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ProviderError {
    /// Transient network or server error — safe to retry with backoff.
    #[error("transient error: {message}")]
    TransientError {
        /// Human-readable description of the error.
        message: String,
        /// HTTP status code if available.
        status: Option<u16>,
    },

    /// Provider rate-limited the request.
    #[error("rate limited")]
    RateLimited,

    /// Content blocked by safety filter — do NOT retry.
    #[error("content blocked: {message}")]
    ContentBlocked {
        /// Human-readable description of what was blocked.
        message: String,
    },

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
            ProviderError::RateLimited | ProviderError::TransientError { .. }
        )
    }
}

/// LLM provider interface.
///
/// Each provider (Anthropic, OpenAI, Ollama) implements this trait.
/// Provider-native features (truncation, caching, thinking blocks)
/// are handled by the provider impl using `InferRequest.extra`.
///
/// This trait uses RPITIT and is NOT object-safe. That's intentional —
/// `NeuronTurn<P: Provider>` is generic, and the object-safe boundary
/// is `layer0::Turn`.
pub trait Provider: Send + Sync {
    /// Run inference using layer0 [`Message`] types directly.
    ///
    /// Operators call this. The provider converts to its wire format internally.
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send;
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_display() {
        assert_eq!(
            ProviderError::TransientError {
                message: "timeout".into(),
                status: None,
            }
            .to_string(),
            "transient error: timeout"
        );
        assert_eq!(
            ProviderError::TransientError {
                message: "server error".into(),
                status: Some(500),
            }
            .to_string(),
            "transient error: server error"
        );
        assert_eq!(
            ProviderError::ContentBlocked {
                message: "blocked".into(),
            }
            .to_string(),
            "content blocked: blocked"
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
        assert!(
            ProviderError::TransientError {
                message: "timeout".into(),
                status: None,
            }
            .is_retryable()
        );
        assert!(
            ProviderError::TransientError {
                message: "server error".into(),
                status: Some(500),
            }
            .is_retryable()
        );
        assert!(!ProviderError::AuthFailed("bad key".into()).is_retryable());
        assert!(!ProviderError::InvalidResponse("x".into()).is_retryable());
        assert!(
            !ProviderError::ContentBlocked {
                message: "blocked".into(),
            }
            .is_retryable()
        );
    }

    #[test]
    fn content_blocked_is_not_retryable() {
        assert!(
            !ProviderError::ContentBlocked {
                message: "safety filter triggered".into(),
            }
            .is_retryable()
        );
    }

    #[test]
    fn auth_failed_is_not_retryable() {
        assert!(!ProviderError::AuthFailed("401 Unauthorized".into()).is_retryable());
    }

    #[test]
    fn transient_error_is_retryable() {
        assert!(
            ProviderError::TransientError {
                message: "connection reset".into(),
                status: None,
            }
            .is_retryable()
        );
        assert!(
            ProviderError::TransientError {
                message: "HTTP 503: service unavailable".into(),
                status: Some(503),
            }
            .is_retryable()
        );
    }

    #[test]
    fn rate_limited_is_retryable() {
        assert!(ProviderError::RateLimited.is_retryable());
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
