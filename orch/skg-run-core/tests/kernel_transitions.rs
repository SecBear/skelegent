use serde_json::json;
use skg_run_core::{
    DispatchPayload, KernelError, OrchestrationCommand, PortableWakeDeadline, ResumeAction,
    ResumeInput, RunEvent, RunId, RunKernel, RunView, WaitPointId, WaitReason,
};

#[test]
fn start_moves_into_running_and_dispatches_operator() {
    let run_id = RunId::new("run-1");
    let input = json!({ "job": "demo" });

    let transition = RunKernel::apply(
        None,
        RunEvent::Start {
            run_id: run_id.clone(),
            input: input.clone(),
        },
    )
    .unwrap();

    assert_eq!(transition.next, RunView::running(run_id.clone()));
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::DispatchOperator {
            run_id,
            payload: DispatchPayload::Start { input },
        }]
    );
}

#[test]
fn running_enters_waitpoint() {
    let run_id = RunId::new("run-1");
    let wait_point = WaitPointId::new("wait-1");
    let reason = WaitReason::ExternalInput;

    let transition = RunKernel::apply(
        Some(&RunView::running(run_id.clone())),
        RunEvent::Wait {
            wait_point: wait_point.clone(),
            reason: reason.clone(),
            wake_at: None,
        },
    )
    .unwrap();

    assert_eq!(
        transition.next,
        RunView::waiting(run_id.clone(), wait_point.clone(), reason.clone())
    );
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::EnterWaitPoint {
            run_id,
            wait_point,
            reason,
        }]
    );
}

#[test]
fn waiting_resume_continue_returns_to_running_and_dispatches_operator() {
    let run_id = RunId::new("run-1");
    let wait_point = WaitPointId::new("wait-1");
    let resume_input = ResumeInput::new(json!({ "answer": 42 }));
    let current = RunView::waiting(
        run_id.clone(),
        wait_point.clone(),
        WaitReason::ExternalInput,
    );

    let transition = RunKernel::apply(
        Some(&current),
        RunEvent::Resume {
            wait_point: wait_point.clone(),
            input: resume_input.clone(),
            action: ResumeAction::Continue,
        },
    )
    .unwrap();

    assert_eq!(transition.next, RunView::running(run_id.clone()));
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::DispatchOperator {
            run_id,
            payload: DispatchPayload::Resume {
                wait_point,
                input: resume_input,
            },
        }]
    );
}

#[test]
fn waiting_resume_complete_finishes_run() {
    let run_id = RunId::new("run-1");
    let wait_point = WaitPointId::new("wait-1");
    let result = json!({ "approved": true });
    let current = RunView::waiting(run_id.clone(), wait_point.clone(), WaitReason::Approval);

    let transition = RunKernel::apply(
        Some(&current),
        RunEvent::Resume {
            wait_point,
            input: ResumeInput::new(json!({ "decision": "approve" })),
            action: ResumeAction::Complete {
                result: result.clone(),
            },
        },
    )
    .unwrap();

    assert_eq!(
        transition.next,
        RunView::terminal(
            run_id.clone(),
            skg_run_core::RunOutcome::Completed {
                result: result.clone(),
            },
        )
    );
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::CompleteRun { run_id, result }]
    );
}

#[test]
fn cancel_running_transitions_to_cancelled() {
    let run_id = RunId::new("run-1");

    let transition =
        RunKernel::apply(Some(&RunView::running(run_id.clone())), RunEvent::Cancel).unwrap();

    assert_eq!(
        transition.next,
        RunView::terminal(run_id.clone(), skg_run_core::RunOutcome::Cancelled)
    );
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::CancelRun { run_id }]
    );
}

#[test]
fn cancel_waiting_transitions_to_cancelled() {
    let run_id = RunId::new("run-1");

    let transition = RunKernel::apply(
        Some(&RunView::waiting(
            run_id.clone(),
            WaitPointId::new("wait-1"),
            WaitReason::ExternalInput,
        )),
        RunEvent::Cancel,
    )
    .unwrap();

    assert_eq!(
        transition.next,
        RunView::terminal(run_id.clone(), skg_run_core::RunOutcome::Cancelled)
    );
    assert_eq!(
        transition.commands,
        vec![OrchestrationCommand::CancelRun { run_id }]
    );
}

