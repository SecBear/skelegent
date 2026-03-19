//! Model routing provider — selects a backend per-request based on pluggable policy.
//!
//! The [`Provider`] trait uses RPITIT and is not object-safe.
//! [`DynProvider`] wraps any `Provider` into a boxed-future form so
//! [`RoutingProvider`] can hold a heterogeneous list of backends.
//!
//! # Example
//!
//! ```ignore
//! use skg_provider_router::{RoutingProvider, ModelMapPolicy};
//!
//! let policy = ModelMapPolicy::new()
//!     .route("claude-opus", 0)
//!     .route("claude-haiku", 1);
//!
//! let router = RoutingProvider::new(
//!     vec![box_provider(anthropic), box_provider(openai)],
//!     0,
//!     Box::new(policy),
//! ).unwrap();
//! ```

use std::collections::HashMap;
use std::future::Future;

use async_trait::async_trait;
use skg_turn::embedding::{EmbedRequest, EmbedResponse};
use skg_turn::infer::{InferRequest, InferResponse};
use skg_turn::infer_middleware::{EmbedNext, EmbedStack, InferNext, InferStack};
use skg_turn::provider::{Provider, ProviderError};
use skg_turn::stream::{InferStream, single_response_stream};

// Re-export DynProvider and box_provider for backwards compatibility.
pub use skg_turn::provider::{DynProvider, box_provider};

// ---------------------------------------------------------------------------
// RoutingPolicy
// ---------------------------------------------------------------------------

/// Policy that selects a provider for each inference request.
///
/// Implementations range from static rules (model string → provider index)
/// to LLM-driven classification (cheap model classifies task, routes to tier).
pub trait RoutingPolicy: Send + Sync {
    /// Select a provider index for this infer request.
    ///
    /// Returns an index into the [`RoutingProvider`]'s provider list.
    /// If `None`, the default provider is used.
    fn select(&self, request: &InferRequest) -> Option<usize>;

    /// Select a provider index for this embed request.
    ///
    /// Returns an index into the [`RoutingProvider`]'s provider list.
    /// The default implementation always returns `0`, routing to the first
    /// provider. Override to route embedding requests to a dedicated backend.
    fn select_embed(&self, _request: &EmbedRequest) -> usize {
        0
    }
}

// ---------------------------------------------------------------------------
// ModelMapPolicy
// ---------------------------------------------------------------------------

/// Routes requests by matching `InferRequest.model` against a string map.
///
/// Unknown models (or requests with `model: None`) fall through to the
/// default provider.
#[derive(Debug, Clone, Default)]
pub struct ModelMapPolicy {
    map: HashMap<String, usize>,
}

impl ModelMapPolicy {
    /// Create an empty policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a model name to a provider index. Builder-style.
    pub fn route(mut self, model: impl Into<String>, provider_idx: usize) -> Self {
        self.map.insert(model.into(), provider_idx);
        self
    }
}

impl RoutingPolicy for ModelMapPolicy {
    fn select(&self, request: &InferRequest) -> Option<usize> {
        request
            .model
            .as_deref()
            .and_then(|m| self.map.get(m).copied())
    }
}

// ---------------------------------------------------------------------------
// RoutingProvider
// ---------------------------------------------------------------------------

/// Error returned when constructing a [`RoutingProvider`] with invalid config.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// Provider list must not be empty.
    #[error("provider list must not be empty")]
    EmptyProviders,

    /// Default index is out of bounds.
    #[error("default_idx {idx} out of bounds (have {len} providers)")]
    DefaultOutOfBounds {
        /// The invalid index.
        idx: usize,
        /// Number of providers.
        len: usize,
    },
}

/// A [`Provider`] that dispatches each request to one of several backends
/// based on a [`RoutingPolicy`].
pub struct RoutingProvider {
    providers: Vec<Box<dyn DynProvider>>,
    default_idx: usize,
    policy: Box<dyn RoutingPolicy>,
}

impl std::fmt::Debug for RoutingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingProvider")
            .field("providers_count", &self.providers.len())
            .field("default_idx", &self.default_idx)
            .finish_non_exhaustive()
    }
}

impl RoutingProvider {
    /// Build a router.
    ///
    /// # Errors
    ///
    /// Returns [`RouterError::EmptyProviders`] if `providers` is empty, or
    /// [`RouterError::DefaultOutOfBounds`] if `default_idx >= providers.len()`.
    pub fn new(
        providers: Vec<Box<dyn DynProvider>>,
        default_idx: usize,
        policy: Box<dyn RoutingPolicy>,
    ) -> Result<Self, RouterError> {
        if providers.is_empty() {
            return Err(RouterError::EmptyProviders);
        }
        if default_idx >= providers.len() {
            return Err(RouterError::DefaultOutOfBounds {
                idx: default_idx,
                len: providers.len(),
            });
        }
        Ok(Self {
            providers,
            default_idx,
            policy,
        })
    }

