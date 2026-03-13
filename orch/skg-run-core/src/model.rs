//! Portable durable run read models.

use crate::id::{RunId, WaitPointId};
use crate::wait::WaitReason;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current lifecycle state for a durable run.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run is currently executing.
    Running,
    /// The run is blocked on a durable wait point.
    Waiting,
    /// The run completed successfully.
    Completed,
    /// The run reached a terminal failure.
    Failed,
    /// The run was cancelled.
    Cancelled,
}

/// Portable summary of a terminal run outcome.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunOutcome {
    /// The run completed and produced a backend-neutral result payload.
    Completed {
        /// Terminal result payload.
        result: Value,
    },
    /// The run failed with a backend-neutral error summary.
    Failed {
        /// Human-readable failure summary.
        error: String,
    },
    /// The run was cancelled before completion.
    Cancelled,
}

impl RunOutcome {
    /// Return the terminal status represented by this outcome.
    pub fn status(&self) -> RunStatus {
        match self {
            Self::Completed { .. } => RunStatus::Completed,
            Self::Failed { .. } => RunStatus::Failed,
            Self::Cancelled => RunStatus::Cancelled,
        }
    }
}

/// Current read model for a durable run.
///
/// The enum shape makes impossible state combinations unrepresentable: waiting
/// runs always carry a wait point and reason, while terminal runs always carry
/// terminal data.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunView {
    /// The run is actively executing.
    Running {
        /// Durable run identifier.
        run_id: RunId,
    },
    /// The run is blocked on a specific durable wait point.
    Waiting {
        /// Durable run identifier.
        run_id: RunId,
        /// Active wait point that must be resumed.
        wait_point: WaitPointId,
        /// Reason the run is currently blocked.
        wait_reason: WaitReason,
    },
    /// The run completed successfully.
    Completed {
        /// Durable run identifier.
        run_id: RunId,
        /// Terminal result payload.
        result: Value,
    },
    /// The run reached a terminal failure.
    Failed {
        /// Durable run identifier.
        run_id: RunId,
        /// Human-readable failure summary.
        error: String,
    },
    /// The run was cancelled before completion.
    Cancelled {
        /// Durable run identifier.
        run_id: RunId,
    },
}

impl RunView {
    /// Create a running run view.
    pub fn running(run_id: RunId) -> Self {
        Self::Running { run_id }
    }

    /// Create a waiting run view for a specific durable wait point.
    pub fn waiting(run_id: RunId, wait_point: WaitPointId, wait_reason: WaitReason) -> Self {
        Self::Waiting {
            run_id,
            wait_point,
            wait_reason,
        }
    }

    /// Create a terminal run view from an outcome.
    pub fn terminal(run_id: RunId, outcome: RunOutcome) -> Self {
        match outcome {
            RunOutcome::Completed { result } => Self::Completed { run_id, result },
            RunOutcome::Failed { error } => Self::Failed { run_id, error },
            RunOutcome::Cancelled => Self::Cancelled { run_id },
        }
    }

    /// Borrow the durable run identifier.
    pub fn run_id(&self) -> &RunId {
        match self {
            Self::Running { run_id }
            | Self::Waiting { run_id, .. }
            | Self::Completed { run_id, .. }
            | Self::Failed { run_id, .. }
            | Self::Cancelled { run_id } => run_id,
        }
    }

    /// Return the current lifecycle state.
    pub fn status(&self) -> RunStatus {
        match self {
            Self::Running { .. } => RunStatus::Running,
            Self::Waiting { .. } => RunStatus::Waiting,
            Self::Completed { .. } => RunStatus::Completed,
            Self::Failed { .. } => RunStatus::Failed,
            Self::Cancelled { .. } => RunStatus::Cancelled,
        }
    }

    /// Borrow the active wait point when the run is waiting.
    pub fn wait_point(&self) -> Option<&WaitPointId> {
        match self {
            Self::Waiting { wait_point, .. } => Some(wait_point),
            _ => None,
        }
    }

    /// Borrow the active wait reason when the run is waiting.
    pub fn wait_reason(&self) -> Option<&WaitReason> {
        match self {
            Self::Waiting { wait_reason, .. } => Some(wait_reason),
            _ => None,
        }
    }

    /// Return the terminal outcome summary when the run has finished.
    pub fn outcome(&self) -> Option<RunOutcome> {
        match self {
            Self::Completed { result, .. } => Some(RunOutcome::Completed {
                result: result.clone(),
            }),
            Self::Failed { error, .. } => Some(RunOutcome::Failed {
                error: error.clone(),
            }),
            Self::Cancelled { .. } => Some(RunOutcome::Cancelled),
            Self::Running { .. } | Self::Waiting { .. } => None,
        }
    }
}
