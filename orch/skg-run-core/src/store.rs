//! Lower-level durable run and wait persistence seams for DIY backends.

use crate::id::{RunId, WaitPointId};
use crate::model::RunView;
use crate::wait::ResumeInput;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use thiserror::Error;

/// Backend-owned opaque reference associated with a durable run.
///
/// DIY backends may use this to point at checkpoint rows, continuation blobs, or
/// native workflow identifiers without forcing a shared replay schema.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackendRunRef(String);

impl BackendRunRef {
    /// Create a new opaque backend run reference.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the opaque backend-owned string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BackendRunRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BackendRunRef {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for BackendRunRef {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Current durable record persisted for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreRunRecord {
    /// Current portable run read model.
    pub view: RunView,
    /// Optional backend-owned continuation or linkage reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_ref: Option<BackendRunRef>,
}

impl StoreRunRecord {
    /// Create a durable run record from the portable read model plus an optional backend reference.
    pub fn new(view: RunView, backend_ref: Option<BackendRunRef>) -> Self {
        Self { view, backend_ref }
    }
}

/// Persisted durable resume input waiting to satisfy a specific wait point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingResume {
    /// Run receiving the durable resume.
    pub run_id: RunId,
    /// Wait point that must be satisfied.
    pub wait_point: WaitPointId,
    /// Structured portable resume input.
    pub input: ResumeInput,
}

impl PendingResume {
    /// Create a pending durable resume payload.
    pub fn new(run_id: RunId, wait_point: WaitPointId, input: ResumeInput) -> Self {
        Self {
            run_id,
            wait_point,
            input,
        }
    }
}

/// Persisted asynchronous signal for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingSignal {
    /// Run receiving the control-plane signal.
    pub run_id: RunId,
    /// Portable signal payload.
    pub signal: Value,
}

impl PendingSignal {
    /// Create a pending durable signal payload.
    pub fn new(run_id: RunId, signal: Value) -> Self {
        Self { run_id, signal }
    }
}

/// Portable error surfaced by durable run metadata persistence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunStoreError {
    /// A write targeted a run record that does not exist.
    #[error("run record not found: {0}")]
    RunNotFound(RunId),
    /// The write conflicts with existing durable state.
    #[error("run store conflict: {0}")]
    Conflict(String),
    /// Backend-specific failure surfaced through the portable seam.
    #[error("run store backend error: {0}")]
    Backend(String),
}

/// Portable error surfaced by durable wait persistence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WaitStoreError {
    /// The write conflicts with existing durable state.
    #[error("wait store conflict: {0}")]
    Conflict(String),
    /// Backend-specific failure surfaced through the portable seam.
    #[error("wait store backend error: {0}")]
    Backend(String),
}

/// Durable metadata store for current run state.
#[async_trait]
pub trait RunStore: Send + Sync {
    /// Insert a newly created durable run record.
    async fn insert_run(&self, run: StoreRunRecord) -> Result<(), RunStoreError>;

    /// Fetch the current durable record for a run.
    ///
    /// Returns `Ok(None)` when no durable record exists for the supplied run id.
    async fn get_run(&self, run_id: &RunId) -> Result<Option<StoreRunRecord>, RunStoreError>;

    /// Replace the current durable record for a run.
    ///
    /// Returns [`RunStoreError::RunNotFound`] when the durable record does not exist yet.
    async fn put_run(&self, run: StoreRunRecord) -> Result<(), RunStoreError>;
}

/// Durable store for wait-point resumes and asynchronous signals.
///
/// This keeps `resume` and `signal` distinct even when a backend persists both in
/// the same physical system.
#[async_trait]
pub trait WaitStore: Send + Sync {
    /// Persist a resume that satisfies a specific wait point.
    async fn save_resume(&self, resume: PendingResume) -> Result<(), WaitStoreError>;

    /// Take the next persisted resume for the given run and wait point.
    ///
    /// Returns `Ok(None)` when no resume is currently pending for that wait point.
    async fn take_resume(
        &self,
        run_id: &RunId,
        wait_point: &WaitPointId,
    ) -> Result<Option<PendingResume>, WaitStoreError>;

    /// Persist an asynchronous signal for a run.
    async fn push_signal(&self, signal: PendingSignal) -> Result<(), WaitStoreError>;

    /// Drain any queued signals for a run.
    async fn drain_signals(&self, run_id: &RunId) -> Result<Vec<PendingSignal>, WaitStoreError>;
}
