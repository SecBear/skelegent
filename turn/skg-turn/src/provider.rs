//! Provider trait for LLM backends.
//!
//! The [`Provider`] trait uses RPITIT (return-position `impl Trait` in traits)
//! and is intentionally NOT object-safe. The object-safe boundary is
//! `layer0::Turn` — Turn<P: Provider> implements Turn.
//!
//! [`Message`]: layer0::context::Message

use crate::embedding::{EmbedRequest, EmbedResponse};
use crate::infer::{InferRequest, InferResponse};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
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
    RateLimited {
        /// Server-suggested delay before retry, if provided via `Retry-After` header.
        retry_after: Option<Duration>,
    },

    /// Client-side error (malformed request, bad parameters) — do NOT retry.
    #[error("invalid request: {message}")]
    InvalidRequest {
        /// Human-readable description.
        message: String,
        /// HTTP status code.
        status: Option<u16>,
    },

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
            ProviderError::RateLimited { .. } | ProviderError::TransientError { .. }
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
/// `Turn<P: Provider>` is generic, and the object-safe boundary
/// is `layer0::Turn`.
pub trait Provider: Send + Sync {
    /// Run inference using [`layer0::context::Message`] types directly.
    ///
    /// Operators call this. The provider converts to its wire format internally.
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send;

    /// Embed texts into vector space.
    ///
    /// Not all providers support embedding. The default implementation returns
    /// an error. Providers that support embedding (Anthropic, OpenAI) override
    /// this method.
    fn embed(
        &self,
        _request: EmbedRequest,
    ) -> impl Future<Output = Result<EmbedResponse, ProviderError>> + Send {
        async {
            Err(ProviderError::Other(
                "embedding not supported by this provider".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// DynProvider — object-safe companion trait
// ---------------------------------------------------------------------------

/// Object-safe wrapper for [`Provider`].
///
/// You almost never implement this directly — implement [`Provider`] instead.
/// The blanket impl automatically provides `DynProvider` for any `Provider`.
///
/// Use `DynProvider` when you need:
/// - Heterogeneous collections: `Vec<Box<dyn DynProvider>>`
/// - Middleware stacks: `MiddlewareProvider` wraps `Box<dyn DynProvider>`
/// - Runtime provider selection: match on config, get different provider
pub trait DynProvider: Send + Sync {
    /// Run inference, returning a boxed future.
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>>;

    /// Run embedding, returning a boxed future.
    ///
    /// Default implementation returns an unsupported error. Override for
    /// providers that support embedding.
    fn embed_boxed(
        &self,
        _request: EmbedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EmbedResponse, ProviderError>> + Send + '_>> {
        Box::pin(async {
            Err(ProviderError::Other(
                "embedding not supported by this provider".into(),
            ))
        })
    }
}

/// Blanket impl: any [`Provider`] is automatically a [`DynProvider`].
impl<T: Provider> DynProvider for T {
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>> {
        Box::pin(self.infer(request))
    }

    fn embed_boxed(
        &self,
        request: EmbedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EmbedResponse, ProviderError>> + Send + '_>> {
        Box::pin(self.embed(request))
    }
}

/// Wrap a concrete [`Provider`] into a `Box<dyn DynProvider>`.
///
/// # Example
///
/// ```rust,ignore
/// use skg_turn::box_provider;
/// let boxed = box_provider(my_anthropic_provider);
/// ```
pub fn box_provider<P: Provider + 'static>(p: P) -> Box<dyn DynProvider> {
    Box::new(p)
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
        assert_eq!(
            ProviderError::RateLimited { retry_after: None }.to_string(),
            "rate limited"
        );
        assert_eq!(
            ProviderError::InvalidRequest {
                message: "bad param".into(),
                status: Some(400),
            }
            .to_string(),
            "invalid request: bad param"
        );
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
        assert!(ProviderError::RateLimited { retry_after: None }.is_retryable());
        assert!(
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
            .is_retryable()
        );
        assert!(
            !ProviderError::InvalidRequest {
                message: "bad".into(),
                status: Some(400),
            }
            .is_retryable()
        );
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
        assert!(ProviderError::RateLimited { retry_after: None }.is_retryable());
    }

    #[test]
    fn rate_limited_with_retry_after_display() {
        assert_eq!(
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
            .to_string(),
            "rate limited"
        );
    }

    #[test]
    fn invalid_request_is_not_retryable() {
        assert!(
            !ProviderError::InvalidRequest {
                message: "missing field".into(),
                status: Some(422),
            }
            .is_retryable()
        );
    }

    #[test]
    fn invalid_request_display() {
        assert_eq!(
            ProviderError::InvalidRequest {
                message: "bad param".into(),
                status: Some(400),
            }
            .to_string(),
            "invalid request: bad param"
        );
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
