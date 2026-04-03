//! Shared wait and resume nouns.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Shared reason an invocation is waiting for external progress.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    /// Waiting for approval.
    Approval,
    /// Waiting for caller-provided input.
    ExternalInput,
    /// Waiting for a timer/deadline.
    Timer,
    /// Waiting for a child run.
    ChildRun,
    /// Waiting for a custom reason.
    Custom(String),
}

/// Shared value-level fact that an invocation is suspended.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitState {
    /// Why execution is currently suspended.
    pub reason: WaitReason,
}

/// Structured input used to resume a suspended invocation.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeInput {
    /// Portable resume payload.
    pub payload: Value,
    /// Optional metadata payload.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

impl ResumeInput {
    /// Create resume input with an arbitrary payload.
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            metadata: Map::new(),
        }
    }

    /// Add a metadata entry.
    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}
