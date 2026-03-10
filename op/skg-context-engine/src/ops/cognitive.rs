//! Compressed Cognitive State (CCS) types for the ACC memory pattern.
//!
//! The Agent Cognitive Compressor (ACC) maintains a bounded internal state
//! that replaces transcript replay. CCS is updated once per turn via
//! controlled replacement — state never accumulates.
//!
//! # Source
//!
//! Bousetouane, F. (2026). "AI Agents Need Memory Control Over More Context."
//! <https://doi.org/10.32388/MZQB3T>

use serde::{Deserialize, Serialize};

/// Compressed Cognitive State (CCS) — the bounded internal representation
/// maintained by ACC across interaction turns.
///
/// CCS is neither a transcript summary nor a memory cache. It is a
/// control-oriented cognitive state designed to preserve decision-critical
/// information while discarding irrelevant detail.
///
/// Each field corresponds to a functional element of human cognition:
/// - `episodic_trace` → short-term event updating
/// - `semantic_gist` → meaning abstraction beyond surface form
/// - `focal_entities` → stable reference resolution
/// - `relational_map` → causal/temporal reasoning
/// - `goal` → goal maintenance in executive control
/// - `constraints` → rule-based inhibition
/// - `predictive_cue` → anticipatory planning
/// - `uncertainty` → preventing overconfident inference
/// - `artifact_refs` → external evidence references (not internalized)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CognitiveState {
    /// What changed in the current turn (episodic update).
    pub episodic_trace: String,
    /// Dominant intent or topic (semantic abstraction).
    pub semantic_gist: String,
    /// Canonicalized entities with types.
    pub focal_entities: Vec<Entity>,
    /// Causal and temporal dependencies between entities.
    pub relational_map: Vec<Relation>,
    /// Persistent objective guiding the interaction.
    pub goal: String,
    /// Invariant rules, policies, or safety constraints.
    pub constraints: Vec<String>,
    /// Expected next cognitive operation.
    pub predictive_cue: String,
    /// Unresolved or low-confidence elements.
    pub uncertainty: Vec<String>,
    /// References to external evidence (not internalized into state).
    pub artifact_refs: Vec<String>,
}

/// A canonicalized entity tracked in the cognitive state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    /// Canonical name of the entity.
    pub name: String,
    /// Application-defined type (e.g. "server", "person", "concept").
    pub entity_type: String,
}

impl Entity {
    /// Create a new entity.
    pub fn new(name: impl Into<String>, entity_type: impl Into<String>) -> Self {
        Self { name: name.into(), entity_type: entity_type.into() }
    }
}

/// A directed relation between two entities in the cognitive state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    /// Source entity name.
    pub from: String,
    /// Target entity name.
    pub to: String,
    /// Relation type (e.g. "serves", "depends_on", "supersedes").
    pub relation: String,
}

impl Relation {
    /// Create a new relation.
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        relation: impl Into<String>,
    ) -> Self {
        Self { from: from.into(), to: to.into(), relation: relation.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ccs_roundtrips_through_json() {
        let ccs = CognitiveState {
            episodic_trace: "User asked about deployment constraints".into(),
            semantic_gist: "Infrastructure migration planning".into(),
            focal_entities: vec![
                Entity::new("prod-db-01", "server"),
            ],
            relational_map: vec![
                Relation::new("prod-db-01", "app-cluster", "serves"),
            ],
            goal: "Migrate to new region without downtime".into(),
            constraints: vec!["No restarts during business hours".into()],
            predictive_cue: "Next: verify backup completion".into(),
            uncertainty: vec!["Backup ETA unknown".into()],
            artifact_refs: vec!["turn:3:tool:check_backup".into()],
        };
        let json = serde_json::to_value(&ccs).unwrap();
        let restored: CognitiveState = serde_json::from_value(json).unwrap();
        assert_eq!(restored, ccs);
    }

    #[test]
    fn default_ccs_is_empty() {
        let ccs = CognitiveState::default();
        assert!(ccs.episodic_trace.is_empty());
        assert!(ccs.focal_entities.is_empty());
        assert!(ccs.constraints.is_empty());
        assert!(ccs.goal.is_empty());
    }

    #[test]
    fn ccs_json_has_expected_fields() {
        let ccs = CognitiveState {
            goal: "test goal".into(),
            ..Default::default()
        };
        let json = serde_json::to_value(&ccs).unwrap();
        assert!(json.get("episodic_trace").is_some());
        assert!(json.get("semantic_gist").is_some());
        assert!(json.get("focal_entities").is_some());
        assert!(json.get("relational_map").is_some());
        assert!(json.get("goal").is_some());
        assert!(json.get("constraints").is_some());
        assert!(json.get("predictive_cue").is_some());
        assert!(json.get("uncertainty").is_some());
        assert!(json.get("artifact_refs").is_some());
        assert_eq!(json["goal"], "test goal");
    }

    #[test]
    fn entity_constructor_works() {
        let e = Entity::new("server-1", "machine");
        assert_eq!(e.name, "server-1");
        assert_eq!(e.entity_type, "machine");
    }

    #[test]
    fn relation_constructor_works() {
        let r = Relation::new("A", "B", "depends_on");
        assert_eq!(r.from, "A");
        assert_eq!(r.to, "B");
        assert_eq!(r.relation, "depends_on");
    }
}
