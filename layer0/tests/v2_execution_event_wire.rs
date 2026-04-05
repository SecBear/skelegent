use layer0::OperatorOutput;
use layer0::content::Content;
use layer0::dispatch::Artifact;
use layer0::error::ProtocolError;
use layer0::event::{EventKind, ExecutionEvent};
use layer0::intent::{Intent, IntentKind};
use layer0::operator::{Outcome, TerminalOutcome};
use layer0::wait::WaitReason;
use serde_json::json;
use std::collections::HashMap;

/// Construct an ExecutionEvent via serde to work around #[non_exhaustive].
fn make_event(kind: EventKind) -> ExecutionEvent {
    // Build the event via JSON → deserialize, using a minimal EventMeta.
    let kind_json = serde_json::to_value(&kind).expect("serialize kind");
    let event_json = json!({
        "meta": {
            "event_id": "event-test",
            "dispatch_id": "dispatch-1",
            "seq": 1,
            "timestamp_unix_ms": 1700000000000_u64,
            "source": "runtime"
        },
        "kind": kind_json
    });
    serde_json::from_value(event_json).expect("deserialize event")
}

fn assert_event_kind_round_trip(kind: EventKind) {
    let event = make_event(kind);
    let encoded = serde_json::to_value(&event).expect("serialize");
    let decoded: ExecutionEvent = serde_json::from_value(encoded.clone()).expect("deserialize");
    let reencoded = serde_json::to_value(&decoded).expect("re-serialize");
    assert_eq!(encoded, reencoded, "round-trip mismatch");
}

// ── Each EventKind variant round-trips with locked wire shape ───────────────

#[test]
fn invocation_started_round_trip() {
    let kind = EventKind::InvocationStarted {
        operator: layer0::OperatorId::new("echo"),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("invocation_started"));
    assert_eq!(encoded["operator"], json!("echo"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn inference_started_round_trip() {
    let kind = EventKind::InferenceStarted {
        model: Some("claude-3-opus".into()),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("inference_started"));
    assert_eq!(encoded["model"], json!("claude-3-opus"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn tool_call_assembled_round_trip() {
    let kind = EventKind::ToolCallAssembled {
        call_id: "call-1".into(),
        capability_id: "search".into(),
        input: json!({"q": "hello"}),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("tool_call_assembled"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn tool_result_received_round_trip() {
    let kind = EventKind::ToolResultReceived {
        call_id: "call-1".into(),
        capability_id: "search".into(),
        output: json!({"results": []}),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("tool_result_received"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn intent_declared_round_trip() {
    let kind = EventKind::IntentDeclared {
        intent: Intent::new(IntentKind::DeleteMemory {
            scope: layer0::Scope::Global,
            key: "k".into(),
        }),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("intent_declared"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn progress_round_trip() {
    let kind = EventKind::Progress {
        content: Content::text("thinking..."),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("progress"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn artifact_produced_round_trip() {
    let kind = EventKind::ArtifactProduced {
        artifact: Artifact::new("a1", vec![Content::text("output")]),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("artifact_produced"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn log_round_trip() {
    let kind = EventKind::Log {
        level: "info".into(),
        message: "step completed".into(),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("log"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn observation_round_trip() {
    let kind = EventKind::Observation {
        key: "tokens".into(),
        value: json!(42),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("observation"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn metric_round_trip() {
    let kind = EventKind::Metric {
        name: "latency_ms".into(),
        value: 123.4,
        tags: {
            let mut m = HashMap::new();
            m.insert("provider".into(), "anthropic".into());
            m
        },
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("metric"));
    assert_event_kind_round_trip(kind);
}

#[test]
fn suspended_round_trip() {
    // Construct Suspended via serde since WaitState is #[non_exhaustive].
    let kind: EventKind = serde_json::from_value(json!({
        "kind": "suspended",
        "wait": { "reason": "approval" }
    }))
    .expect("deserialize");
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("suspended"));
    assert_eq!(encoded["wait"]["reason"], json!("approval"));

    // Round-trip
    let decoded: EventKind = serde_json::from_value(encoded.clone()).expect("deserialize");
    let reencoded = serde_json::to_value(&decoded).expect("re-serialize");
    assert_eq!(encoded, reencoded);
}

#[test]
fn completed_round_trip() {
    let kind = EventKind::Completed {
        output: OperatorOutput::new(
            Content::text("result"),
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            },
        ),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("completed"));
    assert!(encoded["output"]["outcome"].is_object());
    assert_event_kind_round_trip(kind);
}

#[test]
fn failed_round_trip() {
    let kind = EventKind::Failed {
        error: ProtocolError::internal("boom"),
    };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    assert_eq!(encoded["kind"], json!("failed"));
    assert_eq!(encoded["error"]["code"], json!("internal"));
    assert_event_kind_round_trip(kind);
}

// ── Golden fixture: suspended approval event ────────────────────────────────

#[test]
fn suspended_approval_golden_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "golden/v2/execution-event-suspended-approval.json"
    ))
    .expect("fixture json");
    let event: ExecutionEvent = serde_json::from_value(fixture.clone()).expect("deserialize");
    assert_eq!(event.meta.event_id, "event-10");
    match &event.kind {
        EventKind::Suspended { wait, .. } => {
            assert_eq!(wait.reason, WaitReason::Approval);
        }
        other => panic!("expected Suspended, got {other:?}"),
    }
    let reencoded = serde_json::to_value(&event).expect("serialize");
    assert_eq!(reencoded, fixture);
}

// ── InferenceStarted round-trips as a semantic event ────────────────────────

#[test]
fn inference_started_is_semantic_event() {
    let kind = EventKind::InferenceStarted { model: None };
    let encoded = serde_json::to_value(&kind).expect("serialize");
    // InferenceStarted round-trips as a semantic event
    let decoded: EventKind = serde_json::from_value(encoded).expect("deserialize");
    assert!(matches!(decoded, EventKind::InferenceStarted { .. }));
}
