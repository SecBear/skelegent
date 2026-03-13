//! Run observation and streaming update primitives.

use crate::control::RunControlError;
use crate::id::RunId;
use crate::model::{RunArtifact, RunStatus};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Event emitted when a durable run's observable state changes.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunUpdate {
    /// The run's status changed.
    #[serde(rename = "status_changed")]
    StatusChanged {
        /// The run that changed.
        run_id: RunId,
        /// The new status.
        status: RunStatus,
    },
    /// The run produced an artifact.
    #[serde(rename = "artifact_produced")]
    ArtifactProduced {
        /// The run that produced the artifact.
        run_id: RunId,
        /// The produced artifact.
        artifact: RunArtifact,
    },
}

/// Handle for receiving run updates. Drop to unsubscribe.
///
/// Wraps a `tokio::sync::broadcast::Receiver`. Lagged messages (when the
/// consumer is too slow) are silently skipped — the next available update
/// is returned instead.
pub struct RunSubscription {
    rx: tokio::sync::broadcast::Receiver<RunUpdate>,
}

impl RunSubscription {
    /// Create a subscription from a broadcast receiver.
    pub fn new(rx: tokio::sync::broadcast::Receiver<RunUpdate>) -> Self {
        Self { rx }
    }

    /// Wait for the next update.
    ///
    /// Returns `None` when the run reaches a terminal state and the
    /// sender is dropped, or when the broadcast channel is closed.
    pub async fn recv(&mut self) -> Option<RunUpdate> {
        loop {
            match self.rx.recv().await {
                Ok(update) => return Some(update),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl std::fmt::Debug for RunSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSubscription").finish_non_exhaustive()
    }
}

/// Observe durable run state changes via push subscription.
///
/// This is the streaming counterpart to [`crate::RunController`]'s poll-based
/// `get_run()`. Implementations emit [`RunUpdate`] events when a run's status
/// changes or when it produces artifacts.
#[async_trait]
pub trait RunObserver: Send + Sync {
    /// Subscribe to updates for a specific run.
    ///
    /// The returned [`RunSubscription`] delivers updates until the run
    /// reaches a terminal state or the subscription is dropped.
    async fn subscribe(&self, run_id: &RunId) -> Result<RunSubscription, RunControlError>;
}
