//! Lower-level driver seam for executing durable run work.

use crate::RunId;
use crate::command::DispatchPayload;
use crate::kernel::RunEvent;
use crate::store::BackendRunRef;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Driver work request for a durable run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverRequest {
    /// Run receiving the work.
    pub run_id: RunId,
    /// Portable reason the backend is dispatching the run.
    pub payload: DispatchPayload,
    /// Optional opaque backend continuation or workflow reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_ref: Option<BackendRunRef>,
}

impl DriverRequest {
    /// Create a new driver work request.
    pub fn new(
        run_id: RunId,
        payload: DispatchPayload,
        backend_ref: Option<BackendRunRef>,
    ) -> Self {
        Self {
            run_id,
            payload,
            backend_ref,
        }
    }
}

/// Semantic result returned by a driver after executing run work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverResponse {
    /// Next kernel event derived from the executed work.
    pub next_event: RunEvent,
    /// Current opaque backend continuation or workflow reference after the drive step.
    ///
    /// Drivers that mint or rotate backend-owned linkage return the latest value here so
    /// callers can persist it honestly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_ref: Option<BackendRunRef>,
}

impl DriverResponse {
    /// Create a new driver response carrying the next kernel event plus the latest backend reference.
    pub fn new(next_event: RunEvent, backend_ref: Option<BackendRunRef>) -> Self {
        Self {
            next_event,
            backend_ref,
        }
    }
}

/// Portable error surfaced by durable driver execution.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DriverError {
    /// The request conflicts with current durable execution state.
    #[error("run driver conflict: {0}")]
    Conflict(String),
    /// The request data is invalid for the backend.
    #[error("run driver invalid input: {0}")]
    InvalidInput(String),
    /// Backend-specific failure surfaced through the portable seam.
    #[error("run driver backend error: {0}")]
    Backend(String),
}

/// Executes runnable durable work and projects it back into kernel events.
#[async_trait]
pub trait RunDriver: Send + Sync {
    /// Execute a unit of durable run work and return the next semantic kernel event plus
    /// the latest backend-owned linkage for the run.
    async fn drive_run(&self, request: DriverRequest) -> Result<DriverResponse, DriverError>;
}
