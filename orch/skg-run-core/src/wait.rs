//! Durable waiting and resume primitives.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Durable reason a run is waiting for external progress.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    /// Waiting for an approval or supervisor decision.
    Approval,
    /// Waiting for caller-provided input.
    ExternalInput,
    /// Waiting for a timer or wake-up deadline.
    Timer,
    /// Waiting for another durable run to make progress.
    ChildRun,
    /// Waiting for an implementation-specific reason.
    Custom(String),
}

/// Structured input used to satisfy a durable wait point.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeInput {
    /// Backend-neutral resume payload.
    ///
    /// JSON values are the current canonical portable payload format.
    pub payload: Value,
    /// Optional metadata that helps the backend or caller correlate the resume.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

impl ResumeInput {
    /// Create a new resume input with an arbitrary payload.
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            metadata: Map::new(),
        }
    }

    /// Add a metadata entry and return the updated resume input.
    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}
