//! Phase 1 acceptance tests for the Layer 0 trait crate.
//!
//! Tests cover:
//! - Message type serialization round-trips
//! - Trait object safety (Box<dyn Trait> is Send + Sync)
//! - Blanket StateReader impl
//! - Typed ID conversions
//! - Content helper methods
//! - Custom variant round-trips

use layer0::*;
use rust_decimal::Decimal;
use serde_json::json;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Object Safety: Box<dyn Trait> compiles and is Send + Sync
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn _assert_send_sync<T: Send + Sync>() {}

#[test]
fn operator_is_object_safe_send_sync() {
    _assert_send_sync::<Box<dyn Operator>>();
}

#[test]
fn arc_operator_is_send_sync() {
    _assert_send_sync::<std::sync::Arc<dyn Operator>>();
}

#[test]
fn arc_state_store_is_send_sync() {
    _assert_send_sync::<std::sync::Arc<dyn StateStore>>();
}

#[test]
fn arc_state_reader_is_send_sync() {
    _assert_send_sync::<std::sync::Arc<dyn StateReader>>();
}

#[test]
fn arc_environment_is_send_sync() {
    _assert_send_sync::<std::sync::Arc<dyn Environment>>();
}

#[test]
fn state_store_is_object_safe_send_sync() {
    _assert_send_sync::<Box<dyn StateStore>>();
}

#[test]
fn state_reader_is_object_safe_send_sync() {
    _assert_send_sync::<Box<dyn StateReader>>();
}

#[test]
fn environment_is_object_safe_send_sync() {
    _assert_send_sync::<Box<dyn Environment>>();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Typed ID conversions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn agent_id_from_str() {
    let id = OperatorId::from("agent-1");
    assert_eq!(id.as_str(), "agent-1");
    assert_eq!(id.to_string(), "agent-1");
}

#[test]
fn session_id_from_string() {
    let id = SessionId::from(String::from("sess-abc"));
    assert_eq!(id.as_str(), "sess-abc");
}

#[test]
fn workflow_id_new() {
    let id = WorkflowId::new("wf-123");
    assert_eq!(id.0, "wf-123");
}

#[test]
fn typed_id_serde_round_trip() {
    let id = OperatorId::new("test-agent");
    let json = serde_json::to_string(&id).unwrap();
    let back: OperatorId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Content helpers and round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn content_text_helper() {
    let c = Content::text("hello");
    assert_eq!(c.as_text(), Some("hello"));
}

#[test]
fn content_blocks_as_text_returns_first_text() {
    let c = Content::Blocks(vec![
        ContentBlock::Text {
            text: "first".into(),
        },
        ContentBlock::Text {
            text: "second".into(),
        },
    ]);
    assert_eq!(c.as_text(), Some("first"));
}

#[test]
fn content_blocks_as_text_skips_non_text() {
    let c = Content::Blocks(vec![
        ContentBlock::ToolResult {
            tool_use_id: "id".into(),
            content: "result".into(),
            is_error: false,
        },
        ContentBlock::Text {
            text: "found".into(),
        },
    ]);
    assert_eq!(c.as_text(), Some("found"));
}

#[test]
fn content_text_serde_round_trip() {
    let c = Content::text("hello world");
    let json = serde_json::to_string(&c).unwrap();
    let back: Content = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back);
}

#[test]
fn content_blocks_serde_round_trip() {
    let c = Content::Blocks(vec![
        ContentBlock::Text {
            text: "hello".into(),
        },
        ContentBlock::Image {
            source: layer0::content::ContentSource::Url {
                url: "https://example.com/img.png".into(),
            },
            media_type: "image/png".into(),
        },
        ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "read_file".into(),
            input: json!({"path": "/tmp/test"}),
        },
        ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "file contents".into(),
            is_error: false,
        },
    ]);
    let json = serde_json::to_string(&c).unwrap();
    let back: Content = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back);
}

