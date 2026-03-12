//! Phase boundary: [`Context::compile()`] → [`CompiledContext::infer()`].
//!
//! Assembly produces a [`Context`]. `compile()` snapshots it for the model.
//! `infer()` crosses the network boundary. The model is a discontinuity —
//! what comes out is generated, not transformed.

use crate::context::Context;
use crate::error::EngineError;
use skg_turn::infer::{InferRequest, InferResponse};
use skg_turn::provider::Provider;
use skg_turn::types::ToolSchema;

/// Configuration for compiling context into an inference request.
#[derive(Debug, Clone, Default)]
pub struct CompileConfig {
    /// System prompt to inject.
    pub system: Option<String>,
    /// Model to use. `None` = provider default.
    pub model: Option<String>,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Tool schemas available to the model.
    pub tools: Vec<ToolSchema>,
    /// Provider-specific config passthrough.
    pub extra: serde_json::Value,
}

/// A snapshot of context compiled for inference.
///
/// Produced by [`Context::compile()`]. This snapshots context for a later
/// provider call, but it is not itself the governed inference boundary.
/// Runtime loops should target [`crate::InferBoundary`] or
/// [`crate::StreamInferBoundary`] for pre-inference rules.
pub struct CompiledContext {
    /// The inference request ready to send.
    pub request: InferRequest,
}

impl CompiledContext {
    /// Send this compiled context to a provider for inference.
    ///
    /// This is the network boundary crossing. What comes back is generated,
    /// not a transformation of what went in.
    ///
    /// The response is NOT automatically appended to context. The caller
    /// decides what to do with it — append, route, discard, transform.
    pub async fn infer<P: Provider>(self, provider: &P) -> Result<InferResult, EngineError> {
        let response = provider.infer(self.request).await?;
        Ok(InferResult { response })
    }
}

/// The result of an inference call.
///
/// Wraps [`InferResponse`] with convenience methods. The response is NOT
/// automatically appended to context — that's a separate context op that
/// the caller chooses to run (or not).
pub struct InferResult {
    /// The raw inference response.
    pub response: InferResponse,
}

impl InferResult {
    /// Whether the model is requesting tool calls.
    pub fn has_tool_calls(&self) -> bool {
        self.response.has_tool_calls()
    }

    /// Get the text content of the response, if any.
    pub fn text(&self) -> Option<&str> {
        self.response.text()
    }
}

impl Context {
    /// Compile the current context into an inference request.
    ///
    /// This snapshots the current assembled context into a provider request.
    /// The actual governed inference boundary is `InferBoundary` /
    /// `StreamInferBoundary`, not `compile()` itself. The context is NOT
    /// consumed — you can compile multiple times (e.g., for retry).
    ///
    /// Messages are cloned into the request. The context continues to exist
    /// for post-inference operations.
    pub fn compile(&self, config: &CompileConfig) -> CompiledContext {
        let mut request = InferRequest::new(self.messages().to_vec());

        if let Some(system) = &config.system {
            request = request.with_system(system.clone());
        }
        if let Some(model) = &config.model {
            request = request.with_model(model.clone());
        }
        if let Some(max_tokens) = config.max_tokens {
            request = request.with_max_tokens(max_tokens);
        }
        if let Some(temp) = config.temperature {
            request = request.with_temperature(temp);
        }
        if !config.tools.is_empty() {
            request = request.with_tools(config.tools.clone());
        }
        if config.extra != serde_json::Value::Null {
            request = request.with_extra(config.extra.clone());
        }

        CompiledContext { request }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::{Message, Role};

    #[test]
    fn compile_produces_request_with_messages() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        let config = CompileConfig {
            system: Some("You are helpful.".into()),
            model: Some("test-model".into()),
            max_tokens: Some(1024),
            ..Default::default()
        };

        let compiled = ctx.compile(&config);
        assert_eq!(compiled.request.messages.len(), 1);
        assert_eq!(compiled.request.system.as_deref(), Some("You are helpful."));
        assert_eq!(compiled.request.model.as_deref(), Some("test-model"));
        assert_eq!(compiled.request.max_tokens, Some(1024));
    }

    #[test]
    fn compile_does_not_consume_context() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        let config = CompileConfig::default();
        let _compiled = ctx.compile(&config);

        // Context still has its messages
        assert_eq!(ctx.messages().len(), 1);
    }
}
