use layer0::{
    ApprovalFacts, AuthFacts, CapabilityDescriptor, CapabilityFilter, CapabilityId, CapabilityKind,
    CapabilityModality, ExecutionClass, SchedulingFacts, StreamingSupport,
};
use serde_json::json;

#[test]
fn descriptor_fixture_round_trip_tool() {
    let fixture = include_str!("golden/v2/capability-descriptor-tool.json");
    let descriptor: CapabilityDescriptor = serde_json::from_str(fixture).expect("deserialize");
    assert_eq!(descriptor.kind, CapabilityKind::Tool);
    assert_eq!(descriptor.id.as_str(), "memory_store");
    assert_eq!(descriptor.streaming, StreamingSupport::None);
    assert_eq!(
        descriptor.scheduling.execution_class,
        ExecutionClass::Shared
    );
    assert_eq!(descriptor.approval, ApprovalFacts::None);
    assert_eq!(descriptor.auth, AuthFacts::Caller);
    assert_eq!(descriptor.accepts, vec![CapabilityModality::Json]);

    let encoded = serde_json::to_value(&descriptor).expect("serialize");
    assert_eq!(encoded["kind"], json!("tool"));
    assert!(encoded.get("scheduling").is_some());
    assert!(encoded.get("approval").is_some());
    assert!(encoded.get("auth").is_some());
}

#[test]
fn descriptor_fixture_round_trip_resource() {
    let fixture = include_str!("golden/v2/capability-descriptor-resource.json");
    let descriptor: CapabilityDescriptor = serde_json::from_str(fixture).expect("deserialize");
    assert_eq!(descriptor.kind, CapabilityKind::Resource);
    assert_eq!(descriptor.id.as_str(), "state://global/config");
    assert_eq!(
        descriptor.scheduling.execution_class,
        ExecutionClass::Shared
    );
    assert_eq!(descriptor.scheduling.max_concurrency, Some(32));
    assert_eq!(descriptor.approval, ApprovalFacts::None);
    assert_eq!(
        descriptor.auth,
        AuthFacts::Service {
            scopes: vec!["state.read".to_string()]
        }
    );

    let encoded = serde_json::to_value(&descriptor).expect("serialize");
    assert_eq!(encoded["kind"], json!("resource"));
    assert!(encoded.get("scheduling").is_some());
}

#[test]
fn filter_fixture_round_trip_and_semantics() {
    let filter_fixture = include_str!("golden/v2/capability-filter.json");
    let filter: CapabilityFilter = serde_json::from_str(filter_fixture).expect("deserialize");
    assert_eq!(
        filter.kinds,
        vec![CapabilityKind::Tool, CapabilityKind::Resource]
    );
    assert_eq!(filter.name_contains.as_deref(), Some("memory"));
    assert_eq!(filter.requires_streaming, Some(false));
    assert_eq!(filter.requires_approval, Some(true));
    assert_eq!(filter.tags, vec!["state".to_string()]);

    let mut descriptor = CapabilityDescriptor::new(
        CapabilityId::new("memory_guarded"),
        CapabilityKind::Tool,
        "memory_guarded",
        "Requires runtime approval",
        SchedulingFacts::new(ExecutionClass::Exclusive, false, false, false, None),
        ApprovalFacts::RuntimePolicy,
        AuthFacts::Open,
    );
    descriptor.tags = vec!["state".to_string()];
    assert!(descriptor.matches_filter(&filter));

    let encoded = serde_json::to_value(&filter).expect("serialize");
    assert_eq!(encoded["kinds"], json!(["tool", "resource"]));
    assert_eq!(encoded["requires_approval"], json!(true));
}
