#[allow(deprecated)]
use layer0::error::{EnvError, ErrorCode, OperatorError, OrchError, ProtocolError};
use serde_json::json;

// ── ProtocolError serde round-trip ──────────────────────────────────────────

#[test]
fn protocol_error_round_trip_all_codes() {
    let codes = [
        (ErrorCode::NotFound, "not_found"),
        (ErrorCode::InvalidInput, "invalid_input"),
        (ErrorCode::Unavailable, "unavailable"),
        (ErrorCode::Conflict, "conflict"),
        (ErrorCode::Internal, "internal"),
    ];
    for (code, expected_wire) in codes {
        let pe = ProtocolError::new(code, "test", false);
        let json = serde_json::to_value(&pe).expect("serialize");
        assert_eq!(json["code"], json!(expected_wire), "code wire name mismatch");
        let back: ProtocolError = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.code, code);
    }
}

// ── Golden fixture ──────────────────────────────────────────────────────────

#[test]
fn protocol_error_unavailable_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/protocol-error-unavailable.json"))
            .expect("fixture json");
    let pe: ProtocolError = serde_json::from_value(fixture.clone()).expect("deserialize");
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
    assert_eq!(pe.details.get("provider").map(String::as_str), Some("anthropic"));
    let reencoded = serde_json::to_value(&pe).expect("serialize");
    assert_eq!(reencoded, fixture);
}

// ── From<OperatorError> mappings ────────────────────────────────────────────

#[allow(deprecated)]
#[test]
fn operator_error_model_retryable_maps_to_unavailable_retryable() {
    let e = OperatorError::model_retryable(std::io::Error::other("transient"));
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
    assert_eq!(pe.details.get("kind").map(String::as_str), Some("operator_error"));
    assert_eq!(pe.details.get("variant").map(String::as_str), Some("model"));
}

#[allow(deprecated)]
#[test]
fn operator_error_model_permanent_maps_to_internal() {
    let e = OperatorError::model("permanent");
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Internal);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn operator_error_sub_dispatch_maps_to_unavailable_with_details() {
    let e = OperatorError::SubDispatch {
        operator: "child".into(),
        source: "timeout".into(),
    };
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(!pe.retryable);
    assert_eq!(pe.details.get("variant").map(String::as_str), Some("sub_dispatch"));
    assert_eq!(pe.details.get("operator").map(String::as_str), Some("child"));
}

#[allow(deprecated)]
#[test]
fn operator_error_context_assembly_maps_to_invalid_input() {
    let e = OperatorError::context_assembly(std::io::Error::other("bad context"));
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::InvalidInput);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn operator_error_retryable_maps_to_unavailable() {
    let e = OperatorError::retryable("please retry");
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[allow(deprecated)]
#[test]
fn operator_error_non_retryable_maps_to_internal() {
    let e = OperatorError::non_retryable("fatal");
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Internal);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn operator_error_halted_maps_to_conflict() {
    let e = OperatorError::Halted {
        reason: "policy".into(),
    };
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Conflict);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn operator_error_other_maps_to_internal() {
    let e = OperatorError::Other("catch-all".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Internal);
    assert!(!pe.retryable);
}

// ── From<OrchError> mappings ────────────────────────────────────────────────

#[allow(deprecated)]
#[test]
fn orch_error_operator_not_found_maps_to_not_found() {
    let e = OrchError::OperatorNotFound("echo".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::NotFound);
    assert!(!pe.retryable);
    assert_eq!(pe.details.get("variant").map(String::as_str), Some("operator_not_found"));
    assert_eq!(pe.details.get("name").map(String::as_str), Some("echo"));
}

#[allow(deprecated)]
#[test]
fn orch_error_workflow_not_found_maps_to_not_found() {
    let e = OrchError::WorkflowNotFound("wf-1".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::NotFound);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn orch_error_dispatch_failed_maps_to_unavailable_retryable() {
    let e = OrchError::DispatchFailed("timeout".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[allow(deprecated)]
#[test]
fn orch_error_signal_failed_maps_to_unavailable_retryable() {
    let e = OrchError::SignalFailed("network".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[allow(deprecated)]
#[test]
fn orch_error_operator_error_propagates_inner_mapping() {
    let inner = OperatorError::Halted {
        reason: "policy".into(),
    };
    let e = OrchError::OperatorError(inner);
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Conflict);
    assert!(!pe.retryable);
}

#[allow(deprecated)]
#[test]
fn orch_error_environment_error_maps_correctly() {
    let env = EnvError::ProvisionFailed("no capacity".into());
    let e = OrchError::EnvironmentError(env);
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[allow(deprecated)]
#[test]
fn orch_error_other_maps_to_internal() {
    let e = OrchError::Other("catch-all".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Internal);
    assert!(!pe.retryable);
}
