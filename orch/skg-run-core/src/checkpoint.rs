//! Execution checkpoints for session rewind and time-travel replay.
//!
//! A [`Checkpoint`] captures execution state at a step boundary — which
//! operator just ran, what state looked like, and where in the execution
//! sequence this occurred. The [`CheckpointStore`] trait provides persistence.
//!
//! # Design
//!
//! Checkpoints form a linked list via `parent`. Walking the parent chain
//! reconstructs the full execution history. Branching (replaying from an
//! earlier checkpoint) creates a new chain that shares a common prefix.

use crate::id::{CheckpointId, RunId};
use async_trait::async_trait;
use layer0::id::OperatorId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A snapshot of execution state at a step boundary.
///
/// Created by orchestrators at each dispatch boundary. The `state` field
/// carries serialized execution context (conversation, metrics, pending
/// effects) — the format is orchestrator-defined, not standardized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique checkpoint identifier.
    pub id: CheckpointId,
    /// The run this checkpoint belongs to.
    pub run_id: RunId,
    /// Step number in the execution sequence (0-indexed).
    pub step: u32,
    /// The operator that produced this checkpoint (just completed or about to run).
    pub operator_id: OperatorId,
    /// Serialized execution state. Format is orchestrator-defined.
    pub state: serde_json::Value,
    /// Previous checkpoint in the chain. `None` for the initial checkpoint.
    pub parent: Option<CheckpointId>,
    /// Unix timestamp in milliseconds.
    pub created_at: u64,
}

impl Checkpoint {
    /// Create a new checkpoint.
    pub fn new(
        id: impl Into<CheckpointId>,
        run_id: impl Into<RunId>,
        step: u32,
        operator_id: impl Into<OperatorId>,
        state: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            run_id: run_id.into(),
            step,
            operator_id: operator_id.into(),
            state,
            parent: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Set the parent checkpoint.
    pub fn with_parent(mut self, parent: impl Into<CheckpointId>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

/// Errors from checkpoint storage operations.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Checkpoint not found.
    #[error("checkpoint not found: {0}")]
    NotFound(String),
    /// Storage backend failure.
    #[error("checkpoint store error: {0}")]
    Store(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence backend for execution checkpoints.
///
/// Implementations: SQLite (`skg-run-sqlite` in extras), in-memory (tests).
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Save a checkpoint. Returns the checkpoint ID.
    async fn save_checkpoint(
        &self,
        checkpoint: Checkpoint,
    ) -> Result<CheckpointId, CheckpointError>;

    /// Retrieve a checkpoint by ID.
    async fn get_checkpoint(
        &self,
        id: &CheckpointId,
    ) -> Result<Option<Checkpoint>, CheckpointError>;

    /// List all checkpoints for a run, ordered by step.
    async fn list_checkpoints(&self, run_id: &RunId) -> Result<Vec<Checkpoint>, CheckpointError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_serde_round_trip() {
        let cp = Checkpoint::new(
            "cp-1",
            "run-1",
            0,
            "operator-a",
            serde_json::json!({"messages": []}),
        )
        .with_parent("cp-0");
        let json = serde_json::to_string(&cp).unwrap();
        let back: Checkpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id.as_str(), "cp-1");
        assert_eq!(back.run_id.as_str(), "run-1");
        assert_eq!(back.step, 0);
        assert_eq!(back.operator_id.as_str(), "operator-a");
        assert_eq!(back.parent.as_ref().unwrap().as_str(), "cp-0");
    }

    #[test]
    fn checkpoint_without_parent() {
        let cp = Checkpoint::new("cp-1", "run-1", 0, "op", serde_json::Value::Null);
        assert!(cp.parent.is_none());
    }

    #[test]
    fn checkpoint_id_display() {
        let id = CheckpointId::new("cp-abc");
        assert_eq!(id.to_string(), "cp-abc");
    }
}
