//! Integration test for the full ACC (Agent Cognitive Compressor) cycle.
//!
//! Tests the data pipeline with real types and mocked LLM responses —
//! no actual LLM calls are made.

use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::effect::Scope;
use serde_json::json;
use skg_context_engine::ops::cognitive::{
    CommitCognitiveState, CompressCognitiveStateConfig, Entity, Relation,
};
use skg_context_engine::ops::qualify::{QualifyRecallConfig, RecalledArtifact};
use skg_state_memory::MemoryStore;

/// The constraint inherited by CCS_2 from CCS_1 (proves cross-turn propagation).
const PRESERVED_CONSTRAINT: &str = "No restarts during business hours";

/// Simulate a deterministic LLM response for turn 1.
fn simulated_ccs_1_json() -> String {
    let ccs = json!({
        "episodic_trace": "User asked about migrating prod-db-01 to new region",
        "semantic_gist": "Infrastructure migration planning",
        "focal_entities": [
            {"name": "prod-db-01", "entity_type": "server"},
            {"name": "app-cluster", "entity_type": "cluster"}
        ],
        "relational_map": [
            {"from": "prod-db-01", "to": "app-cluster", "relation": "serves"}
        ],
        "goal": "Migrate prod-db-01 to new region without downtime",
        "constraints": [PRESERVED_CONSTRAINT],
        "predictive_cue": "Next: verify backup completion",
        "uncertainty": ["Backup ETA unknown"],
        "artifact_refs": []
    });
    ccs.to_string()
}

/// Simulate a deterministic LLM response for the qualify gate.
fn simulated_qualify_response() -> &'static str {
    r#"{"approved": ["art:backup-log"]}"#
}

/// Simulate a deterministic LLM response for turn 2 CCS compression.
/// Preserves `PRESERVED_CONSTRAINT` from CCS_1.
fn simulated_ccs_2_json() -> String {
    let ccs = json!({
        "episodic_trace": "Backup confirmed complete; migration window approved",
        "semantic_gist": "Infrastructure migration — execution phase",
        "focal_entities": [
            {"name": "prod-db-01", "entity_type": "server"},
            {"name": "app-cluster", "entity_type": "cluster"},
            {"name": "new-region-vpc", "entity_type": "network"}
        ],
        "relational_map": [
            {"from": "prod-db-01", "to": "app-cluster", "relation": "serves"},
            {"from": "prod-db-01", "to": "new-region-vpc", "relation": "migrating_to"}
        ],
        "goal": "Migrate prod-db-01 to new region without downtime",
        "constraints": [PRESERVED_CONSTRAINT, "Backup must be verified before cutover"],
        "predictive_cue": "Next: initiate live replication to new-region-vpc",
        "uncertainty": [],
        "artifact_refs": ["art:backup-log"]
    });
    ccs.to_string()
}

