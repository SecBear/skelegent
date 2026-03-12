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
use std::pin::Pin;

use skg_turn::infer::{InferRequest, InferResponse};
use skg_turn::provider::{Provider, ProviderError};

// ---------------------------------------------------------------------------
// DynProvider — object-safe wrapper
// ---------------------------------------------------------------------------

/// Object-safe wrapper around [`Provider`].
///
/// [`Provider`] uses RPITIT (`-> impl Future`) which prevents `dyn Provider`.
/// This trait boxes the future so we can store heterogeneous providers in a `Vec`.
pub trait DynProvider: Send + Sync {
    /// Run inference, returning a boxed future.
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>>;
}

impl<P: Provider> DynProvider for P {
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>> {
        Box::pin(self.infer(request))
    }
}

/// Wrap a concrete [`Provider`] into a `Box<dyn DynProvider>`.
pub fn box_provider<P: Provider + 'static>(p: P) -> Box<dyn DynProvider> {
    Box::new(p)
}

// ---------------------------------------------------------------------------
// RoutingPolicy
// ---------------------------------------------------------------------------

/// Policy that selects a provider for each inference request.
///
/// Implementations range from static rules (model string → provider index)
/// to LLM-driven classification (cheap model classifies task, routes to tier).
pub trait RoutingPolicy: Send + Sync {
    /// Select a provider index for this request.
    ///
    /// Returns an index into the [`RoutingProvider`]'s provider list.
    /// If `None`, the default provider is used.
    fn select(&self, request: &InferRequest) -> Option<usize>;
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

    /// Resolve which provider index handles this request.
    fn resolve_idx(&self, request: &InferRequest) -> usize {
        self.policy
            .select(request)
            .filter(|&idx| idx < self.providers.len())
            .unwrap_or(self.default_idx)
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
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

        let providers: Vec<Box<dyn DynProvider>> =
            vec![box_provider(RecordingProvider::new("only", Arc::clone(&calls)))];

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
}
