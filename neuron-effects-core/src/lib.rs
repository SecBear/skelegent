#![deny(missing_docs)]
//! Core effect execution primitives: trait and error types.

use async_trait::async_trait;
use layer0::effect::Effect;
use layer0::error::{OrchError, StateError};
use thiserror::Error;

/// Error type for effect execution.
#[derive(Debug, Error)]
pub enum Error {
    /// Orchestrator error.
    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] OrchError),
    /// State backend error.
    #[error("state error: {0}")]
    State(#[from] StateError),
    /// An unknown/unhandled effect was encountered and policy is Error.
    #[error("unknown or unsupported effect encountered")]
    UnknownEffect,
}

/// Policy for handling unknown/custom effects.
#[derive(Debug, Clone, Copy, Default)]
pub enum UnknownEffectPolicy {
    /// Ignore unknown/custom effects and log a warning via `tracing::warn`.
    #[default]
    IgnoreAndWarn,
    /// Treat unknown/custom effects as an error.
    Error,
}

/// Execute Layer 0 effects in a deterministic, ordered fashion.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute a list of effects in order. Implementations must preserve
    /// the provided order and short-circuit on the first error.
    async fn execute(&self, effects: &[Effect]) -> Result<(), Error>;
}
