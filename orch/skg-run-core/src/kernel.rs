//! Pure durable orchestration transition kernel.

use crate::command::{DispatchPayload, OrchestrationCommand};
use crate::deadline::PortableWakeDeadline;
use crate::id::{RunId, WaitPointId};
use crate::model::{RunOutcome, RunStatus, RunView};
use crate::wait::{ResumeInput, WaitReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Pure input event applied to durable run state.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    /// Start a new run from the not-yet-created state.
    Start {
        /// Durable run identifier to create.
        run_id: RunId,
        /// Portable start payload.
        input: Value,
    },
    /// Enter a durable wait point while running.
    Wait {
        /// Wait point that becomes active.
        wait_point: WaitPointId,
        /// Portable reason the run is blocked.
        reason: WaitReason,
        /// Optional backend-neutral wake deadline encoded as canonical RFC 3339 UTC.
        wake_at: Option<PortableWakeDeadline>,
    },
    /// Resume an active wait point.
    Resume {
        /// Wait point the caller intends to satisfy.
        wait_point: WaitPointId,
        /// Structured portable resume input.
        input: ResumeInput,
        /// The semantic effect of the resume on the run.
        action: ResumeAction,
    },
    /// Finish a run successfully.
    Complete {
        /// Portable terminal result payload.
        result: Value,
    },
    /// Finish a run with a terminal failure.
    Fail {
        /// Human-readable portable failure summary.
        error: String,
    },
    /// Cancel a currently active run.
    Cancel,
}

/// Semantic result of satisfying a durable wait point.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeAction {
    /// Resume execution and dispatch more operator work.
    Continue,
    /// Resume into successful terminal completion.
    Complete {
        /// Portable terminal result payload.
        result: Value,
    },
    /// Resume into terminal failure.
    Fail {
        /// Human-readable portable failure summary.
        error: String,
    },
}

/// Result of applying a durable kernel transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTransition {
    /// Next portable run view.
    pub next: RunView,
    /// High-level orchestration intents to execute outside the kernel.
    pub commands: Vec<OrchestrationCommand>,
}

/// Error returned when a transition is invalid for the current run state.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KernelError {
    /// The event is incompatible with the current durable lifecycle state.
    #[error("invalid durable transition from {status} via {event}")]
    InvalidTransition {
        /// Current lifecycle state, or `not_started` before creation.
        status: &'static str,
        /// Event kind that was rejected.
        event: &'static str,
    },
    /// The caller attempted to resume a different wait point than the one currently active.
    #[error(
        "invalid resume token for run {run_id}: expected {expected}, found {found}"
    )]
    InvalidResumeToken {
        /// Run receiving the invalid resume attempt.
        run_id: RunId,
        /// Currently active wait point.
        expected: WaitPointId,
        /// Wait point supplied by the caller.
        found: WaitPointId,
    },
}

/// Stateless pure transition kernel for durable orchestration.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunKernel;

impl RunKernel {
    /// Apply an event to the current durable run view and emit high-level commands.
    pub fn apply(current: Option<&RunView>, event: RunEvent) -> Result<RunTransition, KernelError> {
        match event {
            RunEvent::Start { run_id, input } => Self::start(current, run_id, input),
            RunEvent::Wait {
                wait_point,
                reason,
                wake_at,
            } => Self::wait(current, wait_point, reason, wake_at),
            RunEvent::Resume {
                wait_point,
                input,
                action,
            } => Self::resume(current, wait_point, input, action),
            RunEvent::Complete { result } => Self::complete(current, result),
            RunEvent::Fail { error } => Self::fail(current, error),
            RunEvent::Cancel => Self::cancel(current),
        }
    }

    fn start(
        current: Option<&RunView>,
        run_id: RunId,
        input: Value,
    ) -> Result<RunTransition, KernelError> {
        if current.is_some() {
            return Err(Self::invalid_transition(current, "start"));
        }

        Ok(RunTransition {
            next: RunView::running(run_id.clone()),
            commands: vec![OrchestrationCommand::DispatchOperator {
                run_id,
                payload: DispatchPayload::Start { input },
            }],
        })
    }

