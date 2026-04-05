//! Integration tests for the HITL approval typed contract.
//!
//! These tests prove:
//!   1. ApprovalRequest/Response round-trip correctly through serde.
//!   2. An orchestrator can construct an ApprovalRequest from Effect::ToolApprovalRequired entries.
//!   3. The typed ApprovalResponse fits through the untyped ResumeInput.metadata path.

use layer0::Content;
use layer0::approval::{
    ApprovalReason, ApprovalRequest, ApprovalResponse, PendingToolCall, ToolCallAction,
    ToolCallDecision,
};
use layer0::operator::{OperatorInput, TriggerType};
use layer0::{Intent, IntentKind};
use serde_json::{Value, json};
use skg_run_core::ResumeInput;

// ---------------------------------------------------------------------------
// Test 1: ApprovalRequest round-trips through serde
// ---------------------------------------------------------------------------

#[test]
fn approval_request_serde_round_trip() {
    let request = ApprovalRequest::new(
        "run-abc123",
        "wp-approval-1",
        vec![
            PendingToolCall {
                call_id: "call_001".to_owned(),
                tool_name: "delete_file".to_owned(),
                tool_input: json!({ "path": "/etc/hosts" }),
                reason: ApprovalReason::PolicyAlways,
            },
            PendingToolCall {
                call_id: "call_002".to_owned(),
                tool_name: "exec_shell".to_owned(),
                tool_input: json!({ "command": "rm -rf /tmp/scratch" }),
                reason: ApprovalReason::PolicyPredicate {
                    description: "command contains destructive flags".to_owned(),
                },
            },
            PendingToolCall {
                call_id: "call_003".to_owned(),
                tool_name: "send_email".to_owned(),
                tool_input: json!({ "to": "ceo@example.com", "subject": "Quarterly Report" }),
                reason: ApprovalReason::MiddlewareBlocked {
                    guard: "email-blast-guard".to_owned(),
                },
            },
        ],
    )
    .with_prompt("Please review these tool calls before proceeding.")
    .with_context(Content::text(
        "Agent has drafted a report and wants to send it.",
    ));

    let serialized = serde_json::to_string(&request).expect("serialization must succeed");
    let deserialized: ApprovalRequest =
        serde_json::from_str(&serialized).expect("deserialization must succeed");

    assert_eq!(deserialized.run_id, request.run_id);
    assert_eq!(deserialized.wait_point, request.wait_point);
    assert_eq!(deserialized.prompt, request.prompt);
    assert_eq!(deserialized.pending.len(), 3);

    // Verify each pending call survives the round-trip
    assert_eq!(deserialized.pending[0].call_id, "call_001");
    assert_eq!(deserialized.pending[0].tool_name, "delete_file");
    assert!(matches!(
        deserialized.pending[0].reason,
        ApprovalReason::PolicyAlways
    ));

    assert_eq!(deserialized.pending[1].call_id, "call_002");
    assert!(matches!(
        &deserialized.pending[1].reason,
        ApprovalReason::PolicyPredicate { description } if description == "command contains destructive flags"
    ));

    assert_eq!(deserialized.pending[2].call_id, "call_003");
    assert!(matches!(
        &deserialized.pending[2].reason,
        ApprovalReason::MiddlewareBlocked { guard } if guard == "email-blast-guard"
    ));

    // context field also survives
    assert!(deserialized.context.is_some());
}

// ---------------------------------------------------------------------------
// Test 2: All ApprovalResponse variants round-trip through serde
// ---------------------------------------------------------------------------

