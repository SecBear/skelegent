#![deny(missing_docs)]
//! Shared toolkit for building operators.
//!
//! Provides the [`Provider`] trait for making model calls,
//! and all the types needed by operator implementations.
//!
//! ## API
//!
//! Use [`InferRequest`] / [`InferResponse`] via [`Provider::infer()`].

pub mod config;
pub mod embedding;
pub mod infer;
pub mod infer_middleware;
pub mod provider;
pub mod safety;
pub mod stream;
pub mod token_counter;
pub mod types;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

// Re-exports
pub use config::TurnConfig;
pub use embedding::{EmbedRequest, EmbedResponse, Embedding};
pub use infer::{InferRequest, InferResponse, ToolCall};
pub use infer_middleware::{
    EmbedMiddleware, EmbedNext, EmbedStack, EmbedStackBuilder, InferMiddleware, InferNext,
    InferStack, InferStackBuilder,
};
pub use provider::{DynProvider, Provider, ProviderError, box_provider};
pub use safety::{allow_real_requests, assert_real_requests_allowed, deny_real_requests};
pub use stream::{InferStream, StreamEvent, single_response_stream};
pub use token_counter::{HeuristicTokenCounter, TokenCounter, limits};
pub use types::*;
