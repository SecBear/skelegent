#![deny(missing_docs)]
//! Shared toolkit for building operators.
//!
//! Provides the [`Provider`] trait for making model calls,
//! [`ContextStrategy`] for managing context between calls,
//! and all the types needed by operator implementations.

pub mod config;
pub mod context;
pub mod convert;
pub mod provider;
pub mod types;

// Re-exports
pub use config::NeuronTurnConfig;
pub use context::{ContextStrategy, NoCompaction};
pub use convert::{
    content_block_to_part, content_part_to_block, content_to_parts, content_to_user_message,
    parts_to_content,
};
pub use provider::{Provider, ProviderError};
pub use types::*;