#[tokio::test]
async fn acc_full_loop_integration() {
    let store = MemoryStore::new();
    let scope = Scope::Global;
    let compress_cfg = CompressCognitiveStateConfig::default();
    let qualify_cfg = QualifyRecallConfig::default();

    // ── Turn 1 ────────────────────────────────────────────────────────────────

    // Step 1: Build compression request for turn 1, no previous CCS.
    let turn1_messages = vec![Message::new(
        Role::User,
        Content::text("I need to migrate prod-db-01 to a new region."),
    )];
    let req1 = compress_cfg.build_request(&turn1_messages, None, &[]);

    // Request must carry a system prompt and one user message.
    assert!(
        req1.system.is_some(),
        "turn 1 request must have a system prompt"
    );
    assert_eq!(
        req1.messages.len(),
        1,
        "turn 1 request must have exactly one user message"
    );

    let sys1 = req1.system.as_ref().unwrap();
    assert!(
        sys1.contains("episodic_trace"),
        "system prompt must include CCS schema"
    );

    // The user message must echo back the conversation content.
    let user_text1 = req1.messages[0].text_content();
    assert!(
        user_text1.contains("prod-db-01"),
        "turn 1 request user text must include conversation content"
    );

    // Step 2 & 3: Simulate LLM response → parse into CognitiveState.
    let ccs_1 = compress_cfg
        .parse_response(&simulated_ccs_1_json())
        .expect("CCS_1 must parse from simulated JSON");

    assert_eq!(
        ccs_1.goal,
        "Migrate prod-db-01 to new region without downtime"
    );
    assert_eq!(ccs_1.constraints, vec![PRESERVED_CONSTRAINT]);
    assert_eq!(ccs_1.focal_entities.len(), 2);
    assert_eq!(ccs_1.relational_map.len(), 1);

    // Step 4: Commit CCS_1 to the store.
    CommitCognitiveState::commit(&store, &scope, &ccs_1)
        .await
        .expect("CCS_1 commit must succeed");

    // Step 5: Load CCS_1 and verify round-trip fidelity.
    let loaded_1 = CommitCognitiveState::load(&store, &scope)
        .await
        .expect("store read must succeed")
        .expect("CCS_1 must be present after commit");

    assert_eq!(loaded_1, ccs_1, "loaded CCS_1 must equal committed CCS_1");

    // ── Turn 2 ────────────────────────────────────────────────────────────────

    // Step 6: Build turn 2 compression request with CCS_1 as previous state.
    let turn2_messages = vec![Message::new(
        Role::User,
        Content::text("Backup is done. Can we start the migration?"),
    )];
    let req2 = compress_cfg.build_request(&turn2_messages, Some(&loaded_1), &[]);

    let user_text2 = req2.messages[0].text_content();
    assert!(
        user_text2.contains("Migrate prod-db-01"),
        "turn 2 request must embed previous CCS goal"
    );
    assert!(
        user_text2.contains(PRESERVED_CONSTRAINT),
        "turn 2 request must embed previous CCS constraints"
    );

    // Step 7: Build a QualifyRecall request with candidate artifacts.
    let candidates = vec![
        RecalledArtifact::new("art:backup-log", "Backup completed at 03:14 UTC, 0 errors"),
        RecalledArtifact::new("art:cost-report", "Monthly cloud spend report — Q1 2026"),
    ];
    let qualify_req = qualify_cfg.build_request(&loaded_1, &candidates);

    assert!(
        qualify_req.system.is_some(),
        "qualify request must have a system prompt"
    );
    let qualify_user_text = qualify_req.messages[0].text_content();
    assert!(
        qualify_user_text.contains("art:backup-log"),
        "qualify request must include candidate keys"
    );
    assert!(
        qualify_user_text.contains("Backup completed"),
        "qualify request must include candidate snippets"
    );
    assert!(
        qualify_user_text.contains("art:cost-report"),
        "qualify request must include second candidate"
    );

    // Step 8 & 9: Simulate qualify LLM response → parse approved artifacts.
    let approved = qualify_cfg
        .parse_response(simulated_qualify_response())
        .expect("qualify response must parse");

    assert_eq!(
        approved,
        vec!["art:backup-log"],
        "only the relevant artifact must be approved"
    );

    // Step 10: Simulate CCS_2 via approved artifact refs in the compression request.
    let req2_with_artifacts =
        compress_cfg.build_request(&turn2_messages, Some(&loaded_1), &approved);
    let user_text2_artifacts = req2_with_artifacts.messages[0].text_content();
    assert!(
        user_text2_artifacts.contains("art:backup-log"),
        "compression request must include approved artifact refs"
    );

    // Parse CCS_2 from simulated LLM response.
    let ccs_2 = compress_cfg
        .parse_response(&simulated_ccs_2_json())
        .expect("CCS_2 must parse from simulated JSON");

    assert_eq!(
        ccs_2.goal,
        "Migrate prod-db-01 to new region without downtime"
    );
    assert_eq!(ccs_2.artifact_refs, vec!["art:backup-log"]);
    assert_eq!(
        ccs_2.focal_entities.len(),
        3,
        "CCS_2 must add new-region-vpc entity"
    );

    // Step 11: Commit CCS_2 — must overwrite CCS_1.
    CommitCognitiveState::commit(&store, &scope, &ccs_2)
        .await
        .expect("CCS_2 commit must succeed");

    let loaded_2 = CommitCognitiveState::load(&store, &scope)
        .await
        .expect("store read must succeed")
        .expect("CCS_2 must be present after commit");

    assert_eq!(loaded_2, ccs_2, "loaded CCS_2 must equal committed CCS_2");
    assert_ne!(
        loaded_2.episodic_trace, ccs_1.episodic_trace,
        "CCS_2 must replace CCS_1"
    );

    // Step 12: Verify CCS_2 preserved the constraint from turn 1.
    assert!(
        loaded_2
            .constraints
            .contains(&PRESERVED_CONSTRAINT.to_string()),
        "CCS_2 must carry forward the constraint '{PRESERVED_CONSTRAINT}' from turn 1"
    );
    // Also picked up a new constraint in turn 2.
    assert!(
        loaded_2
            .constraints
            .contains(&"Backup must be verified before cutover".to_string()),
        "CCS_2 must add the turn-2 constraint"
    );

    // Structural sanity: entities and relations in CCS_2 are proper types.
    assert!(
        loaded_2
            .focal_entities
            .contains(&Entity::new("prod-db-01", "server"))
    );
    assert!(
        loaded_2
            .focal_entities
            .contains(&Entity::new("new-region-vpc", "network"))
    );
    assert!(loaded_2.relational_map.contains(&Relation::new(
        "prod-db-01",
        "new-region-vpc",
        "migrating_to"
    )));
}
