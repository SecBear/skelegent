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
pub mod infer;
pub mod provider;
pub mod stream;
pub mod types;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

// Re-exports
pub use config::NeuronTurnConfig;
pub use infer::{InferRequest, InferResponse, ToolCall};
pub use provider::{Provider, ProviderError};
pub use types::*;
