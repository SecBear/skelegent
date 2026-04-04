use layer0::content::Content;
use layer0::error::ProtocolError;
use layer0::event::EventKind;
use layer0::operator::{Outcome, TerminalOutcome};
use layer0::OperatorOutput;
use serde_json::json;

// ── EventKind::Failed is for protocol failure only ──────────────────────────

#[test]
fn event_kind_failed_is_protocol_failure() {
    let kind = EventKind::Failed {
        error: ProtocolError::internal("network failure"),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");

    // Failed events carry a ProtocolError, not an OperatorOutput.
    assert_eq!(encoded["kind"], json!("failed"));
    assert!(encoded.get("error").is_some());
    assert!(encoded.get("output").is_none());
}

// ── Outcome::Terminal::Failed travels through EventKind::Completed ──────────

#[test]
fn terminal_failed_outcome_travels_through_completed_event() {
    // When an operator finishes with Outcome::Terminal { Failed },
    // the event is EventKind::Completed (not EventKind::Failed).
    // EventKind::Failed is reserved for protocol-level failures.
    let output = OperatorOutput::new(
        Content::text("error details for user"),
        Outcome::Terminal {
            terminal: TerminalOutcome::Failed,
        },
    );
    let kind = EventKind::Completed {
        output: output.clone(),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");

    // The event is "completed", not "failed".
    assert_eq!(encoded["kind"], json!("completed"));

    // But the inner output's outcome indicates terminal failure.
    assert_eq!(encoded["output"]["outcome"]["type"], json!("terminal"));
    assert_eq!(encoded["output"]["outcome"]["terminal"], json!("failed"));

    // Round-trip preserves the distinction.
    let decoded: EventKind = serde_json::from_value(encoded).expect("deserialize");
    match decoded {
        EventKind::Completed { output: decoded_output } => {
            assert_eq!(
                decoded_output.outcome,
                Outcome::Terminal {
                    terminal: TerminalOutcome::Failed
                }
            );
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

// ── Protocol failure vs operator failure are distinct event paths ────────────

#[test]
fn protocol_failure_and_operator_failure_are_distinct_event_paths() {
    // Protocol failure: EventKind::Failed { error }
    let protocol_failure = EventKind::Failed {
        error: ProtocolError::unavailable("provider down"),
    };

    // Operator failure: EventKind::Completed { output } where outcome = Terminal::Failed
    let operator_failure = EventKind::Completed {
        output: OperatorOutput::new(
            Content::text("unrecoverable"),
            Outcome::Terminal {
                terminal: TerminalOutcome::Failed,
            },
        ),
    };

    let pf_json = serde_json::to_value(&protocol_failure).expect("serialize");
    let of_json = serde_json::to_value(&operator_failure).expect("serialize");

    // They use different event kinds.
    assert_eq!(pf_json["kind"], json!("failed"));
    assert_eq!(of_json["kind"], json!("completed"));

    // Protocol failure has no output.
    assert!(pf_json.get("output").is_none());

    // Operator failure has no top-level error.
    assert!(of_json.get("error").is_none());
}
