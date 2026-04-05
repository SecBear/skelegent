use layer0::error::{EnvError, ErrorCode, ProtocolError};
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
        assert_eq!(
            json["code"],
            json!(expected_wire),
            "code wire name mismatch"
        );
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
    assert_eq!(
        pe.details.get("provider").map(String::as_str),
        Some("anthropic")
    );
    let reencoded = serde_json::to_value(&pe).expect("serialize");
    assert_eq!(reencoded, fixture);
}

// ── From<EnvError> mappings ─────────────────────────────────────────────────

#[test]
fn env_error_provision_failed_maps_to_unavailable_retryable() {
    let e = EnvError::ProvisionFailed("no capacity".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[test]
fn env_error_isolation_violation_maps_to_internal() {
    let e = EnvError::IsolationViolation("sandbox breach".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Internal);
    assert!(!pe.retryable);
}

#[test]
fn env_error_credential_failed_maps_to_unavailable() {
    let e = EnvError::CredentialFailed("token expired".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Unavailable);
    assert!(pe.retryable);
}

#[test]
fn env_error_resource_exceeded_maps_to_conflict() {
    let e = EnvError::ResourceExceeded("oom".into());
    let pe: ProtocolError = e.into();
    assert_eq!(pe.code, ErrorCode::Conflict);
    assert!(!pe.retryable);
}