#[test]
fn approval_response_variants_serde() {
    // ApproveAll
    let approve_all = ApprovalResponse::ApproveAll;
    round_trip_approval_response(&approve_all);
    assert!(matches!(
        serde_json::from_str::<ApprovalResponse>(&serde_json::to_string(&approve_all).unwrap())
            .unwrap(),
        ApprovalResponse::ApproveAll
    ));

    // RejectAll
    let reject_all = ApprovalResponse::RejectAll {
        reason: "Not authorized at this time.".to_owned(),
    };
    round_trip_approval_response(&reject_all);
    let rt = deserialize_rt(&reject_all);
    assert!(
        matches!(&rt, ApprovalResponse::RejectAll { reason } if reason == "Not authorized at this time.")
    );

    // Approve with specific IDs
    let approve_some = ApprovalResponse::Approve {
        call_ids: vec!["call_001".to_owned(), "call_003".to_owned()],
    };
    round_trip_approval_response(&approve_some);
    let rt = deserialize_rt(&approve_some);
    assert!(
        matches!(&rt, ApprovalResponse::Approve { call_ids } if call_ids == &["call_001", "call_003"])
    );

    // Reject specific IDs
    let reject_some = ApprovalResponse::Reject {
        call_ids: vec!["call_002".to_owned()],
        reason: "Dangerous command.".to_owned(),
    };
    round_trip_approval_response(&reject_some);
    let rt = deserialize_rt(&reject_some);
    assert!(matches!(&rt, ApprovalResponse::Reject { call_ids, reason }
            if call_ids == &["call_002"] && reason == "Dangerous command."));

    // Modify
    let modify = ApprovalResponse::Modify {
        call_id: "call_002".to_owned(),
        new_input: json!({ "command": "ls /tmp/scratch" }),
    };
    round_trip_approval_response(&modify);
    let rt = deserialize_rt(&modify);
    assert!(
        matches!(&rt, ApprovalResponse::Modify { call_id, new_input }
            if call_id == "call_002" && new_input == &json!({ "command": "ls /tmp/scratch" }))
    );

    // Batch with mixed decisions
    let batch = ApprovalResponse::Batch {
        decisions: vec![
            ToolCallDecision {
                call_id: "call_001".to_owned(),
                action: ToolCallAction::Approve,
            },
            ToolCallDecision {
                call_id: "call_002".to_owned(),
                action: ToolCallAction::Reject {
                    reason: "Too risky.".to_owned(),
                },
            },
            ToolCallDecision {
                call_id: "call_003".to_owned(),
                action: ToolCallAction::Modify {
                    new_input: json!({ "to": "internal@example.com", "subject": "Quarterly Report" }),
                },
            },
        ],
    };
    round_trip_approval_response(&batch);
    let rt = deserialize_rt(&batch);
    let ApprovalResponse::Batch { decisions } = rt else {
        panic!("expected Batch variant");
    };
    assert_eq!(decisions.len(), 3);
    assert_eq!(decisions[0].call_id, "call_001");
    assert!(matches!(decisions[0].action, ToolCallAction::Approve));
    assert_eq!(decisions[1].call_id, "call_002");
    assert!(
        matches!(&decisions[1].action, ToolCallAction::Reject { reason } if reason == "Too risky.")
    );
    assert_eq!(decisions[2].call_id, "call_003");
    assert!(matches!(
        &decisions[2].action,
        ToolCallAction::Modify { .. }
    ));
}

/// Serialize and deserialize; panic on either failure.
fn round_trip_approval_response(response: &ApprovalResponse) {
    let json = serde_json::to_string(response).expect("ApprovalResponse must serialize");
    let _: ApprovalResponse =
        serde_json::from_str(&json).expect("ApprovalResponse must deserialize");
}

fn deserialize_rt(response: &ApprovalResponse) -> ApprovalResponse {
    let json = serde_json::to_string(response).unwrap();
    serde_json::from_str(&json).unwrap()
}

// ---------------------------------------------------------------------------
// Test 3: Orchestrator constructs ApprovalRequest from IntentKind::RequestApproval
// ---------------------------------------------------------------------------

