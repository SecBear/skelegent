//! Backend-agnostic orchestration commands emitted by the durable kernel.

use crate::deadline::PortableWakeDeadline;
use crate::id::{RunId, WaitPointId};
use crate::wait::{ResumeInput, WaitReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// High-level cause for dispatching operator work.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DispatchPayload {
    /// Start a run with its initial portable input payload.
    Start {
        /// Portable start payload.
        input: Value,
    },
    /// Resume a waiting run at a specific wait point.
    Resume {
        /// Wait point being satisfied.
        wait_point: WaitPointId,
        /// Structured portable resume input.
        input: ResumeInput,
    },
}

/// Backend-neutral orchestration intent emitted by the durable kernel.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrchestrationCommand {
    /// Dispatch operator work for the run.
    DispatchOperator {
        /// Durable run receiving work.
        run_id: RunId,
        /// Why the dispatch is occurring.
        payload: DispatchPayload,
    },
    /// Persist that a run entered a durable wait point.
    EnterWaitPoint {
        /// Durable run entering a wait point.
        run_id: RunId,
        /// Wait point now active.
        wait_point: WaitPointId,
        /// Portable reason the run is waiting.
        reason: WaitReason,
    },
    /// Schedule a durable wake-up for a wait point.
    ScheduleWake {
        /// Durable run that should wake later.
        run_id: RunId,
        /// Wait point associated with the wake.
        wait_point: WaitPointId,
        /// Backend-neutral wake deadline encoded as canonical RFC 3339 UTC.
        wake_at: PortableWakeDeadline,
    },
    /// Commit successful terminal completion for the run.
    CompleteRun {
        /// Durable run that completed.
        run_id: RunId,
        /// Portable terminal result payload.
        result: Value,
    },
    /// Commit terminal failure for the run.
    FailRun {
        /// Durable run that failed.
        run_id: RunId,
        /// Human-readable portable failure summary.
        error: String,
    },
    /// Commit terminal cancellation for the run.
    CancelRun {
        /// Durable run that was cancelled.
        run_id: RunId,
    },
}