    fn wait(
        current: Option<&RunView>,
        wait_point: WaitPointId,
        reason: WaitReason,
        wake_at: Option<PortableWakeDeadline>,
    ) -> Result<RunTransition, KernelError> {
        let run_id = Self::running_run_id(current, "wait")?;
        let mut commands = vec![OrchestrationCommand::EnterWaitPoint {
            run_id: run_id.clone(),
            wait_point: wait_point.clone(),
            reason: reason.clone(),
        }];

        if let Some(wake_at) = wake_at {
            commands.push(OrchestrationCommand::ScheduleWake {
                run_id: run_id.clone(),
                wait_point: wait_point.clone(),
                wake_at,
            });
        }

        Ok(RunTransition {
            next: RunView::waiting(run_id, wait_point, reason),
            commands,
        })
    }

    fn resume(
        current: Option<&RunView>,
        wait_point: WaitPointId,
        input: ResumeInput,
        action: ResumeAction,
    ) -> Result<RunTransition, KernelError> {
        let (run_id, active_wait_point) = Self::waiting_context(current)?;
        if active_wait_point != &wait_point {
            return Err(KernelError::InvalidResumeToken {
                run_id,
                expected: active_wait_point.clone(),
                found: wait_point,
            });
        }

        match action {
            ResumeAction::Continue => Ok(RunTransition {
                next: RunView::running(run_id.clone()),
                commands: vec![OrchestrationCommand::DispatchOperator {
                    run_id,
                    payload: DispatchPayload::Resume { wait_point, input },
                }],
            }),
            ResumeAction::Complete { result } => Ok(RunTransition {
                next: RunView::terminal(
                    run_id.clone(),
                    RunOutcome::Completed {
                        result: result.clone(),
                    },
                ),
                commands: vec![OrchestrationCommand::CompleteRun { run_id, result }],
            }),
            ResumeAction::Fail { error } => Ok(RunTransition {
                next: RunView::terminal(
                    run_id.clone(),
                    RunOutcome::Failed {
                        error: error.clone(),
                    },
                ),
                commands: vec![OrchestrationCommand::FailRun { run_id, error }],
            }),
        }
    }

    fn complete(current: Option<&RunView>, result: Value) -> Result<RunTransition, KernelError> {
        let run_id = Self::running_run_id(current, "complete")?;
        Ok(RunTransition {
            next: RunView::terminal(
                run_id.clone(),
                RunOutcome::Completed {
                    result: result.clone(),
                },
            ),
            commands: vec![OrchestrationCommand::CompleteRun { run_id, result }],
        })
    }

    fn fail(current: Option<&RunView>, error: String) -> Result<RunTransition, KernelError> {
        let run_id = Self::running_run_id(current, "fail")?;
        Ok(RunTransition {
            next: RunView::terminal(
                run_id.clone(),
                RunOutcome::Failed {
                    error: error.clone(),
                },
            ),
            commands: vec![OrchestrationCommand::FailRun { run_id, error }],
        })
    }

    fn cancel(current: Option<&RunView>) -> Result<RunTransition, KernelError> {
        let run_id = Self::cancellable_run_id(current)?;
        Ok(RunTransition {
            next: RunView::terminal(run_id.clone(), RunOutcome::Cancelled),
            commands: vec![OrchestrationCommand::CancelRun { run_id }],
        })
    }

    fn running_run_id(current: Option<&RunView>, event: &'static str) -> Result<RunId, KernelError> {
        match current {
            Some(RunView::Running { run_id }) => Ok(run_id.clone()),
            _ => Err(Self::invalid_transition(current, event)),
        }
    }

    fn waiting_context(current: Option<&RunView>) -> Result<(RunId, &WaitPointId), KernelError> {
        match current {
            Some(RunView::Waiting {
                run_id,
                wait_point,
                ..
            }) => Ok((run_id.clone(), wait_point)),
            _ => Err(Self::invalid_transition(current, "resume")),
        }
    }

    fn cancellable_run_id(current: Option<&RunView>) -> Result<RunId, KernelError> {
        match current {
            Some(RunView::Running { run_id }) | Some(RunView::Waiting { run_id, .. }) => {
                Ok(run_id.clone())
            }
            _ => Err(Self::invalid_transition(current, "cancel")),
        }
    }

    fn invalid_transition(current: Option<&RunView>, event: &'static str) -> KernelError {
        KernelError::InvalidTransition {
            status: current.map_or("not_started", Self::status_name),
            event,
        }
    }

    fn status_name(view: &RunView) -> &'static str {
        match view.status() {
            RunStatus::Running => "running",
            RunStatus::Waiting => "waiting",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }
}