#[test]
fn approval_request_from_effects() {
    // Simulate what an operator would push into OperatorOutput::intents
    let intents = [
        Intent::new(IntentKind::RequestApproval {
            tool_name: "delete_file".to_owned(),
            call_id: "call_001".to_owned(),
            input: json!({ "path": "/etc/passwd" }),
        }),
        Intent::new(IntentKind::RequestApproval {
            tool_name: "send_email".to_owned(),
            call_id: "call_002".to_owned(),
            input: json!({ "to": "external@example.com" }),
        }),
        // Non-approval intent — must not appear in pending
        Intent::new(IntentKind::Custom {
            name: "progress".to_owned(),
            payload: json!({ "content": "Thinking..." }),
        }),
    ];

    // Orchestrator construction pattern: drain RequestApproval entries
    let run_id = "run-xyz789".to_owned();
    let wait_point = "wp-approval-2".to_owned();

    let pending: Vec<PendingToolCall> = intents
        .iter()
        .filter_map(|e| {
            if let IntentKind::RequestApproval {
                tool_name,
                call_id,
                input,
            } = &e.kind
            {
                Some(PendingToolCall {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    tool_input: input.clone(),
                    // Default reason for operator-declared approval intents is PolicyAlways
                    reason: ApprovalReason::PolicyAlways,
                })
            } else {
                None
            }
        })
        .collect();

    let request = ApprovalRequest::new(run_id.clone(), wait_point.clone(), pending);

    // Only the two ToolApprovalRequired effects become pending calls
    assert_eq!(request.pending.len(), 2);
    assert_eq!(request.run_id, run_id);
    assert_eq!(request.wait_point, wait_point);

    assert_eq!(request.pending[0].call_id, "call_001");
    assert_eq!(request.pending[0].tool_name, "delete_file");
    assert_eq!(
        request.pending[0].tool_input,
        json!({ "path": "/etc/passwd" })
    );

    assert_eq!(request.pending[1].call_id, "call_002");
    assert_eq!(request.pending[1].tool_name, "send_email");

    // The request also serializes cleanly (ready to store or emit as DispatchEvent)
    let serialized = serde_json::to_string(&request).unwrap();
    let deserialized: ApprovalRequest = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.pending.len(), 2);
}

// ---------------------------------------------------------------------------
// Test 4: ApprovalResponse travels through the untyped ResumeInput path
// ---------------------------------------------------------------------------

#[test]
fn approval_response_as_resume_input() {
    // The well-known metadata key the orchestrator writes and the operator reads
    const APPROVAL_RESPONSE_KEY: &str = "approval_response";

    let response = ApprovalResponse::Batch {
        decisions: vec![
            ToolCallDecision {
                call_id: "call_001".to_owned(),
                action: ToolCallAction::Approve,
            },
            ToolCallDecision {
                call_id: "call_002".to_owned(),
                action: ToolCallAction::Reject {
                    reason: "Rejected by operator.".to_owned(),
                },
            },
        ],
    };

    // Step 1: caller serializes ApprovalResponse into ResumeInput.metadata
    let response_value: Value =
        serde_json::to_value(&response).expect("ApprovalResponse must convert to Value");

    let resume = ResumeInput::new(json!("approval"))
        .with_metadata(APPROVAL_RESPONSE_KEY, response_value.clone());

    assert!(resume.metadata.contains_key(APPROVAL_RESPONSE_KEY));

    // Step 2: prove the OperatorInput layer preserves the metadata field
    let operator_input = OperatorInput::new(Content::text("approval"), TriggerType::Signal)
        .with_metadata(serde_json::to_value(&resume).expect("ResumeInput must serialize"));

    // Verify the outer OperatorInput serde round-trip preserves the nested value
    let input_json = serde_json::to_string(&operator_input).unwrap();
    let roundtripped: OperatorInput = serde_json::from_str(&input_json).unwrap();

    // Step 3: recipient extracts and deserializes the ApprovalResponse from metadata
    let inner_resume: ResumeInput = serde_json::from_value(roundtripped.metadata)
        .expect("metadata must deserialize as ResumeInput");

    let raw_response = inner_resume
        .metadata
        .get(APPROVAL_RESPONSE_KEY)
        .cloned()
        .expect("approval_response key must be present");

    let decoded: ApprovalResponse =
        serde_json::from_value(raw_response).expect("must deserialize back to ApprovalResponse");

    // Verify the decoded response matches the original
    let ApprovalResponse::Batch { decisions } = decoded else {
        panic!("expected Batch variant after round-trip through ResumeInput");
    };
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].call_id, "call_001");
    assert!(matches!(decisions[0].action, ToolCallAction::Approve));
    assert_eq!(decisions[1].call_id, "call_002");
    assert!(
        matches!(&decisions[1].action, ToolCallAction::Reject { reason } if reason == "Rejected by operator.")
    );
}
