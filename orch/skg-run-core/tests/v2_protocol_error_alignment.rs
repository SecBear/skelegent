use layer0::{ErrorCode, ProtocolError};
use serde_json::json;
use skg_run_core::{
    OrchestrationCommand, ResumeAction, RunEvent, RunId, RunKernel, RunOutcome, RunView,
};

fn test_error() -> ProtocolError {
    ProtocolError::new(ErrorCode::Unavailable, "provider timeout", true)
}

// ── RunOutcome::Failed round-trips with ProtocolError ───────────────────────

#[test]
fn run_outcome_failed_round_trip() {
    let outcome = RunOutcome::Failed {
        error: test_error(),
    };
    let json = serde_json::to_value(&outcome).expect("serialize");
    assert_eq!(json["kind"], json!("failed"));
    assert_eq!(json["error"]["code"], json!("unavailable"));
    assert!(json["error"]["retryable"].as_bool().unwrap());

    let back: RunOutcome = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, outcome);
}

#[test]
fn run_outcome_failed_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/run-outcome-failed.json"))
            .expect("fixture json");
    let outcome: RunOutcome = serde_json::from_value(fixture.clone()).expect("deserialize");
    match &outcome {
        RunOutcome::Failed { error } => {
            assert_eq!(error.code, ErrorCode::Unavailable);
            assert!(error.retryable);
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let reencoded = serde_json::to_value(&outcome).expect("serialize");
    assert_eq!(reencoded, fixture);
}

// ── RunView::Failed round-trips with ProtocolError ──────────────────────────

#[test]
fn run_view_failed_round_trip() {
    let view = RunView::terminal(
        RunId::new("run-1"),
        RunOutcome::Failed {
            error: test_error(),
        },
    );
    let json = serde_json::to_value(&view).expect("serialize");
    assert_eq!(json["status"], json!("failed"));
    assert_eq!(json["error"]["code"], json!("unavailable"));

    let back: RunView = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, view);
}

// ── RunEvent::Fail round-trips with ProtocolError ───────────────────────────

#[test]
fn run_event_fail_round_trip() {
    let event = RunEvent::Fail {
        error: test_error(),
    };
    let json = serde_json::to_value(&event).expect("serialize");
    assert_eq!(json["kind"], json!("fail"));
    assert_eq!(json["error"]["code"], json!("unavailable"));

    let back: RunEvent = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, event);
}

// ── ResumeAction::Fail round-trips with ProtocolError ───────────────────────

#[test]
fn resume_action_fail_round_trip() {
    let action = ResumeAction::Fail {
        error: test_error(),
    };
    let json = serde_json::to_value(&action).expect("serialize");
    assert_eq!(json["kind"], json!("fail"));
    assert_eq!(json["error"]["code"], json!("unavailable"));

    let back: ResumeAction = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, action);
}

// ── OrchestrationCommand::FailRun round-trips with ProtocolError ────────────

#[test]
fn orchestration_command_fail_run_round_trip() {
    let cmd = OrchestrationCommand::FailRun {
        run_id: RunId::new("run-1"),
        error: test_error(),
    };
    let json = serde_json::to_value(&cmd).expect("serialize");
    assert_eq!(json["kind"], json!("fail_run"));
    assert_eq!(json["error"]["code"], json!("unavailable"));

    let back: OrchestrationCommand = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, cmd);
}

// ── Kernel fail transition produces ProtocolError in view ───────────────────

#[test]
fn kernel_fail_transition_carries_protocol_error() {
    let running = RunView::running(RunId::new("run-1"));
    let transition = RunKernel::apply(
        Some(&running),
        RunEvent::Fail {
            error: test_error(),
        },
    )
    .expect("valid transition");

    match &transition.next {
        RunView::Failed { error, .. } => {
            assert_eq!(error.code, ErrorCode::Unavailable);
            assert!(error.retryable);
        }
        other => panic!("expected Failed view, got {other:?}"),
    }

    assert!(transition.commands.iter().any(|c| matches!(
        c,
        OrchestrationCommand::FailRun { .. }
    )));
}
