//! Error types for the context engine.

use layer0::error::OperatorError;
use layer0::operator::Outcome;
use skg_tool::ToolError;
use skg_turn::provider::ProviderError;
use std::fmt;

/// Errors produced by context engine operations.
#[derive(Debug)]
pub enum EngineError {
    /// A rule or operation halted execution.
    Halted {
        /// Human-readable reason for the halt.
        reason: String,
    },
    /// A rule or runtime path requested a structured operator exit.
    Exit {
        /// Structured outcome that should propagate to operator output.
        outcome: Outcome,
        /// Human-readable detail for logging and debugging.
        detail: String,
    },
    /// Inference failed at the provider level.
    Provider(ProviderError),
    /// Layer0 operator error.
    Operator(OperatorError),
    /// Tool dispatch failed.
    Tool(ToolError),
    /// Catch-all for other errors.
    Custom(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Halted { reason } => write!(f, "halted: {reason}"),
            Self::Exit { outcome, detail } => write!(f, "exit {outcome}: {detail}"),
            Self::Provider(e) => write!(f, "provider: {e}"),
            Self::Operator(e) => write!(f, "operator: {e}"),
            Self::Tool(e) => write!(f, "tool: {e}"),
            Self::Custom(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provider(e) => Some(e),
            Self::Operator(e) => Some(e),
            Self::Tool(e) => Some(e),
            Self::Custom(e) => Some(e.as_ref()),
            Self::Halted { .. } => None,
            Self::Exit { .. } => None,
        }
    }
}

impl From<ProviderError> for EngineError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<OperatorError> for EngineError {
    fn from(e: OperatorError) -> Self {
        Self::Operator(e)
    }
}

impl From<ToolError> for EngineError {
    fn from(e: ToolError) -> Self {
        Self::Tool(e)
    }
}
