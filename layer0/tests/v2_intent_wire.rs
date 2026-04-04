use layer0::operator::TriggerType;
use layer0::{
    Content, HandoffContext, Intent, IntentKind, MemoryLink, MemoryScope, OperatorId,
    OperatorInput, Scope, SignalPayload, WorkflowId,
};
use serde_json::json;

fn assert_round_trip(intent: Intent) {
    let encoded = serde_json::to_value(&intent).expect("serialize");
    let decoded: Intent = serde_json::from_value(encoded.clone()).expect("deserialize");
    let reencoded = serde_json::to_value(decoded).expect("re-serialize");
    assert_eq!(encoded, reencoded);
}

#[test]
fn intent_golden_fixtures_round_trip() {
    let write_fixture = include_str!("golden/v2/intent-write-memory.json");
    let write_intent: Intent = serde_json::from_str(write_fixture).expect("deserialize");
    let write_encoded = serde_json::to_value(&write_intent).expect("serialize");
    assert_eq!(write_encoded["kind"]["kind"], json!("write_memory"));

    let approval_fixture = include_str!("golden/v2/intent-request-approval.json");
    let approval_intent: Intent = serde_json::from_str(approval_fixture).expect("deserialize");
    let approval_encoded = serde_json::to_value(&approval_intent).expect("serialize");
    assert_eq!(approval_encoded["kind"]["kind"], json!("request_approval"));
}

#[test]
fn all_intent_kinds_round_trip() {
    let delegate_input = OperatorInput::new(Content::text("{\"task\":\"do\"}"), TriggerType::Task);
    let intents = vec![
        Intent::new(IntentKind::WriteMemory {
            scope: Scope::Session("session-1".into()),
            key: "k".into(),
            value: json!({"v": 1}),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        }),
        Intent::new(IntentKind::DeleteMemory {
            scope: Scope::Global,
            key: "k".into(),
        }),
        Intent::new(IntentKind::LinkMemory {
            scope: Scope::Global,
            link: MemoryLink::new("a", "b", "related_to"),
        }),
        Intent::new(IntentKind::UnlinkMemory {
            scope: Scope::Global,
            from_key: "a".into(),
            to_key: "b".into(),
            relation: "related_to".into(),
        }),
        Intent::new(IntentKind::Signal {
            target: WorkflowId::new("wf-1"),
            payload: SignalPayload::new("wake", json!({"x": 1})),
        }),
        Intent::new(IntentKind::Delegate {
            operator: OperatorId::new("op.delegate"),
            input: Box::new(delegate_input),
        }),
        Intent::new(IntentKind::Handoff {
            operator: OperatorId::new("op.next"),
            context: HandoffContext {
                task: Content::text("continue"),
                history: None,
                metadata: None,
            },
        }),
        Intent::new(IntentKind::RequestApproval {
            tool_name: "dangerous_tool".into(),
            call_id: "call-1".into(),
            input: json!({"cmd": "rm -rf /tmp/test"}),
        }),
        Intent::new(IntentKind::Custom {
            name: "domain.exec".into(),
            payload: json!({"a": 1}),
        }),
    ];

    for intent in intents {
        assert_round_trip(intent);
    }
}

#[test]
fn observational_payload_is_not_a_valid_intent_kind() {
    let parsed: Result<IntentKind, _> =
        serde_json::from_value(json!({"kind": "progress", "content": "hello"}));
    assert!(
        parsed.is_err(),
        "progress should not deserialize as IntentKind"
    );
}
