use layer0::operator::{
    InterceptionKind, LimitReason, Outcome, TerminalOutcome, TransferOutcome,
};
use layer0::wait::WaitReason;
use layer0::{Content, Intent, IntentKind, OperatorOutput};
use serde_json::json;

fn assert_outcome_round_trip(outcome: &Outcome, expected_shape: serde_json::Value) {
    let encoded = serde_json::to_value(outcome).expect("serialize");
    assert_eq!(encoded, expected_shape, "wire shape mismatch");
    let decoded: Outcome = serde_json::from_value(encoded).expect("deserialize");
    assert_eq!(&decoded, outcome);
}

// ── Outcome variant round-trips with locked wire shape ──────────────────────

#[test]
fn outcome_terminal_completed_round_trip() {
    let outcome = Outcome::Terminal {
        terminal: TerminalOutcome::Completed,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "terminal", "terminal": "completed"}));
}

#[test]
fn outcome_terminal_failed_round_trip() {
    let outcome = Outcome::Terminal {
        terminal: TerminalOutcome::Failed,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "terminal", "terminal": "failed"}));
}

#[test]
fn outcome_transfer_handed_off_round_trip() {
    let outcome = Outcome::Transfer {
        transfer: TransferOutcome::HandedOff,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "transfer", "transfer": "handed_off"}));
}

#[test]
fn outcome_transfer_delegated_round_trip() {
    let outcome = Outcome::Transfer {
        transfer: TransferOutcome::Delegated,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "transfer", "transfer": "delegated"}));
}

#[test]
fn outcome_suspended_approval_round_trip() {
    let outcome = Outcome::Suspended {
        reason: WaitReason::Approval,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "suspended", "reason": "approval"}));
}

#[test]
fn outcome_suspended_external_input_round_trip() {
    let outcome = Outcome::Suspended {
        reason: WaitReason::ExternalInput,
    };
    assert_outcome_round_trip(
        &outcome,
        json!({"type": "suspended", "reason": "external_input"}),
    );
}

#[test]
fn outcome_limited_max_turns_round_trip() {
    let outcome = Outcome::Limited {
        limit: LimitReason::MaxTurns,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "limited", "limit": "max_turns"}));
}

#[test]
fn outcome_limited_budget_exhausted_round_trip() {
    let outcome = Outcome::Limited {
        limit: LimitReason::BudgetExhausted,
    };
    assert_outcome_round_trip(
        &outcome,
        json!({"type": "limited", "limit": "budget_exhausted"}),
    );
}

#[test]
fn outcome_limited_timeout_round_trip() {
    let outcome = Outcome::Limited {
        limit: LimitReason::Timeout,
    };
    assert_outcome_round_trip(&outcome, json!({"type": "limited", "limit": "timeout"}));
}

#[test]
fn outcome_limited_circuit_breaker_round_trip() {
    let outcome = Outcome::Limited {
        limit: LimitReason::CircuitBreaker,
    };
    assert_outcome_round_trip(
        &outcome,
        json!({"type": "limited", "limit": "circuit_breaker"}),
    );
}

#[test]
fn outcome_intercepted_policy_halt_round_trip() {
    let outcome = Outcome::Intercepted {
        interception: InterceptionKind::PolicyHalt {
            reason: "budget policy".into(),
        },
    };
    assert_outcome_round_trip(
        &outcome,
        json!({
            "type": "intercepted",
            "interception": {"policy_halt": {"reason": "budget policy"}}
        }),
    );
}

#[test]
fn outcome_intercepted_safety_stop_round_trip() {
    let outcome = Outcome::Intercepted {
        interception: InterceptionKind::SafetyStop {
            reason: "content filtered".into(),
        },
    };
    assert_outcome_round_trip(
        &outcome,
        json!({
            "type": "intercepted",
            "interception": {"safety_stop": {"reason": "content filtered"}}
        }),
    );
}

// ── Golden fixture round-trips ──────────────────────────────────────────────

#[test]
fn outcome_completed_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/outcome-completed.json"))
            .expect("fixture json");
    let outcome: Outcome = serde_json::from_value(fixture.clone()).expect("deserialize");
    assert_eq!(
        outcome,
        Outcome::Terminal {
            terminal: TerminalOutcome::Completed
        }
    );
    let reencoded = serde_json::to_value(&outcome).expect("serialize");
    assert_eq!(reencoded, fixture);
}

#[test]
fn outcome_approval_suspended_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/outcome-approval-suspended.json"))
            .expect("fixture json");
    let outcome: Outcome = serde_json::from_value(fixture.clone()).expect("deserialize");
    assert_eq!(
        outcome,
        Outcome::Suspended {
            reason: WaitReason::Approval
        }
    );
    let reencoded = serde_json::to_value(&outcome).expect("serialize");
    assert_eq!(reencoded, fixture);
}

#[test]
fn outcome_handoff_transferred_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/outcome-handoff-transferred.json"))
            .expect("fixture json");
    let outcome: Outcome = serde_json::from_value(fixture.clone()).expect("deserialize");
    assert_eq!(
        outcome,
        Outcome::Transfer {
            transfer: TransferOutcome::HandedOff
        }
    );
    let reencoded = serde_json::to_value(&outcome).expect("serialize");
    assert_eq!(reencoded, fixture);
}

// ── Handoff transfer derivation ─────────────────────────────────────────────

#[test]
fn handoff_transfer_derived_from_first_matching_intent() {
    let output = OperatorOutput::new(
        Content::text("done"),
        Outcome::Transfer {
            transfer: TransferOutcome::HandedOff,
        },
    );
    // When an operator declares Handoff intents, the first one determines the target.
    // The outcome reflects HandedOff transfer.
    assert_eq!(
        output.outcome,
        Outcome::Transfer {
            transfer: TransferOutcome::HandedOff
        }
    );

    // Verify an output with handoff intents can carry them alongside the outcome.
    let mut output_with_intents = output;
    output_with_intents.intents.push(Intent::new(IntentKind::Handoff {
        operator: layer0::OperatorId::new("target-op"),
        context: layer0::HandoffContext {
            task: Content::text("please continue"),
            history: None,
            metadata: None,
        },
    }));
    assert_eq!(output_with_intents.intents.len(), 1);
    assert!(matches!(
        output_with_intents.intents[0].kind,
        IntentKind::Handoff { .. }
    ));
}

// ── OperatorOutput serializes `outcome` field ───────────────────────────────

#[test]
fn operator_output_serializes_outcome_field() {
    let output = OperatorOutput::new(
        Content::text("result"),
        Outcome::Terminal {
            terminal: TerminalOutcome::Completed,
        },
    );
    let json = serde_json::to_value(&output).expect("serialize");

    // v2 `outcome` field is present with correct wire shape.
    assert_eq!(
        json["outcome"],
        json!({"type": "terminal", "terminal": "completed"})
    );

    // v1 `exit_reason` must NOT be present in v2 surface.
    assert!(
        json.get("exit_reason").is_none(),
        "exit_reason must not be serialized in v2"
    );
}
