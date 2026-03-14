//! Public durable run control traits.

use crate::id::{RunId, WaitPointId};
use crate::model::RunView;
use crate::wait::ResumeInput;
use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

/// Portable error returned by durable run control operations.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum RunControlError {
    /// The requested run does not exist.
    #[error("run not found: {0}")]
    RunNotFound(RunId),
    /// The requested durable wait point does not exist.
    #[error("wait point not found: {0}")]
    WaitPointNotFound(WaitPointId),
    /// The request conflicts with the run's current state.
    #[error("run control conflict: {0}")]
    Conflict(String),
    /// The caller supplied invalid data.
    #[error("invalid run control input: {0}")]
    InvalidInput(String),
    /// Backend-specific failure surfaced through the portable control plane.
    #[error("run control backend error: {0}")]
    Backend(String),
}

/// Starts new durable runs.
///
/// JSON values are the current canonical portable payload format. Concrete
/// orchestrators may project richer start contracts on top once a stable
/// cross-backend shape emerges.
#[async_trait]
pub trait RunStarter: Send + Sync {
    /// Start a durable run and return its identifier once accepted.
    async fn start_run(&self, input: Value) -> Result<RunId, RunControlError>;
}

/// Inspects and controls existing durable runs.
#[async_trait]
pub trait RunController: Send + Sync {
    /// Fetch the current read model for a run.
    ///
    /// Returns [`RunControlError::RunNotFound`] when the run does not exist.
    async fn get_run(&self, run_id: &RunId) -> Result<RunView, RunControlError>;

    /// Send asynchronous control-plane data to a run.
    ///
    /// JSON values are the current canonical portable signal payload format.
    /// This is intentionally distinct from [`Self::resume_run`]. Signals do not
    /// satisfy a specific durable wait point.
    async fn signal_run(&self, run_id: &RunId, signal: Value) -> Result<(), RunControlError>;

    /// Satisfy a specific durable wait point with structured input.
    async fn resume_run(
        &self,
        run_id: &RunId,
        wait_point: &WaitPointId,
        input: ResumeInput,
    ) -> Result<(), RunControlError>;

    /// Cancel a durable run.
    async fn cancel_run(&self, run_id: &RunId) -> Result<(), RunControlError>;
}
