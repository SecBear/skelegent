//! Per-boundary middleware traits for provider operations using the continuation pattern.
//!
//! Two middleware traits — one per provider protocol boundary:
//! - [`InferMiddleware`] wraps [`crate::Provider`]`::infer`
//! - [`EmbedMiddleware`] wraps [`crate::Provider`]`::embed`
//!
//! These live in the turn layer (Layer 1) because [`crate::Provider`] uses
//! RPITIT and is not object-safe. The middleware traits use `#[async_trait]`
//! to achieve object safety.

use crate::embedding::{EmbedRequest, EmbedResponse};
use crate::infer::{InferRequest, InferResponse};
use crate::provider::ProviderError;
use async_trait::async_trait;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// INFER MIDDLEWARE (wraps Provider::infer)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in an infer middleware chain.
///
/// Call `infer()` to pass control to the inner layer.
/// Don't call it to short-circuit (guardrail halt).
#[async_trait]
pub trait InferNext: Send + Sync {
    /// Forward the inference request to the next layer.
    async fn infer(&self, request: InferRequest) -> Result<InferResponse, ProviderError>;
}

/// Middleware wrapping `Provider::infer`.
///
/// Code before `next.infer()` = pre-processing (request mutation, logging).
/// Code after `next.infer()` = post-processing (response mutation, metrics).
/// Not calling `next.infer()` = short-circuit (guardrail halt, cached response).
///
/// Use for: token budget enforcement, content filtering, request logging,
/// response caching, cost tracking.
#[async_trait]
pub trait InferMiddleware: Send + Sync {
    /// Intercept an inference call.
    async fn infer(
        &self,
        request: InferRequest,
        next: &dyn InferNext,
    ) -> Result<InferResponse, ProviderError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EMBED MIDDLEWARE (wraps Provider::embed)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in an embed middleware chain.
///
/// Call `embed()` to pass control to the inner layer.
/// Don't call it to short-circuit (guardrail halt).
#[async_trait]
pub trait EmbedNext: Send + Sync {
    /// Forward the embed request to the next layer.
    async fn embed(&self, request: EmbedRequest) -> Result<EmbedResponse, ProviderError>;
}

/// Middleware wrapping `Provider::embed`.
///
/// Code before `next.embed()` = pre-processing (request mutation, logging).
/// Code after `next.embed()` = post-processing (response mutation, metrics).
/// Not calling `next.embed()` = short-circuit (guardrail halt, cached response).
///
/// Use for: dimensionality enforcement, embedding caching, cost tracking,
/// input normalization.
#[async_trait]
pub trait EmbedMiddleware: Send + Sync {
    /// Intercept an embed call.
    async fn embed(
        &self,
        request: EmbedRequest,
        next: &dyn EmbedNext,
    ) -> Result<EmbedResponse, ProviderError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// INFER STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for inference operations.
///
/// Built via [`InferStack::builder()`]. Stacking order:
/// Observers (outermost) → Transformers → Guards (innermost).
///
/// Observers always run (even if a guard halts) because they're
/// the outermost layer. Guards see transformed input because
/// transformers are between observers and guards.
pub struct InferStack {
    /// Middleware layers in call order (outermost first).
    layers: Vec<Arc<dyn InferMiddleware>>,
}

/// Builder for [`InferStack`].
pub struct InferStackBuilder {
    observers: Vec<Arc<dyn InferMiddleware>>,
    transformers: Vec<Arc<dyn InferMiddleware>>,
    guards: Vec<Arc<dyn InferMiddleware>>,
}

impl InferStack {
    /// Start building an infer middleware stack.
    pub fn builder() -> InferStackBuilder {
        InferStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Infer through the middleware chain, ending at `terminal`.
    pub async fn infer_with(
        &self,
        request: InferRequest,
        terminal: &dyn InferNext,
    ) -> Result<InferResponse, ProviderError> {
        if self.layers.is_empty() {
            return terminal.infer(request).await;
        }
        let chain = InferChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.infer(request).await
    }
}

impl InferStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn InferMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware (mutates input/output, always calls next).
    pub fn transform(mut self, mw: Arc<dyn InferMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit by not calling next).
    pub fn guard(mut self, mw: Arc<dyn InferMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> InferStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        InferStack { layers }
    }
}

struct InferChain<'a> {
    layers: &'a [Arc<dyn InferMiddleware>],
    index: usize,
    terminal: &'a dyn InferNext,
}

#[async_trait]
impl InferNext for InferChain<'_> {
    async fn infer(&self, request: InferRequest) -> Result<InferResponse, ProviderError> {
        if self.index >= self.layers.len() {
            return self.terminal.infer(request).await;
        }
        let next = InferChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].infer(request, &next).await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EMBED STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for embedding operations.
///
/// Built via [`EmbedStack::builder()`]. Same observer/transform/guard
/// ordering as [`InferStack`].
pub struct EmbedStack {
    /// Middleware layers in call order (outermost first).
    layers: Vec<Arc<dyn EmbedMiddleware>>,
}

/// Builder for [`EmbedStack`].
pub struct EmbedStackBuilder {
    observers: Vec<Arc<dyn EmbedMiddleware>>,
    transformers: Vec<Arc<dyn EmbedMiddleware>>,
    guards: Vec<Arc<dyn EmbedMiddleware>>,
}

impl EmbedStack {
    /// Start building an embed middleware stack.
    pub fn builder() -> EmbedStackBuilder {
        EmbedStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Embed through the middleware chain, ending at `terminal`.
    pub async fn embed_with(
        &self,
        request: EmbedRequest,
        terminal: &dyn EmbedNext,
    ) -> Result<EmbedResponse, ProviderError> {
        if self.layers.is_empty() {
            return terminal.embed(request).await;
        }
        let chain = EmbedChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.embed(request).await
    }
}

impl EmbedStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn EmbedMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware.
    pub fn transform(mut self, mw: Arc<dyn EmbedMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit).
    pub fn guard(mut self, mw: Arc<dyn EmbedMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> EmbedStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        EmbedStack { layers }
    }
}

struct EmbedChain<'a> {
    layers: &'a [Arc<dyn EmbedMiddleware>],
    index: usize,
    terminal: &'a dyn EmbedNext,
}

