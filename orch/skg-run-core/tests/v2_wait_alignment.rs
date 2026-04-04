use serde_json::json;

// ── WaitReason and ResumeInput are re-exports of layer0 types ───────────────

#[test]
fn wait_reason_is_layer0_reexport() {
    // skg_run_core::WaitReason and layer0::WaitReason must be the same type.
    let layer0_val = layer0::WaitReason::Approval;
    let run_core_val: skg_run_core::WaitReason = layer0_val.clone();
    assert_eq!(run_core_val, layer0::WaitReason::Approval);
}

#[test]
fn resume_input_is_layer0_reexport() {
    // skg_run_core::ResumeInput and layer0::ResumeInput must be the same type.
    let layer0_val = layer0::ResumeInput::new(json!("approved"));
    let run_core_val: skg_run_core::ResumeInput = layer0_val.clone();
    assert_eq!(run_core_val, layer0::ResumeInput::new(json!("approved")));
}

// ── Waiting run view round-trips with shared wait wire format ───────────────

#[test]
fn waiting_run_view_round_trip_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/run-view-waiting.json"))
            .expect("fixture json");
    let view: skg_run_core::RunView =
        serde_json::from_value(fixture.clone()).expect("deserialize");

    // Verify the structural decode.
    assert_eq!(view.run_id().as_str(), "run-42");
    assert_eq!(view.status(), skg_run_core::RunStatus::Waiting);
    assert_eq!(
        view.wait_reason(),
        Some(&skg_run_core::WaitReason::Approval)
    );

    // Round-trip: re-serialize must match fixture.
    let reencoded = serde_json::to_value(&view).expect("serialize");
    assert_eq!(reencoded, fixture);
}

#[test]
fn waiting_run_view_uses_layer0_wait_reason_wire_format() {
    let view = skg_run_core::RunView::waiting(
        skg_run_core::RunId::new("run-99"),
        skg_run_core::WaitPointId::new("wp-x"),
        layer0::WaitReason::ExternalInput,
    );
    let json = serde_json::to_value(&view).expect("serialize");
    assert_eq!(json["wait_reason"], json!("external_input"));
    assert_eq!(json["status"], json!("waiting"));
}