#[test]
fn content_custom_block_round_trip() {
    let c = Content::Blocks(vec![ContentBlock::Custom {
        content_type: "audio".into(),
        data: json!({"codec": "opus", "samples": 48000}),
    }]);
    let json = serde_json::to_string(&c).unwrap();
    let back: Content = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OperatorInput / OperatorOutput round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn sample_operator_input() -> OperatorInput {
    let mut config = OperatorConfig::default();
    config.max_turns = Some(10);
    config.max_cost = Some(Decimal::new(100, 2)); // $1.00
    config.max_duration = Some(DurationMs::from_secs(60));
    config.model = Some("claude-sonnet-4-20250514".into());
    config.allowed_operators = Some(vec!["read_file".into()]);
    config.system_addendum = Some("Be concise.".into());

    let mut input = OperatorInput::new(
        Content::text("do something"),
        layer0::operator::TriggerType::User,
    );
    input.session = Some(SessionId::new("sess-1"));
    input.config = Some(config);
    input.metadata = json!({"trace_id": "abc123"});
    input
}

#[test]
fn operator_input_serde_round_trip() {
    let input = sample_operator_input();
    let json = serde_json::to_string(&input).unwrap();
    let back: OperatorInput = serde_json::from_str(&json).unwrap();
    assert_eq!(input.message, back.message);
    assert_eq!(input.trigger, back.trigger);
    assert_eq!(input.session, back.session);
    assert_eq!(input.metadata, back.metadata);
}

fn sample_operator_output() -> OperatorOutput {
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = 100;
    meta.tokens_out = 50;
    meta.cost = Decimal::new(5, 3); // $0.005
    meta.turns_used = 1;
    meta.sub_dispatches = vec![SubDispatchRecord::new(
        "read_file",
        DurationMs::from_millis(150),
        true,
    )];
    meta.duration = DurationMs::from_secs(2);

    let mut output = OperatorOutput::new(Content::text("done"), ExitReason::Complete);
    output.metadata = meta;
    output
}

#[test]
fn operator_output_serde_round_trip() {
    let output = sample_operator_output();
    let json = serde_json::to_string(&output).unwrap();
    let back: OperatorOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(output.message, back.message);
    assert_eq!(output.exit_reason, back.exit_reason);
}

#[test]
fn operator_metadata_default() {
    let m = OperatorMetadata::default();
    assert_eq!(m.tokens_in, 0);
    assert_eq!(m.tokens_out, 0);
    assert_eq!(m.cost, Decimal::ZERO);
    assert_eq!(m.turns_used, 0);
    assert!(m.sub_dispatches.is_empty());
    assert_eq!(m.duration, DurationMs::ZERO);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TriggerType / ExitReason Custom variant round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn trigger_type_custom_round_trip() {
    let t = layer0::operator::TriggerType::Custom("webhook".into());
    let json = serde_json::to_string(&t).unwrap();
    let back: layer0::operator::TriggerType = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}

#[test]
fn exit_reason_custom_round_trip() {
    let e = ExitReason::Custom("user_cancelled".into());
    let json = serde_json::to_string(&e).unwrap();
    let back: ExitReason = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back);
}

#[test]
fn exit_reason_observer_halt_round_trip() {
    let e = ExitReason::InterceptorHalt {
        reason: "budget exceeded".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: ExitReason = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back);
}

#[test]
fn exit_reason_safety_stop_round_trip() {
    let e = ExitReason::SafetyStop {
        reason: "refusal".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: ExitReason = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Effect round-trips (including Custom)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn effect_write_memory_round_trip() {
    let e = Effect::WriteMemory {
        scope: Scope::Session(SessionId::new("s1")),
        key: "notes".into(),
        value: json!({"text": "remember this"}),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_signal_round_trip() {
    let e = Effect::Signal {
        target: WorkflowId::new("wf-1"),
        payload: SignalPayload::new("user_feedback", json!({"rating": 5})),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_delegate_round_trip() {
    let e = Effect::Delegate {
        operator: OperatorId::new("helper"),
        input: Box::new(sample_operator_input()),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_custom_round_trip() {
    let e = Effect::Custom {
        effect_type: "send_email".into(),
        data: json!({"to": "user@example.com", "subject": "done"}),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scope round-trips (including Custom)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn scope_session_round_trip() {
    let s = Scope::Session(SessionId::new("s1"));
    let json = serde_json::to_string(&s).unwrap();
    let back: Scope = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn scope_agent_round_trip() {
    let s = Scope::Operator {
        workflow: WorkflowId::new("wf-1"),
        operator: OperatorId::new("a-1"),
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: Scope = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn scope_global_round_trip() {
    let s = Scope::Global;
    let json = serde_json::to_string(&s).unwrap();
    let back: Scope = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn scope_custom_round_trip() {
    let s = Scope::Custom("tenant:acme".into());
    let json = serde_json::to_string(&s).unwrap();
    let back: Scope = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// IsolationBoundary Custom round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn isolation_boundary_custom_round_trip() {
    let b = layer0::environment::IsolationBoundary::Custom {
        boundary_type: "firecracker".into(),
        config: json!({"kernel": "vmlinux", "rootfs": "rootfs.ext4"}),
    };
    let json = serde_json::to_string(&b).unwrap();
    let back: layer0::environment::IsolationBoundary = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn isolation_boundary_all_variants_round_trip() {
    let variants: Vec<layer0::environment::IsolationBoundary> = vec![
        layer0::environment::IsolationBoundary::Process,
        layer0::environment::IsolationBoundary::Container {
            image: Some("ubuntu:24.04".into()),
        },
        layer0::environment::IsolationBoundary::Gvisor,
        layer0::environment::IsolationBoundary::MicroVm,
        layer0::environment::IsolationBoundary::Wasm {
            runtime: Some("wasmtime".into()),
        },
        layer0::environment::IsolationBoundary::NetworkPolicy {
            rules: vec![{
                let mut rule = layer0::environment::NetworkRule::new(
                    "10.0.0.0/8",
                    layer0::environment::NetworkAction::Allow,
                );
                rule.port = Some(443);
                rule
            }],
        },
    ];
    for v in variants {
        let json = serde_json::to_string(&v).unwrap();
        let back: layer0::environment::IsolationBoundary = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EnvironmentSpec round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn environment_spec_round_trip() {
    let mut resources = layer0::environment::ResourceLimits::default();
    resources.cpu = Some("1.0".into());
    resources.memory = Some("2Gi".into());

    let mut api_rule = layer0::environment::NetworkRule::new(
        "api.anthropic.com",
        layer0::environment::NetworkAction::Allow,
    );
    api_rule.port = Some(443);

    let mut spec = EnvironmentSpec::default();
    spec.isolation = vec![layer0::environment::IsolationBoundary::Container {
        image: Some("python:3.12".into()),
    }];
    spec.credentials = vec![layer0::environment::CredentialRef::new(
        "api-key",
        layer0::secret::SecretSource::OsKeystore {
            service: "test".into(),
        },
        layer0::environment::CredentialInjection::EnvVar {
            var_name: "API_KEY".into(),
        },
    )];
    spec.resources = Some(resources);
    spec.network = Some(layer0::environment::NetworkPolicy::new(
        layer0::environment::NetworkAction::Deny,
        vec![api_rule],
    ));
    let json = serde_json::to_string(&spec).unwrap();
    let back: EnvironmentSpec = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Compaction policy round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn compaction_policy_round_trip() {
    let policies = [
        CompactionPolicy::Pinned,
        CompactionPolicy::Normal,
        CompactionPolicy::CompressFirst,
        CompactionPolicy::DiscardWhenDone,
    ];

#[test]
fn compaction_event_round_trip() {
    let e = CompactionEvent::ContextPressure {
        operator: OperatorId::new("a1"),
        fill_percent: 0.85,
        tokens_used: 85000,
        tokens_available: 15000,
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: CompactionEvent = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// State SearchResult round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn search_result_round_trip() {
    let mut r = SearchResult::new("notes/meeting", 0.95);
    r.snippet = Some("discussed the architecture...".into());
    let json = serde_json::to_string(&r).unwrap();
    let back: SearchResult = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Wire format stability: Decimal serializes as string
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn decimal_serializes_as_string_not_number() {
    // With rust_decimal's serde-str feature, Decimal serializes as "1.23"
    // not as 1.23 (number) or {"lo":...,"mid":...,...} (struct).
    // This is critical for wire-format stability across implementations.
    let cost = Decimal::new(123, 2); // 1.23
    let json = serde_json::to_value(cost).unwrap();
    assert!(
        json.is_string(),
        "Decimal must serialize as a JSON string, got: {json}"
    );
    assert_eq!(json.as_str().unwrap(), "1.23");
}

#[test]
fn decimal_zero_serializes_as_string() {
    let cost = Decimal::ZERO;
    let json = serde_json::to_value(cost).unwrap();
    assert!(
        json.is_string(),
        "Decimal::ZERO must serialize as string, got: {json}"
    );
    assert_eq!(json.as_str().unwrap(), "0");
}

#[test]
fn decimal_in_operator_metadata_wire_format() {
    // Verify Decimal format is preserved when nested in protocol types.
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = 100;
    meta.tokens_out = 50;
    meta.cost = Decimal::new(5, 3); // 0.005
    meta.turns_used = 1;
    let json = serde_json::to_value(&meta).unwrap();
    let cost_val = &json["cost"];
    assert!(
        cost_val.is_string(),
        "cost in OperatorMetadata must be string, got: {cost_val}"
    );
    assert_eq!(cost_val.as_str().unwrap(), "0.005");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Wire format stability: Content serialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn content_text_serializes_as_bare_string() {
    // Content::Text serializes as a bare JSON string (untagged).
    let c = Content::text("hello");
    let json = serde_json::to_value(&c).unwrap();
    assert!(
        json.is_string(),
        "Content::Text must serialize as bare string, got: {json}"
    );
    assert_eq!(json.as_str().unwrap(), "hello");
}

#[test]
fn content_blocks_serializes_as_array() {
    // Content::Blocks serializes as a JSON array (untagged).
    let c = Content::Blocks(vec![ContentBlock::Text {
        text: "hello".into(),
    }]);
    let json = serde_json::to_value(&c).unwrap();
    assert!(
        json.is_array(),
        "Content::Blocks must serialize as array, got: {json}"
    );
}

#[test]
fn content_text_and_blocks_are_structurally_distinct() {
    // The untagged Content enum is safe because String and Array
    // are structurally distinct JSON types. Verify round-trip of both
    // from the same test to prove no cross-contamination.
    let text = Content::text("hello");
    let blocks = Content::Blocks(vec![ContentBlock::Text {
        text: "hello".into(),
    }]);

    let text_json = serde_json::to_string(&text).unwrap();
    let blocks_json = serde_json::to_string(&blocks).unwrap();

    let text_back: Content = serde_json::from_str(&text_json).unwrap();
    let blocks_back: Content = serde_json::from_str(&blocks_json).unwrap();

    assert_eq!(text, text_back);
    assert_eq!(blocks, blocks_back);
    // Verify they don't cross-contaminate
    assert_ne!(text_json, blocks_json);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Wire format stability: DurationMs serializes as integer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn duration_ms_serializes_as_integer() {
    let d = DurationMs::from_millis(1500);
    let json = serde_json::to_value(d).unwrap();
    assert!(
        json.is_u64(),
        "DurationMs must serialize as integer, got: {json}"
    );
    assert_eq!(json.as_u64().unwrap(), 1500);
}

#[test]
fn duration_ms_zero_serializes_as_zero() {
    let d = DurationMs::ZERO;
    let json = serde_json::to_value(d).unwrap();
    assert_eq!(json.as_u64().unwrap(), 0);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Forward compatibility: Custom variants accept unknown data
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn trigger_type_custom_preserves_unknown_variant() {
    // A new trigger type should survive round-trip through Custom.
    let json = r#"{"custom":"iot_sensor_event"}"#;
    let t: layer0::operator::TriggerType = serde_json::from_str(json).unwrap();
    assert_eq!(
        t,
        layer0::operator::TriggerType::Custom("iot_sensor_event".into())
    );
}

#[test]
fn exit_reason_custom_preserves_unknown_variant() {
    let json = r#"{"custom":"human_takeover"}"#;
    let e: ExitReason = serde_json::from_str(json).unwrap();
    assert_eq!(e, ExitReason::Custom("human_takeover".into()));
}

#[test]
fn scope_custom_preserves_unknown_scope() {
    let json = r#"{"custom":"tenant:acme-corp"}"#;
    let s: Scope = serde_json::from_str(json).unwrap();
    assert_eq!(s, Scope::Custom("tenant:acme-corp".into()));
}

#[test]
fn effect_custom_preserves_unknown_effect() {
    let json = r##"{"type":"custom","effect_type":"send_slack","data":{"channel":"#ops"}}"##;
    let e: Effect = serde_json::from_str(json).unwrap();
    let reserialized = serde_json::to_string(&e).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed["type"], "custom");
    assert_eq!(reparsed["effect_type"], "send_slack");
}

#[test]
fn content_block_custom_preserves_unknown_modality() {
    let json =
        r#"{"type":"custom","content_type":"audio","data":{"codec":"opus","sample_rate":48000}}"#;
    let b: ContentBlock = serde_json::from_str(json).unwrap();
    let reserialized = serde_json::to_string(&b).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed["type"], "custom");
    assert_eq!(reparsed["content_type"], "audio");
    assert_eq!(reparsed["data"]["codec"], "opus");
}

#[test]
fn isolation_boundary_custom_preserves_unknown_isolation() {
    let json = r#"{"type":"custom","boundary_type":"kata_container","config":{"runtime":"qemu"}}"#;
    let b: layer0::environment::IsolationBoundary = serde_json::from_str(json).unwrap();
    let reserialized = serde_json::to_string(&b).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed["type"], "custom");
    assert_eq!(reparsed["boundary_type"], "kata_container");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Blanket StateReader impl
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Verify that any concrete StateStore type implements StateReader
// via the blanket impl (trait object upcasting is not stable,
// so we test with generics)
fn _takes_state_reader<T: StateReader + ?Sized>(_r: &T) {}
fn _takes_state_store<T: StateStore>(s: &T) {
    // This compiles because of the blanket impl: T: StateStore => T: StateReader
    _takes_state_reader(s);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Error types display
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn operator_error_display() {
    let e = OperatorError::model("rate limited");
    assert_eq!(e.to_string(), "model error: rate limited");

    let e = OperatorError::SubDispatch {
        operator: "bash".into(),
        source: "command failed".to_string().into(),
    };
    assert_eq!(e.to_string(), "sub-dispatch error in bash: command failed");
}

#[test]
fn orch_error_display() {
    let e = OrchError::OperatorNotFound("missing-agent".into());
    assert_eq!(e.to_string(), "operator not found: missing-agent");
}

#[test]
fn state_error_display() {
    let e = StateError::NotFound {
        scope: "session".into(),
        key: "notes".into(),
    };
    assert_eq!(e.to_string(), "not found: session/notes");
}

#[test]
fn env_error_display() {
    let e = EnvError::ProvisionFailed("docker not available".into());
    assert_eq!(e.to_string(), "provisioning failed: docker not available");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Error Display — remaining variants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn operator_error_display_remaining_variants() {
    assert_eq!(
        OperatorError::context_assembly(std::io::Error::other("bad ctx")).to_string(),
        "context assembly: bad ctx"
    );
    assert_eq!(
        OperatorError::retryable("timeout").to_string(),
        "retryable: timeout"
    );
    assert_eq!(
        OperatorError::non_retryable("invalid").to_string(),
        "non-retryable: invalid"
    );
    let boxed: Box<dyn std::error::Error + Send + Sync> = "inner error".into();
    assert_eq!(OperatorError::Other(boxed).to_string(), "inner error");
}

#[test]
fn orch_error_display_remaining_variants() {
    assert_eq!(
        OrchError::WorkflowNotFound("wf-1".into()).to_string(),
        "workflow not found: wf-1"
    );
    assert_eq!(
        OrchError::DispatchFailed("timeout".into()).to_string(),
        "dispatch failed: timeout"
    );
    assert_eq!(
        OrchError::SignalFailed("no handler".into()).to_string(),
        "signal delivery failed: no handler"
    );
    let inner = OperatorError::model("provider down");
    assert_eq!(
        OrchError::OperatorError(inner).to_string(),
        "operator error: model error: provider down"
    );
    let boxed: Box<dyn std::error::Error + Send + Sync> = "orch inner".into();
    assert_eq!(OrchError::Other(boxed).to_string(), "orch inner");
}

#[test]
fn state_error_display_remaining_variants() {
    assert_eq!(
        StateError::WriteFailed("disk full".into()).to_string(),
        "write failed: disk full"
    );
    assert_eq!(
        StateError::Serialization("invalid json".into()).to_string(),
        "serialization error: invalid json"
    );
    let boxed: Box<dyn std::error::Error + Send + Sync> = "state inner".into();
    assert_eq!(StateError::Other(boxed).to_string(), "state inner");
}

#[test]
fn env_error_display_remaining_variants() {
    assert_eq!(
        EnvError::IsolationViolation("escaped sandbox".into()).to_string(),
        "isolation violation: escaped sandbox"
    );
    assert_eq!(
        EnvError::CredentialFailed("secret not found".into()).to_string(),
        "credential injection failed: secret not found"
    );
    assert_eq!(
        EnvError::ResourceExceeded("OOM".into()).to_string(),
        "resource limit exceeded: OOM"
    );
    let inner = OperatorError::model("provider down");
    assert_eq!(
        EnvError::OperatorError(inner).to_string(),
        "operator error: model error: provider down"
    );
    let boxed: Box<dyn std::error::Error + Send + Sync> = "env inner".into();
    assert_eq!(EnvError::Other(boxed).to_string(), "env inner");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BudgetDecision round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn budget_decision_downgrade_round_trip() {
    let d = layer0::lifecycle::BudgetDecision::DowngradeModel {
        from: "claude-opus-4-20250514".into(),
        to: "claude-haiku-4-5-20251001".into(),
    };
    let json = serde_json::to_string(&d).unwrap();
    let back: layer0::lifecycle::BudgetDecision = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_handoff_round_trip() {
    let e = Effect::Handoff {
        operator: OperatorId::new("specialist"),
        state: json!({"context": "user needs help with billing"}),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_delete_memory_round_trip() {
    let e = Effect::DeleteMemory {
        scope: Scope::Global,
        key: "temp_notes".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MemoryTier and StoreOptions round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn memory_tier_round_trip() {
    use layer0::MemoryTier;
    let tiers = [MemoryTier::Hot, MemoryTier::Warm, MemoryTier::Cold];
    for tier in tiers {
        let json = serde_json::to_string(&tier).unwrap();
        let back: MemoryTier = serde_json::from_str(&json).unwrap();
        assert_eq!(tier, back);
    }
}

#[test]
fn store_options_default() {
    use layer0::StoreOptions;
    let opts = StoreOptions::default();
    assert!(opts.tier.is_none());
}

#[test]
fn write_memory_with_tier_round_trip() {
    use layer0::MemoryTier;
    let e = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: json!(1),
        tier: Some(MemoryTier::Warm),
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };
    let json = serde_json::to_value(&e).unwrap();
    let back: Effect = serde_json::from_value(json.clone()).unwrap();
    let json2 = serde_json::to_value(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn write_memory_tier_omitted_when_none() {
    let e = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: json!(1),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };
    let json = serde_json::to_value(&e).unwrap();
    assert!(
        json.get("tier").is_none(),
        "tier: None must not appear in serialized JSON, got: {json}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Lifetime / ContentKind / StoreOptions round-trips
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_lifetime_serde() {
    use layer0::Lifetime;
    for v in [Lifetime::Transient, Lifetime::Session, Lifetime::Durable] {
        let json = serde_json::to_string(&v).unwrap();
        let back: Lifetime = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn test_content_kind_serde() {
    use layer0::ContentKind;
    let cases = [
        ContentKind::Episodic,
        ContentKind::Semantic,
        ContentKind::Procedural,
        ContentKind::Structural,
        ContentKind::Custom("domain::MyKind".into()),
    ];
    for v in cases {
        let json = serde_json::to_string(&v).unwrap();
        let back: ContentKind = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn test_store_options_serde() {
    use layer0::state::StoreOptions;
    use layer0::{ContentKind, DurationMs, Lifetime, MemoryTier};
    let opts = StoreOptions {
        tier: Some(MemoryTier::Hot),
        lifetime: Some(Lifetime::Durable),
        content_kind: Some(ContentKind::Semantic),
        salience: Some(0.9),
        ttl: Some(DurationMs::from_secs(3600)),
    };
    let json = serde_json::to_string(&opts).unwrap();
    let back: StoreOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(back.lifetime, Some(Lifetime::Durable));
    assert_eq!(back.content_kind, Some(ContentKind::Semantic));
    assert_eq!(back.salience, Some(0.9));
}

#[test]
fn test_write_memory_effect_with_new_fields_serde() {
    use layer0::{ContentKind, DurationMs, Lifetime};
    let e = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: serde_json::json!(42),
        tier: None,
        lifetime: Some(Lifetime::Session),
        content_kind: Some(ContentKind::Episodic),
        salience: Some(0.5),
        ttl: Some(DurationMs::from_millis(60_000)),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    if let Effect::WriteMemory {
        lifetime,
        content_kind,
        salience,
        ttl,
        ..
    } = back
    {
        assert_eq!(lifetime, Some(Lifetime::Session));
        assert_eq!(content_kind, Some(ContentKind::Episodic));
        assert_eq!(salience, Some(0.5));
        assert!(ttl.is_some());
    } else {
        panic!("wrong variant");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OperatorConfig default
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn operator_config_default_all_none() {
    let c = OperatorConfig::default();
    assert!(c.max_turns.is_none());
    assert!(c.max_cost.is_none());
    assert!(c.max_duration.is_none());
    assert!(c.model.is_none());
    assert!(c.allowed_operators.is_none());
    assert!(c.system_addendum.is_none());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CredentialInjection variants round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn credential_injection_variants_round_trip() {
    let variants: Vec<layer0::environment::CredentialInjection> = vec![
        layer0::environment::CredentialInjection::EnvVar {
            var_name: "API_KEY".into(),
        },
        layer0::environment::CredentialInjection::File {
            path: "/run/secrets/key".into(),
        },
        layer0::environment::CredentialInjection::Sidecar,
    ];
    for v in variants {
        let json = serde_json::to_string(&v).unwrap();
        let back: layer0::environment::CredentialInjection = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ExitReason all variants round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn exit_reason_all_variants_round_trip() {
    let reasons = vec![
        ExitReason::Complete,
        ExitReason::MaxTurns,
        ExitReason::BudgetExhausted,
        ExitReason::CircuitBreaker,
        ExitReason::Timeout,
        ExitReason::InterceptorHalt {
            reason: "safety".into(),
        },
        ExitReason::Error,
        ExitReason::Custom("special".into()),
    ];
    for reason in &reasons {
        let json = serde_json::to_string(reason).unwrap();
        let back: ExitReason = serde_json::from_str(&json).unwrap();
        assert_eq!(*reason, back);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Secret types — serde roundtrips and Display tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use layer0::secret::{SecretAccessEvent, SecretAccessOutcome, SecretSource};

#[test]
fn secret_source_all_variants_round_trip() {
    let sources = vec![
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/api-key".into(),
        },
        SecretSource::AwsSecretsManager {
            secret_id: "arn:aws:secretsmanager:us-east-1:123:secret:api-key".into(),
            region: Some("us-east-1".into()),
        },
        SecretSource::GcpSecretManager {
            project: "my-project".into(),
            secret_id: "api-key".into(),
        },
        SecretSource::AzureKeyVault {
            vault_url: "https://myvault.vault.azure.net".into(),
            secret_name: "api-key".into(),
        },
        SecretSource::OsKeystore {
            service: "skg-test".into(),
        },
        SecretSource::Kubernetes {
            namespace: "default".into(),
            name: "api-secrets".into(),
            key: "anthropic-key".into(),
        },
        SecretSource::Hardware { slot: "9a".into() },
        SecretSource::Custom {
            provider: "1password".into(),
            config: json!({"vault": "Engineering"}),
        },
    ];
    for source in sources {
        let json = serde_json::to_string(&source).unwrap();
        let back: SecretSource = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
    }
}

#[test]
fn secret_access_outcome_all_variants_round_trip() {
    let outcomes = vec![
        SecretAccessOutcome::Resolved,
        SecretAccessOutcome::Denied,
        SecretAccessOutcome::Failed,
        SecretAccessOutcome::Renewed,
        SecretAccessOutcome::Released,
    ];
    for outcome in &outcomes {
        let json = serde_json::to_string(outcome).unwrap();
        let back: SecretAccessOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(*outcome, back);
    }
}

#[test]
fn secret_access_event_round_trip() {
    let event = SecretAccessEvent::new(
        "anthropic-api-key",
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/api-key".into(),
        },
        SecretAccessOutcome::Resolved,
        1740000000000,
    );
    let json = serde_json::to_string(&event).unwrap();
    let back: SecretAccessEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.credential_name, "anthropic-api-key");
    assert_eq!(back.outcome, SecretAccessOutcome::Resolved);
    assert_eq!(back.timestamp_ms, 1740000000000);
}

#[test]
fn secret_access_event_with_all_fields() {
    let mut event = SecretAccessEvent::new(
        "db-password",
        SecretSource::AwsSecretsManager {
            secret_id: "prod/db/password".into(),
            region: Some("us-west-2".into()),
        },
        SecretAccessOutcome::Denied,
        1740000000000,
    );
    event.lease_id = Some("lease-abc-123".into());
    event.lease_ttl_secs = Some(3600);
    event.reason = Some("policy: requires mfa".into());
    event.workflow_id = Some("wf-001".into());
    event.operator_id = Some("agent-research".into());
    event.trace_id = Some("trace-xyz".into());

    let json = serde_json::to_string(&event).unwrap();
    let back: SecretAccessEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.lease_id.as_deref(), Some("lease-abc-123"));
    assert_eq!(back.lease_ttl_secs, Some(3600));
    assert_eq!(back.reason.as_deref(), Some("policy: requires mfa"));
    assert_eq!(back.workflow_id.as_deref(), Some("wf-001"));
    assert_eq!(back.operator_id.as_deref(), Some("agent-research"));
    assert_eq!(back.trace_id.as_deref(), Some("trace-xyz"));
}

#[test]
fn secret_source_kind_tags() {
    assert_eq!(
        SecretSource::Vault {
            mount: "s".into(),
            path: "p".into()
        }
        .kind(),
        "vault"
    );
    assert_eq!(
        SecretSource::AwsSecretsManager {
            secret_id: "x".into(),
            region: None
        }
        .kind(),
        "aws"
    );
    assert_eq!(
        SecretSource::GcpSecretManager {
            project: "p".into(),
            secret_id: "s".into()
        }
        .kind(),
        "gcp"
    );
    assert_eq!(
        SecretSource::AzureKeyVault {
            vault_url: "u".into(),
            secret_name: "n".into()
        }
        .kind(),
        "azure"
    );
    assert_eq!(
        SecretSource::OsKeystore {
            service: "s".into()
        }
        .kind(),
        "os_keystore"
    );
    assert_eq!(
        SecretSource::Kubernetes {
            namespace: "n".into(),
            name: "n".into(),
            key: "k".into()
        }
        .kind(),
        "kubernetes"
    );
    assert_eq!(
        SecretSource::Hardware { slot: "9a".into() }.kind(),
        "hardware"
    );
    assert_eq!(
        SecretSource::Custom {
            provider: "p".into(),
            config: json!({})
        }
        .kind(),
        "custom"
    );
}

#[test]
fn credential_ref_with_source_round_trip() {
    use layer0::environment::{CredentialInjection, CredentialRef};

    let cred = CredentialRef::new(
        "anthropic-api-key",
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/anthropic".into(),
        },
        CredentialInjection::EnvVar {
            var_name: "ANTHROPIC_API_KEY".into(),
        },
    );
    let json = serde_json::to_string(&cred).unwrap();
    let back: CredentialRef = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "anthropic-api-key");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GraphRAG: SearchOptions, MemoryLink, LinkMemory, UnlinkMemory
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn search_options_default_all_none() {
    use layer0::SearchOptions;
    let opts = SearchOptions::default();
    assert!(opts.min_score.is_none());
    assert!(opts.content_kind.is_none());
    assert!(opts.tier.is_none());
    assert!(opts.max_depth.is_none());
}

#[test]
fn search_options_full_round_trip() {
    use layer0::{ContentKind, MemoryTier, SearchOptions};
    let opts = SearchOptions {
        min_score: Some(0.75),
        content_kind: Some(ContentKind::Semantic),
        tier: Some(MemoryTier::Hot),
        max_depth: Some(3),
    };
    let json = serde_json::to_string(&opts).unwrap();
    let back: SearchOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(back.min_score, Some(0.75));
    assert_eq!(back.content_kind, Some(ContentKind::Semantic));
    assert_eq!(back.tier, Some(MemoryTier::Hot));
    assert_eq!(back.max_depth, Some(3));
}

#[test]
fn search_options_empty_omits_all_fields() {
    use layer0::SearchOptions;
    let opts = SearchOptions::default();
    let json = serde_json::to_value(&opts).unwrap();
    // All Option fields with skip_serializing_if must be absent
    assert!(json.get("min_score").is_none());
    assert!(json.get("content_kind").is_none());
    assert!(json.get("tier").is_none());
    assert!(json.get("max_depth").is_none());
}

#[test]
fn memory_link_new_round_trip() {
    use layer0::MemoryLink;
    let link = MemoryLink::new("key/a", "key/b", "references");
    assert_eq!(link.from_key, "key/a");
    assert_eq!(link.to_key, "key/b");
    assert_eq!(link.relation, "references");
    assert!(link.metadata.is_none());
    let json = serde_json::to_string(&link).unwrap();
    let back: MemoryLink = serde_json::from_str(&json).unwrap();
    assert_eq!(back.from_key, link.from_key);
    assert_eq!(back.to_key, link.to_key);
    assert_eq!(back.relation, link.relation);
    assert!(back.metadata.is_none());
}

#[test]
fn memory_link_with_metadata_round_trip() {
    use layer0::MemoryLink;
    let mut link = MemoryLink::new("a", "b", "supersedes");
    link.metadata = Some(json!({"weight": 0.9}));
    let json = serde_json::to_string(&link).unwrap();
    let back: MemoryLink = serde_json::from_str(&json).unwrap();
    assert_eq!(back.metadata.as_ref().unwrap()["weight"], json!(0.9));
    // Verify the JSON round-trip is stable
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
}

#[test]
fn effect_link_memory_round_trip() {
    use layer0::MemoryLink;
    let e = Effect::LinkMemory {
        scope: Scope::Global,
        link: MemoryLink::new("notes/meeting", "decisions/arch", "references"),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
    // Verify the type tag is correct
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["type"], "link_memory");
}

#[test]
fn effect_unlink_memory_round_trip() {
    let e = Effect::UnlinkMemory {
        scope: Scope::Session(SessionId::new("s1")),
        from_key: "notes/meeting".into(),
        to_key: "decisions/arch".into(),
        relation: "references".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Effect = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&back).unwrap();
    assert_eq!(json, json2);
    // Verify the type tag is correct
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["type"], "unlink_memory");
}

// Compile-time proof: Box<dyn StateStore> and Box<dyn StateReader> are still
// object-safe after adding the new default methods.
// The new methods use no generics and no Self in return position — safe.
fn _assert_state_store_still_object_safe(_: &dyn StateStore) {}
fn _assert_state_reader_still_object_safe(_: &dyn StateReader) {}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CollectedDispatch / collect_all
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn collect_all_preserves_intermediate_events() {
    let (handle, sender) = DispatchHandle::channel(DispatchId::new("test-collect-all"));

    let send_task = tokio::spawn(async move {
        sender
            .send(DispatchEvent::Progress {
                content: Content::text("thinking..."),
            })
            .await
            .unwrap();
        sender
            .send(DispatchEvent::EffectEmitted {
                effect: Effect::Progress {
                    content: Content::text("progress-effect"),
                },
            })
            .await
            .unwrap();
        sender
            .send(DispatchEvent::Completed {
                output: OperatorOutput::new(Content::text("done"), ExitReason::Complete),
            })
            .await
            .unwrap();
        // Drop sender to close the channel.
    });

    let result = handle.collect_all().await.unwrap();
    send_task.await.unwrap();

    // Both Progress and EffectEmitted should be in events.
    assert_eq!(result.events.len(), 2);
    assert!(matches!(result.events[0], DispatchEvent::Progress { .. }));
    assert!(matches!(
        result.events[1],
        DispatchEvent::EffectEmitted { .. }
    ));

    // Effects should also be populated in output.
    assert_eq!(result.output.effects.len(), 1);
    assert_eq!(result.output.message, Content::text("done"));
}

#[tokio::test]
async fn collect_discards_progress_but_collect_all_preserves_it() {
    // collect() discards Progress events.
    let (handle, sender) = DispatchHandle::channel(DispatchId::new("test-collect-discard"));
    let send_task = tokio::spawn(async move {
        sender
            .send(DispatchEvent::Progress {
                content: Content::text("step 1"),
            })
            .await
            .unwrap();
        sender
            .send(DispatchEvent::Completed {
                output: OperatorOutput::new(Content::text("done"), ExitReason::Complete),
            })
            .await
            .unwrap();
    });

    let output = handle.collect().await.unwrap();
    send_task.await.unwrap();
    // collect() gives no way to observe the Progress event.
    assert_eq!(output.message, Content::text("done"));
    assert!(output.effects.is_empty());
}

#[tokio::test]
async fn collect_all_empty_events_on_immediate_complete() {
    let (handle, sender) = DispatchHandle::channel(DispatchId::new("test-empty"));
    tokio::spawn(async move {
        sender
            .send(DispatchEvent::Completed {
                output: OperatorOutput::new(Content::text("ok"), ExitReason::Complete),
            })
            .await
            .unwrap();
    });

    let result = handle.collect_all().await.unwrap();
    assert!(result.events.is_empty());
    assert_eq!(result.output.message, Content::text("ok"));
}

#[tokio::test]
async fn collect_all_returns_error_on_failure() {
    let (handle, sender) = DispatchHandle::channel(DispatchId::new("test-fail"));
    tokio::spawn(async move {
        sender
            .send(DispatchEvent::Progress {
                content: Content::text("working..."),
            })
            .await
            .unwrap();
        sender
            .send(DispatchEvent::Failed {
                error: OrchError::DispatchFailed("boom".into()),
            })
            .await
            .unwrap();
    });

    let err = handle.collect_all().await.unwrap_err();
    assert!(matches!(err, OrchError::DispatchFailed(_)));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Graph operations on InMemoryStore
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "test-utils")]
mod graph_tests {
    use layer0::effect::Scope;
    use layer0::state::{MemoryLink, StateStore};
    use layer0::test_utils::InMemoryStore;

    #[tokio::test]
    async fn graph_link_and_traverse() {
        let store = InMemoryStore::new();
        let scope = Scope::Global;

        // a -> b -> c
        store
            .link(&scope, &MemoryLink::new("a", "b", "related"))
            .await
            .unwrap();
        store
            .link(&scope, &MemoryLink::new("b", "c", "related"))
            .await
            .unwrap();

        // depth 1: only b reachable from a
        let depth1 = store.traverse(&scope, "a", None, 1).await.unwrap();
        assert_eq!(depth1, vec!["b"]);

        // depth 2: b and c reachable from a
        let mut depth2 = store.traverse(&scope, "a", None, 2).await.unwrap();
        depth2.sort();
        assert_eq!(depth2, vec!["b", "c"]);
    }

    #[tokio::test]
    async fn graph_unlink_removes_edge() {
        let store = InMemoryStore::new();
        let scope = Scope::Global;

        store
            .link(&scope, &MemoryLink::new("x", "y", "depends_on"))
            .await
            .unwrap();

        let before = store.traverse(&scope, "x", None, 1).await.unwrap();
        assert_eq!(before, vec!["y"]);

        store.unlink(&scope, "x", "y", "depends_on").await.unwrap();

        let after = store.traverse(&scope, "x", None, 1).await.unwrap();
        assert!(after.is_empty(), "unlink must remove the edge");
    }

    #[tokio::test]
    async fn graph_traverse_filters_by_relation() {
        let store = InMemoryStore::new();
        let scope = Scope::Global;

        store
            .link(&scope, &MemoryLink::new("a", "b", "references"))
            .await
            .unwrap();
        store
            .link(&scope, &MemoryLink::new("a", "c", "supersedes"))
            .await
            .unwrap();

        // Filter by "references" — only b
        let refs = store
            .traverse(&scope, "a", Some("references"), 1)
            .await
            .unwrap();
        assert_eq!(refs, vec!["b"]);

        // Filter by "supersedes" — only c
        let sups = store
            .traverse(&scope, "a", Some("supersedes"), 1)
            .await
            .unwrap();
        assert_eq!(sups, vec!["c"]);

        // No filter — both b and c
        let mut all = store.traverse(&scope, "a", None, 1).await.unwrap();
        all.sort();
        assert_eq!(all, vec!["b", "c"]);
    }
}