    /// Resolve which provider index handles this infer request.
    fn resolve_idx(&self, request: &InferRequest) -> usize {
        self.policy
            .select(request)
            .filter(|&idx| idx < self.providers.len())
            .unwrap_or(self.default_idx)
    }

    /// Resolve which provider index handles this embed request.
    fn resolve_embed_idx(&self, request: &EmbedRequest) -> usize {
        let idx = self.policy.select_embed(request);
        if idx < self.providers.len() {
            idx
        } else {
            self.default_idx
        }
    }
}

impl Provider for RoutingProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
        let idx = self.resolve_idx(&request);
        // Safety: idx is bounds-checked in resolve_idx.
        self.providers[idx].infer_boxed(request)
    }

    fn embed(
        &self,
        request: EmbedRequest,
    ) -> impl Future<Output = Result<EmbedResponse, ProviderError>> + Send {
        let idx = self.resolve_embed_idx(&request);
        // Safety: idx is bounds-checked in resolve_embed_idx.
        self.providers[idx].embed_boxed(request)
    }
}

// ---------------------------------------------------------------------------
// MiddlewareProvider — DynProvider wrapped with middleware stacks
// ---------------------------------------------------------------------------

/// Terminal adapter that bridges `&dyn DynProvider` to [`InferNext`].
///
/// Used to connect the tail of an [`InferStack`] to the actual provider.
struct DynInferTerminal<'a> {
    inner: &'a dyn DynProvider,
}

#[async_trait]
impl InferNext for DynInferTerminal<'_> {
    async fn infer(&self, request: InferRequest) -> Result<InferResponse, ProviderError> {
        self.inner.infer_boxed(request).await
    }
}

/// Terminal adapter that bridges `&dyn DynProvider` to [`EmbedNext`].
///
/// Used to connect the tail of an [`EmbedStack`] to the actual provider.
struct DynEmbedTerminal<'a> {
    inner: &'a dyn DynProvider,
}

#[async_trait]
impl EmbedNext for DynEmbedTerminal<'_> {
    async fn embed(&self, request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
        self.inner.embed_boxed(request).await
    }
}

/// A provider wrapped with inference and embedding middleware stacks.
///
/// Wraps any [`DynProvider`] with an [`InferStack`] and an [`EmbedStack`],
/// allowing cross-cutting concerns (logging, caching, guardrails) to be
/// layered around any backend without modifying it.
///
/// `MiddlewareProvider` implements [`Provider`], so it automatically satisfies
/// [`DynProvider`] via the blanket impl.
pub struct MiddlewareProvider {
    inner: Box<dyn DynProvider>,
    infer_stack: InferStack,
    embed_stack: EmbedStack,
}

impl MiddlewareProvider {
    /// Create a `MiddlewareProvider` with empty (passthrough) middleware stacks.
    pub fn new(inner: Box<dyn DynProvider>) -> Self {
        Self {
            inner,
            infer_stack: InferStack::builder().build(),
            embed_stack: EmbedStack::builder().build(),
        }
    }

    /// Replace the inference middleware stack.
    pub fn with_infer_stack(mut self, stack: InferStack) -> Self {
        self.infer_stack = stack;
        self
    }

    /// Replace the embedding middleware stack.
    pub fn with_embed_stack(mut self, stack: EmbedStack) -> Self {
        self.embed_stack = stack;
        self
    }

    /// Run inference through the middleware stack, terminating at the inner provider.
    async fn infer_via_stack(&self, request: InferRequest) -> Result<InferResponse, ProviderError> {
        let terminal = DynInferTerminal {
            inner: self.inner.as_ref(),
        };
        self.infer_stack.infer_with(request, &terminal).await
    }

    /// Run embedding through the middleware stack, terminating at the inner provider.
    async fn embed_via_stack(&self, request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
        let terminal = DynEmbedTerminal {
            inner: self.inner.as_ref(),
        };
        self.embed_stack.embed_with(request, &terminal).await
    }
}

impl std::fmt::Debug for MiddlewareProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MiddlewareProvider").finish_non_exhaustive()
    }
}

