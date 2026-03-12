//! Lower-level durable timer persistence seams for DIY backends.

use crate::deadline::PortableWakeDeadline;
use crate::id::{RunId, WaitPointId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Durable wake-up scheduled for a run wait point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTimer {
    /// Run that should wake when the deadline becomes due.
    pub run_id: RunId,
    /// Wait point associated with the timer.
    pub wait_point: WaitPointId,
    /// Portable backend-neutral wake deadline.
    pub wake_at: PortableWakeDeadline,
}

impl ScheduledTimer {
    /// Create a new durable scheduled timer.
    pub fn new(run_id: RunId, wait_point: WaitPointId, wake_at: PortableWakeDeadline) -> Self {
        Self {
            run_id,
            wait_point,
            wake_at,
        }
    }
}

/// Portable error surfaced by durable timer persistence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimerStoreError {
    /// The requested timer does not exist.
    #[error("timer not found for run {run_id} wait point {wait_point}")]
    TimerNotFound {
        /// Run that was queried.
        run_id: RunId,
        /// Wait point that was queried.
        wait_point: WaitPointId,
    },
    /// The write conflicts with existing durable state.
    #[error("timer store conflict: {0}")]
    Conflict(String),
    /// Backend-specific failure surfaced through the portable seam.
    #[error("timer store backend error: {0}")]
    Backend(String),
}

/// Durable timer store for wake-up deadlines.
#[async_trait]
pub trait TimerStore: Send + Sync {
    /// Persist a wake-up deadline for a run wait point.
    async fn schedule_timer(&self, timer: ScheduledTimer) -> Result<(), TimerStoreError>;

    /// Cancel a previously scheduled wake-up deadline.
    async fn cancel_timer(
        &self,
        run_id: &RunId,
        wait_point: &WaitPointId,
    ) -> Result<(), TimerStoreError>;

    /// Return timers due at or before the supplied deadline, bounded by `limit`.
    async fn due_timers(
        &self,
        not_after: &PortableWakeDeadline,
        limit: usize,
    ) -> Result<Vec<ScheduledTimer>, TimerStoreError>;
}