#[async_trait]
impl EmbedNext for EmbedChain<'_> {
    async fn embed(&self, request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
        if self.index >= self.layers.len() {
            return self.terminal.embed(request).await;
        }
        let next = EmbedChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].embed(request, &next).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{EmbedRequest, EmbedResponse, Embedding};
    use crate::infer::{InferRequest, InferResponse};
    use crate::provider::ProviderError;
    use crate::types::{StopReason, TokenUsage};
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use std::sync::Arc;

    /// Helper: create a minimal InferRequest.
    fn make_infer_request() -> InferRequest {
        InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
    }

    /// Helper: create a minimal InferResponse.
    fn make_infer_response() -> InferResponse {
        InferResponse {
            content: Content::text("response"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test-model".into(),
            cost: None,
            truncated: None,
        }
    }

    /// Helper: create a minimal EmbedRequest.
    fn make_embed_request() -> EmbedRequest {
        EmbedRequest::new(vec!["hello".into()])
    }

    /// Helper: create a minimal EmbedResponse.
    fn make_embed_response() -> EmbedResponse {
        EmbedResponse {
            embeddings: vec![Embedding {
                vector: vec![0.1, 0.2, 0.3],
            }],
            model: "test-model".into(),
            usage: TokenUsage::default(),
        }
    }

    #[tokio::test]
    async fn infer_stack_empty_passthrough() {
        // An empty InferStack (no middleware) passes through directly to the terminal.
        let stack = InferStack::builder().build();

        struct EchoTerminal;

        #[async_trait]
        impl InferNext for EchoTerminal {
            async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
                Ok(make_infer_response())
            }
        }

        let result = stack.infer_with(make_infer_request(), &EchoTerminal).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().model, "test-model");
    }

    #[tokio::test]
    async fn infer_middleware_is_object_safe() {
        struct TagMiddleware;

        #[async_trait]
        impl InferMiddleware for TagMiddleware {
            async fn infer(
                &self,
                mut request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                request
                    .provider_options
                    .insert("test".to_string(), serde_json::json!({"tagged": true}));
                next.infer(request).await
            }
        }

        let _mw: Box<dyn InferMiddleware> = Box::new(TagMiddleware);
    }

    #[tokio::test]
    async fn embed_middleware_is_object_safe() {
        struct TagMiddleware;

        #[async_trait]
        impl EmbedMiddleware for TagMiddleware {
            async fn embed(
                &self,
                request: EmbedRequest,
                next: &dyn EmbedNext,
            ) -> Result<EmbedResponse, ProviderError> {
                next.embed(request).await
            }
        }

        let _mw: Box<dyn EmbedMiddleware> = Box::new(TagMiddleware);
    }

    #[tokio::test]
    async fn infer_stack_observer_always_runs() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let counter = Arc::new(AtomicU32::new(0));

        struct CountObserver(Arc<AtomicU32>);

        #[async_trait]
        impl InferMiddleware for CountObserver {
            async fn infer(
                &self,
                request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.infer(request).await
            }
        }

        struct HaltGuard;

        #[async_trait]
        impl InferMiddleware for HaltGuard {
            async fn infer(
                &self,
                _request: InferRequest,
                _next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                Err(ProviderError::ContentBlocked {
                    message: "budget exceeded".into(),
                })
            }
        }

        let stack = InferStack::builder()
            .observe(Arc::new(CountObserver(counter.clone())))
            .guard(Arc::new(HaltGuard))
            .build();

        struct EchoTerminal;

        #[async_trait]
        impl InferNext for EchoTerminal {
            async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
                Ok(make_infer_response())
            }
        }

        let result = stack.infer_with(make_infer_request(), &EchoTerminal).await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn infer_stack_transform_then_terminal() {
        struct ModelOverride;

        #[async_trait]
        impl InferMiddleware for ModelOverride {
            async fn infer(
                &self,
                mut request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                request.model = Some("overridden-model".into());
                next.infer(request).await
            }
        }

        struct AssertTerminal;

        #[async_trait]
        impl InferNext for AssertTerminal {
            async fn infer(&self, request: InferRequest) -> Result<InferResponse, ProviderError> {
                assert_eq!(request.model.as_deref(), Some("overridden-model"));
                Ok(make_infer_response())
            }
        }

        let stack = InferStack::builder()
            .transform(Arc::new(ModelOverride))
            .build();

        let result = stack
            .infer_with(make_infer_request(), &AssertTerminal)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn embed_stack_passthrough() {
        struct LogEmbed;

        #[async_trait]
        impl EmbedMiddleware for LogEmbed {
            async fn embed(
                &self,
                request: EmbedRequest,
                next: &dyn EmbedNext,
            ) -> Result<EmbedResponse, ProviderError> {
                next.embed(request).await
            }
        }

        struct EchoTerminal;

        #[async_trait]
        impl EmbedNext for EchoTerminal {
            async fn embed(&self, _request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
                Ok(make_embed_response())
            }
        }

        let stack = EmbedStack::builder().observe(Arc::new(LogEmbed)).build();

        let result = stack.embed_with(make_embed_request(), &EchoTerminal).await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.embeddings.len(), 1);
        assert_eq!(resp.model, "test-model");
    }
}