impl Provider for MiddlewareProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
        self.infer_via_stack(request)
    }

    fn embed(
        &self,
        request: EmbedRequest,
    ) -> impl Future<Output = Result<EmbedResponse, ProviderError>> + Send {
        self.embed_via_stack(request)
    }

    /// Stream inference through the middleware stack, returning a single-event stream.
    ///
    /// Runs the request through the full [`InferStack`] (guards, transformers, observers)
    /// via [`Self::infer_via_stack`], then wraps the response in a single [`StreamEvent::Done`].
    ///
    /// Providers that support native streaming should be used directly rather than wrapped
    /// in `MiddlewareProvider`. The `InferStack` calls `infer()` on the inner provider,
    /// which is not streaming-aware.
    fn infer_stream(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferStream, ProviderError>> + Send {
        let stack_future = self.infer_via_stack(request);
        async move {
            let response = stack_future.await?;
            Ok(single_response_stream(response))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use skg_turn::infer_middleware::{EmbedMiddleware, InferMiddleware};
    use skg_turn::stream::StreamEvent;
    use skg_turn::types::{StopReason, TokenUsage};
    use std::sync::{Arc, Mutex};

    // -- Mock provider that records calls ------------------------------------

    struct RecordingProvider {
        name: String,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingProvider {
        fn new(name: impl Into<String>, calls: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name: name.into(),
                calls,
            }
        }
    }

    impl Provider for RecordingProvider {
        fn infer(
            &self,
            request: InferRequest,
        ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
            let model = request.model.clone().unwrap_or_default();
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:{}", self.name, model));

            async move {
                Ok(InferResponse {
                    content: Content::text("ok"),
                    tool_calls: vec![],
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                    model: model.clone(),
                    cost: None,
                    truncated: None,
                })
            }
        }
    }

    // -- Policy tests --------------------------------------------------------

    #[test]
    fn model_map_selects_correct_index() {
        let policy = ModelMapPolicy::new()
            .route("claude-opus", 0)
            .route("claude-haiku", 1);

        let req = InferRequest::new(vec![]).with_model("claude-opus");
        assert_eq!(policy.select(&req), Some(0));

        let req = InferRequest::new(vec![]).with_model("claude-haiku");
        assert_eq!(policy.select(&req), Some(1));
    }

    #[test]
    fn model_map_returns_none_for_unknown() {
        let policy = ModelMapPolicy::new().route("claude-opus", 0);

        let req = InferRequest::new(vec![]).with_model("gpt-4o");
        assert_eq!(policy.select(&req), None);
    }

    #[test]
    fn model_map_returns_none_when_no_model() {
        let policy = ModelMapPolicy::new().route("claude-opus", 0);
        let req = InferRequest::new(vec![]);
        assert_eq!(policy.select(&req), None);
    }

    // -- Router construction -------------------------------------------------

    #[test]
    fn empty_providers_rejected() {
        let policy = ModelMapPolicy::new();
        let err = RoutingProvider::new(vec![], 0, Box::new(policy)).unwrap_err();
        assert!(matches!(err, RouterError::EmptyProviders));
    }

    #[test]
    fn default_out_of_bounds_rejected() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let providers: Vec<Box<dyn DynProvider>> =
            vec![box_provider(RecordingProvider::new("a", calls))];
        let policy = ModelMapPolicy::new();
        let err = RoutingProvider::new(providers, 5, Box::new(policy)).unwrap_err();
        assert!(matches!(err, RouterError::DefaultOutOfBounds { .. }));
    }

    // -- Routing dispatch ----------------------------------------------------

    #[tokio::test]
    async fn routes_to_correct_provider() {
        let calls = Arc::new(Mutex::new(Vec::new()));

        let providers: Vec<Box<dyn DynProvider>> = vec![
            box_provider(RecordingProvider::new("anthropic", Arc::clone(&calls))),
            box_provider(RecordingProvider::new("openai", Arc::clone(&calls))),
        ];

        let policy = ModelMapPolicy::new()
            .route("claude-opus", 0)
            .route("gpt-4o", 1);

        let router = RoutingProvider::new(providers, 0, Box::new(policy)).unwrap();

        let msg = Message::new(Role::User, Content::text("hi"));

        // Route to anthropic (index 0)
        let req = InferRequest::new(vec![msg.clone()]).with_model("claude-opus");
        let _ = router.infer(req).await.unwrap();

        // Route to openai (index 1)
        let req = InferRequest::new(vec![msg.clone()]).with_model("gpt-4o");
        let _ = router.infer(req).await.unwrap();

        // Unknown model → default (index 0 = anthropic)
        let req = InferRequest::new(vec![msg]).with_model("llama-3");
        let _ = router.infer(req).await.unwrap();

        let log = calls.lock().unwrap();
        assert_eq!(
            *log,
            vec![
                "anthropic:claude-opus",
                "openai:gpt-4o",
                "anthropic:llama-3",
            ]
        );
    }

    #[tokio::test]
    async fn out_of_bounds_policy_falls_back_to_default() {
        let calls = Arc::new(Mutex::new(Vec::new()));

        let providers: Vec<Box<dyn DynProvider>> = vec![box_provider(RecordingProvider::new(
            "only",
            Arc::clone(&calls),
        ))];

        // Policy returns index 99 which is out of bounds
        struct BadPolicy;
        impl RoutingPolicy for BadPolicy {
            fn select(&self, _: &InferRequest) -> Option<usize> {
                Some(99)
            }
        }

        let router = RoutingProvider::new(providers, 0, Box::new(BadPolicy)).unwrap();
        let req = InferRequest::new(vec![]).with_model("anything");
        let _ = router.infer(req).await.unwrap();

        let log = calls.lock().unwrap();
        assert_eq!(*log, vec!["only:anything"]);
    }

    // -- Embed routing -------------------------------------------------------

    /// A provider that supports embedding — records the texts it receives.
    struct RecordingEmbedProvider {
        name: String,
        texts: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingEmbedProvider {
        fn new(name: impl Into<String>, texts: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name: name.into(),
                texts,
            }
        }
    }

    impl Provider for RecordingEmbedProvider {
        async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
            Err(ProviderError::Other("not supported".into()))
        }

        fn embed(
            &self,
            request: EmbedRequest,
        ) -> impl Future<Output = Result<EmbedResponse, ProviderError>> + Send {
            use skg_turn::embedding::Embedding;
            use skg_turn::types::TokenUsage;

            let name = self.name.clone();
            *self.texts.lock().unwrap() = request.texts.clone();
            async move {
                Ok(EmbedResponse {
                    embeddings: request
                        .texts
                        .iter()
                        .map(|_| Embedding { vector: vec![1.0] })
                        .collect(),
                    model: name,
                    usage: TokenUsage::default(),
                })
            }
        }
    }

    #[tokio::test]
    async fn routing_provider_routes_embed() {
        let texts_0 = Arc::new(Mutex::new(Vec::new()));
        let texts_1 = Arc::new(Mutex::new(Vec::new()));

        let providers: Vec<Box<dyn DynProvider>> = vec![
            box_provider(RecordingEmbedProvider::new(
                "embed-backend-0",
                Arc::clone(&texts_0),
            )),
            box_provider(RecordingEmbedProvider::new(
                "embed-backend-1",
                Arc::clone(&texts_1),
            )),
        ];

        // Policy that routes embed to provider 1 explicitly.
        struct EmbedToOne;
        impl RoutingPolicy for EmbedToOne {
            fn select(&self, _: &InferRequest) -> Option<usize> {
                None
            }
            fn select_embed(&self, _: &EmbedRequest) -> usize {
                1
            }
        }

        let router = RoutingProvider::new(providers, 0, Box::new(EmbedToOne)).unwrap();

        let req = EmbedRequest::new(vec!["hello".into(), "world".into()]);
        let resp = router.embed(req).await.unwrap();

        // Response should come from provider 1.
        assert_eq!(resp.model, "embed-backend-1");
        assert_eq!(resp.embeddings.len(), 2);

        // Provider 0 should NOT have been called.
        assert!(texts_0.lock().unwrap().is_empty());
        // Provider 1 should have received our texts.
        assert_eq!(
            *texts_1.lock().unwrap(),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    // -- MiddlewareProvider tests --------------------------------------------

    /// MiddlewareProvider with empty stacks passes calls through to inner provider.
    #[tokio::test]
    async fn middleware_provider_passthrough() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let inner = box_provider(RecordingProvider::new("inner", Arc::clone(&calls)));
        let mp = MiddlewareProvider::new(inner);

        let req = InferRequest::new(vec![]).with_model("test-model");
        let resp = mp.infer(req).await.unwrap();

        assert_eq!(resp.model, "test-model");
        let log = calls.lock().unwrap();
        assert_eq!(*log, vec!["inner:test-model"]);
    }

    /// A transformer middleware that modifies the request model field is applied.
    #[tokio::test]
    async fn middleware_provider_infer_transform() {
        use async_trait::async_trait;

        struct ModelRenamer;

        #[async_trait]
        impl InferMiddleware for ModelRenamer {
            async fn infer(
                &self,
                mut request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                request.model = Some("renamed-model".into());
                next.infer(request).await
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let inner = box_provider(RecordingProvider::new("inner", Arc::clone(&calls)));

        let stack = InferStack::builder()
            .transform(Arc::new(ModelRenamer))
            .build();

        let mp = MiddlewareProvider::new(inner).with_infer_stack(stack);

        let req = InferRequest::new(vec![]).with_model("original-model");
        let resp = mp.infer(req).await.unwrap();

        // The response model comes from what RecordingProvider echoes back
        assert_eq!(resp.model, "renamed-model");
        let log = calls.lock().unwrap();
        // The inner provider should have received the renamed model
        assert_eq!(*log, vec!["inner:renamed-model"]);
    }

    /// An embed transformer middleware that modifies the request is applied.
    #[tokio::test]
    async fn middleware_provider_embed_transform() {
        use async_trait::async_trait;
        use skg_turn::embedding::Embedding;

        struct InputNormalizer;

        #[async_trait]
        impl EmbedMiddleware for InputNormalizer {
            async fn embed(
                &self,
                mut request: EmbedRequest,
                next: &dyn EmbedNext,
            ) -> Result<EmbedResponse, ProviderError> {
                // Normalize: uppercase all texts.
                request.texts = request
                    .texts
                    .into_iter()
                    .map(|s| s.to_uppercase())
                    .collect();
                next.embed(request).await
            }
        }

        // A simple DynProvider that records the embed texts and returns a fixed response.
        struct RecordingEmbedProvider {
            texts: Arc<Mutex<Vec<String>>>,
        }

        impl Provider for RecordingEmbedProvider {
            async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
                Err(ProviderError::Other("not supported".into()))
            }

            async fn embed(&self, request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
                let recorded = request.texts.clone();
                *self.texts.lock().unwrap() = recorded;
                Ok(EmbedResponse {
                    embeddings: vec![Embedding { vector: vec![1.0] }],
                    model: "embed-model".into(),
                    usage: TokenUsage::default(),
                })
            }
        }

        let recorded_texts = Arc::new(Mutex::new(Vec::new()));
        let inner = box_provider(RecordingEmbedProvider {
            texts: Arc::clone(&recorded_texts),
        });

        let stack = EmbedStack::builder()
            .transform(Arc::new(InputNormalizer))
            .build();

        let mp = MiddlewareProvider::new(inner).with_embed_stack(stack);

        let req = EmbedRequest::new(vec!["hello world".into()]);
        let resp = mp.embed(req).await.unwrap();

        assert_eq!(resp.model, "embed-model");
        let texts = recorded_texts.lock().unwrap();
        assert_eq!(*texts, vec!["HELLO WORLD"]);
    }

    // -- infer_stream middleware integration ----------------------------------

    /// Middleware fires on the `infer_stream` path via the `InferStack`.
    #[tokio::test]
    async fn stream_infer_fires_middleware() {
        use async_trait::async_trait;
        use futures_util::StreamExt;
        use skg_turn::infer_middleware::InferNext;
        use std::sync::atomic::{AtomicU32, Ordering};

        // -- Provider that returns a fixed InferResponse ------------------
        struct StubProvider;

        impl Provider for StubProvider {
            fn infer(
                &self,
                request: InferRequest,
            ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
                async move {
                    Ok(InferResponse {
                        content: Content::text("streamed-ok"),
                        tool_calls: vec![],
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage::default(),
                        model: request.model.unwrap_or_default(),
                        cost: None,
                        truncated: None,
                    })
                }
            }
        }

        // -- Counting middleware that records invocations -----------------
        struct CountingMiddleware(Arc<AtomicU32>);

        #[async_trait]
        impl InferMiddleware for CountingMiddleware {
            async fn infer(
                &self,
                request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.infer(request).await
            }
        }

        let call_count = Arc::new(AtomicU32::new(0));

        let stack = InferStack::builder()
            .observe(Arc::new(CountingMiddleware(Arc::clone(&call_count))))
            .build();

        let mp = MiddlewareProvider::new(box_provider(StubProvider)).with_infer_stack(stack);

        let infer_req = InferRequest::new(vec![Message::new(Role::User, Content::text("hi"))])
            .with_model("test-model");
        let mut stream = mp.infer_stream(infer_req).await.unwrap();

        // The stream yields exactly one Done event wrapping the full response.
        let event = stream.next().await.expect("stream must yield Done").unwrap();
        let StreamEvent::Done(response) = event else {
            panic!("expected Done event, got {event:?}");
        };

        // Stream is exhausted after Done.
        assert!(stream.next().await.is_none());

        // Middleware was called exactly once (via infer_via_stack).
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Response carried through the Done event.
        assert_eq!(response.model, "test-model");
        assert_eq!(response.text().unwrap(), "streamed-ok");
    }
}
