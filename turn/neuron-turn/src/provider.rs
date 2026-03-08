//! Provider trait for LLM backends.
//!
//! The [`Provider`] trait uses RPITIT (return-position `impl Trait` in traits)
//! and is intentionally NOT object-safe. The object-safe boundary is
//! `layer0::Turn` — NeuronTurn<P: Provider> implements Turn.
//!
//! ## Migration to `infer()`
//!
//! The `infer()` method is the preferred interface — it speaks [`Message`]
//! natively and eliminates manual conversion. `complete()` is deprecated
//! and will be removed once all providers implement `infer()` natively.
//!
//! [`Message`]: layer0::context::Message

use crate::infer::{InferRequest, InferResponse, ToolCall};
use crate::types::{ContentPart, ProviderRequest, ProviderResponse};
use layer0::content::Content;
use layer0::context::Message;
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
///
/// ## Two methods
///
/// - [`infer()`](Provider::infer) — preferred. Speaks [`Message`] natively.
///   Operators should use this exclusively.
/// - [`complete()`](Provider::complete) — deprecated legacy interface using
///   `ProviderRequest`/`ProviderResponse`. Will be removed.
///
/// Implementors should override `infer()`. The default `infer()` bridges
/// through `complete()` for backward compatibility during migration.
pub trait Provider: Send + Sync {
    /// Preferred: run inference using layer0 [`Message`] types directly.
    ///
    /// Operators call this — not `complete()`. The provider converts to
    /// its wire format internally.
    ///
    /// Default implementation bridges through `complete()` for providers
    /// that haven't migrated yet. Override this for native support.
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
        async move {
            // Bridge: convert InferRequest → ProviderRequest → complete() → InferResponse
            let provider_request = infer_to_provider_request(&request);
            #[allow(deprecated)]
            let provider_response = self.complete(provider_request).await?;
            Ok(provider_response_to_infer(provider_response))
        }
    }

    /// Deprecated: send a completion request using wire-format types.
    ///
    /// New providers should implement [`infer()`](Provider::infer) instead.
    /// This method will be removed once all consumers are migrated.
    #[deprecated(note = "Use `infer()` instead — speaks Message natively")]
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl Future<Output = Result<ProviderResponse, ProviderError>> + Send {
        // Default: bridge through infer() for providers that only implement infer()
        async move {
            let infer_request = provider_request_to_infer(request);
            let infer_response = self.infer(infer_request).await?;
            Ok(infer_to_provider_response(infer_response))
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Bridge functions (temporary — removed when complete() is deleted)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Convert InferRequest → ProviderRequest for the bridge.
fn infer_to_provider_request(req: &InferRequest) -> ProviderRequest {
    use crate::convert::{content_to_parts, role_from_layer0};
    use crate::types::ProviderMessage;

    let messages = req
        .messages
        .iter()
        .map(|msg| ProviderMessage {
            role: role_from_layer0(&msg.role),
            content: content_to_parts(&msg.content),
        })
        .collect();

    ProviderRequest {
        model: req.model.clone(),
        messages,
        tools: req.tools.clone(),
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        system: req.system.clone(),
        extra: req.extra.clone(),
    }
}

/// Convert ProviderResponse → InferResponse for the bridge.
fn provider_response_to_infer(resp: ProviderResponse) -> InferResponse {
    // Extract tool calls from content parts
    let tool_calls: Vec<ToolCall> = resp
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolUse { id, name, input } => Some(ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
            _ => None,
        })
        .collect();

    // Convert remaining content (non-tool-use parts)
    let content = {
        use crate::convert::parts_to_content;
        let non_tool_parts: Vec<ContentPart> = resp
            .content
            .iter()
            .filter(|p| !matches!(p, ContentPart::ToolUse { .. }))
            .cloned()
            .collect();
        if non_tool_parts.is_empty() {
            Content::text("")
        } else {
            parts_to_content(&non_tool_parts)
        }
    };

    InferResponse {
        content,
        tool_calls,
        stop_reason: resp.stop_reason,
        usage: resp.usage,
        model: resp.model,
        cost: resp.cost,
        truncated: resp.truncated,
    }
}

/// Convert ProviderRequest → InferRequest for the reverse bridge.
fn provider_request_to_infer(req: ProviderRequest) -> InferRequest {
    use crate::convert::{parts_to_content, role_to_layer0};

    let messages = req
        .messages
        .into_iter()
        .map(|pm| Message::new(role_to_layer0(&pm.role), parts_to_content(&pm.content)))
        .collect();

    InferRequest {
        model: req.model,
        messages,
        tools: req.tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        system: req.system,
        extra: req.extra,
    }
}

/// Convert InferResponse → ProviderResponse for the reverse bridge.
fn infer_to_provider_response(resp: InferResponse) -> ProviderResponse {
    use crate::convert::content_to_parts;

    let mut content = content_to_parts(&resp.content);
    // Re-add tool use parts
    for tc in &resp.tool_calls {
        content.push(ContentPart::ToolUse {
            id: tc.id.clone(),
            name: tc.name.clone(),
            input: tc.input.clone(),
        });
    }

    ProviderResponse {
        content,
        stop_reason: resp.stop_reason,
        usage: resp.usage,
        model: resp.model,
        cost: resp.cost,
        truncated: resp.truncated,
    }
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