#[test]
fn invalid_resume_token_is_rejected() {
    let run_id = RunId::new("run-1");
    let current = RunView::waiting(
        run_id.clone(),
        WaitPointId::new("wait-expected"),
        WaitReason::ExternalInput,
    );

    let error = RunKernel::apply(
        Some(&current),
        RunEvent::Resume {
            wait_point: WaitPointId::new("wait-found"),
            input: ResumeInput::new(json!({ "answer": 42 })),
            action: ResumeAction::Continue,
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        KernelError::InvalidResumeToken {
            run_id: found_run_id,
            expected,
            found,
        } if found_run_id == run_id
            && expected == WaitPointId::new("wait-expected")
            && found == WaitPointId::new("wait-found")
    ));
}

#[test]
fn waiting_with_wake_deadline_schedules_canonical_deadline() {
    let run_id = RunId::new("run-1");
    let wait_point = WaitPointId::new("wait-1");
    let reason = WaitReason::Timer;
    let wake_at = PortableWakeDeadline::parse("2026-03-12T08:15:30Z").unwrap();

    let transition = RunKernel::apply(
        Some(&RunView::running(run_id.clone())),
        RunEvent::Wait {
            wait_point: wait_point.clone(),
            reason: reason.clone(),
            wake_at: Some(wake_at.clone()),
        },
    )
    .unwrap();

    assert_eq!(
        transition.commands,
        vec![
            OrchestrationCommand::EnterWaitPoint {
                run_id: run_id.clone(),
                wait_point: wait_point.clone(),
                reason,
            },
            OrchestrationCommand::ScheduleWake {
                run_id,
                wait_point,
                wake_at,
            },
        ]
    );
}

#[test]
fn start_from_existing_state_is_rejected() {
    let error = RunKernel::apply(
        Some(&RunView::running(RunId::new("run-1"))),
        RunEvent::Start {
            run_id: RunId::new("run-2"),
            input: json!({ "job": "demo" }),
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        KernelError::InvalidTransition {
            status: "running",
            event: "start",
        }
    );
}

#[test]
fn wait_from_waiting_is_rejected() {
    let error = RunKernel::apply(
        Some(&RunView::waiting(
            RunId::new("run-1"),
            WaitPointId::new("wait-1"),
            WaitReason::ExternalInput,
        )),
        RunEvent::Wait {
            wait_point: WaitPointId::new("wait-2"),
            reason: WaitReason::Timer,
            wake_at: None,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        KernelError::InvalidTransition {
            status: "waiting",
            event: "wait",
        }
    );
}

#[test]
fn terminal_states_reject_cancel_complete_and_fail() {
    let completed = RunView::terminal(
        RunId::new("run-1"),
        skg_run_core::RunOutcome::Completed {
            result: json!({ "ok": true }),
        },
    );

    assert_eq!(
        RunKernel::apply(Some(&completed), RunEvent::Cancel).unwrap_err(),
        KernelError::InvalidTransition {
            status: "completed",
            event: "cancel",
        }
    );
    assert_eq!(
        RunKernel::apply(
            Some(&completed),
            RunEvent::Complete {
                result: json!({ "ok": false }),
            },
        )
        .unwrap_err(),
        KernelError::InvalidTransition {
            status: "completed",
            event: "complete",
        }
    );
    assert_eq!(
        RunKernel::apply(
            Some(&completed),
            RunEvent::Fail {
                error: "boom".to_owned(),
            },
        )
        .unwrap_err(),
        KernelError::InvalidTransition {
            status: "completed",
            event: "fail",
        }
    );
}

#[test]
fn waiting_rejects_direct_complete_and_fail() {
    let waiting = RunView::waiting(
        RunId::new("run-1"),
        WaitPointId::new("wait-1"),
        WaitReason::Approval,
    );

    assert_eq!(
        RunKernel::apply(
            Some(&waiting),
            RunEvent::Complete {
                result: json!({ "approved": true }),
            },
        )
        .unwrap_err(),
        KernelError::InvalidTransition {
            status: "waiting",
            event: "complete",
        }
    );
    assert_eq!(
        RunKernel::apply(
            Some(&waiting),
            RunEvent::Fail {
                error: "denied".to_owned(),
            },
        )
        .unwrap_err(),
        KernelError::InvalidTransition {
            status: "waiting",
            event: "fail",
        }
    );
}
