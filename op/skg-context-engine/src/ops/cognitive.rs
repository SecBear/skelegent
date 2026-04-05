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

use crate::rules::compaction::strip_json_fences;
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::error::StateError;
use layer0::intent::Scope;
use layer0::state::StateStore;
use serde::{Deserialize, Serialize};
use skg_turn::infer::InferRequest;

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
        Self {
            name: name.into(),
            entity_type: entity_type.into(),
        }
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
        Self {
            from: from.into(),
            to: to.into(),
            relation: relation.into(),
        }
    }
}

// ── CognitiveError ────────────────────────────────────────────────────────────

/// Errors produced by cognitive state operations.
#[derive(Debug, thiserror::Error)]
pub enum CognitiveError {
    /// The LLM response could not be parsed as a valid CCS JSON object.
    #[error("failed to parse CCS: {0}")]
    ParseFailed(String),
}

// ── CompressCognitiveStateConfig ──────────────────────────────────────────────

/// Default system prompt for CCS compression (ACC §3.1).
pub const DEFAULT_CCS_PROMPT: &str = r#"You are a cognitive state compressor. Given the conversation messages, previous cognitive state, and recalled artifacts, produce a new Compressed Cognitive State (CCS) as a JSON object.

The CCS schema:
{
  "episodic_trace": "string - what changed this turn",
  "semantic_gist": "string - dominant intent/topic",
  "focal_entities": [{"name": "string", "entity_type": "string"}],
  "relational_map": [{"from": "string", "to": "string", "relation": "string"}],
  "goal": "string - persistent objective",
  "constraints": ["string - invariant rules"],
  "predictive_cue": "string - expected next action",
  "uncertainty": ["string - unresolved elements"],
  "artifact_refs": ["string - references to external evidence"]
}

Rules:
1. REPLACE the previous state entirely — do not append.
2. Preserve all active constraints from the previous state unless explicitly revoked.
3. Update entities and relations based on new evidence.
4. Be concise — this state must remain bounded.
5. Output ONLY valid JSON, no markdown fences."#;

/// Configuration for ACC cognitive state compression.
///
/// DIY-first: call `build_request()` to get an `InferRequest` you send
/// to any provider, then `parse_response()` to extract the CCS.
#[derive(Debug, Clone)]
pub struct CompressCognitiveStateConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_CCS_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the compression response.
    pub max_tokens: u32,
}

impl Default for CompressCognitiveStateConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 4096,
        }
    }
}

impl CompressCognitiveStateConfig {
    /// Build an [`InferRequest`] for CCS compression.
    ///
    /// The system message contains the CCS JSON schema and compression rules.
    /// The user message includes the current turn messages, the previous CCS
    /// (if any), and any recalled artifact references.
    pub fn build_request(
        &self,
        messages: &[Message],
        prev_ccs: Option<&CognitiveState>,
        artifacts: &[String],
    ) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_CCS_PROMPT)
            .to_string();

        let mut parts: Vec<String> = Vec::new();

        if !messages.is_empty() {
            parts.push("## Current Turn Messages".to_string());
            for msg in messages {
                let role_str = match &msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool { name, .. } => name.as_str(),
                    _ => "other",
                };
                parts.push(format!("[{}] {}", role_str, msg.text_content()));
            }
        }

        if let Some(prev) = prev_ccs {
            parts.push(String::new());
            parts.push("## Previous Cognitive State".to_string());
            let prev_json = serde_json::to_string_pretty(prev).unwrap_or_else(|_| "{}".to_string());
            parts.push(prev_json);
        }

        if !artifacts.is_empty() {
            parts.push(String::new());
            parts.push("## Recalled Artifacts".to_string());
            for artifact in artifacts {
                parts.push(format!("- {artifact}"));
            }
        }

        let user_text = parts.join("\n");
        let user_msg = Message::new(Role::User, Content::text(user_text));

        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response into a [`CognitiveState`].
    ///
    /// Strips markdown code fences (` ```json `, ` ``` `) if present, then
    /// deserializes the JSON. Returns [`CognitiveError::ParseFailed`] if the
    /// text cannot be parsed as a valid CCS.
    pub fn parse_response(&self, response: &str) -> Result<CognitiveState, CognitiveError> {
        let trimmed = strip_json_fences(response);
        serde_json::from_str(trimmed).map_err(|e| CognitiveError::ParseFailed(e.to_string()))
    }
}

// ── CommitCognitiveState ──────────────────────────────────────────────────────

/// Writes/reads [`CognitiveState`] to a [`StateStore`].
///
/// ACC principle: state is REPLACED each turn, never accumulated.
/// `commit` unconditionally overwrites the previous state.
pub struct CommitCognitiveState;

impl CommitCognitiveState {
    /// Well-known key for the cognitive state in the store.
    pub const KEY: &'static str = "__skg_cognitive_state";

    /// Commit (write/overwrite) a [`CognitiveState`] to the store.
    ///
    /// Uses the fixed key [`CommitCognitiveState::KEY`] under `scope`.
    /// Any previously committed state is silently replaced.
    pub async fn commit(
        store: &dyn StateStore,
        scope: &Scope,
        state: &CognitiveState,
    ) -> Result<(), StateError> {
        let value =
            serde_json::to_value(state).map_err(|e| StateError::Serialization(e.to_string()))?;
        store.write(scope, Self::KEY, value).await
    }

    /// Load the current [`CognitiveState`] from the store.
    ///
    /// Returns `None` if no state has been committed yet.
    pub async fn load(
        store: &dyn StateStore,
        scope: &Scope,
    ) -> Result<Option<CognitiveState>, StateError> {
        match store.read(scope, Self::KEY).await? {
            Some(value) => {
                let state = serde_json::from_value(value)
                    .map_err(|e| StateError::Serialization(e.to_string()))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CognitiveState basic tests ────────────────────────────────────────────

    #[test]
    fn ccs_roundtrips_through_json() {
        let ccs = CognitiveState {
            episodic_trace: "User asked about deployment constraints".into(),
            semantic_gist: "Infrastructure migration planning".into(),
            focal_entities: vec![Entity::new("prod-db-01", "server")],
            relational_map: vec![Relation::new("prod-db-01", "app-cluster", "serves")],
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

    // ── CompressCognitiveStateConfig tests ────────────────────────────────────

    #[test]
    fn compress_config_builds_request_with_schema() {
        let config = CompressCognitiveStateConfig::default();
        let messages = vec![Message::new(Role::User, Content::text("Deploy to staging"))];
        let request = config.build_request(&messages, None, &[]);
        // System message should contain CCS schema
        assert!(request.system.is_some());
        let sys = request.system.as_ref().unwrap();
        assert!(sys.contains("episodic_trace"));
        assert!(sys.contains("semantic_gist"));
    }

    #[test]
    fn compress_config_includes_previous_ccs() {
        let config = CompressCognitiveStateConfig::default();
        let prev = CognitiveState {
            goal: "migrate DB".into(),
            ..Default::default()
        };
        let request = config.build_request(&[], Some(&prev), &[]);
        let text = request.messages[0].text_content();
        assert!(text.contains("migrate DB"));
    }

    #[test]
    fn compress_config_parses_valid_json() {
        let config = CompressCognitiveStateConfig::default();
        let json = serde_json::to_string(&CognitiveState {
            goal: "test".into(),
            ..Default::default()
        })
        .unwrap();
        let ccs = config.parse_response(&json).unwrap();
        assert_eq!(ccs.goal, "test");
    }

    #[test]
    fn compress_config_parses_json_with_fences() {
        let config = CompressCognitiveStateConfig::default();
        let json = format!(
            "```json\n{}\n```",
            serde_json::to_string(&CognitiveState::default()).unwrap()
        );
        let ccs = config.parse_response(&json).unwrap();
        assert!(ccs.goal.is_empty()); // default
    }

    #[test]
    fn compress_config_rejects_invalid_json() {
        let config = CompressCognitiveStateConfig::default();
        let result = config.parse_response("not json at all");
        assert!(result.is_err());
    }

    // ── CommitCognitiveState tests ────────────────────────────────────────────

    #[tokio::test]
    async fn commit_and_load_cognitive_state() {
        use skg_state_memory::MemoryStore;
        let store = MemoryStore::new();
        let scope = Scope::Global;
        let ccs = CognitiveState {
            goal: "test goal".into(),
            ..Default::default()
        };

        CommitCognitiveState::commit(&store, &scope, &ccs)
            .await
            .unwrap();
        let loaded = CommitCognitiveState::load(&store, &scope).await.unwrap();
        assert_eq!(loaded.unwrap().goal, "test goal");
    }

    #[tokio::test]
    async fn commit_replaces_previous_state() {
        use skg_state_memory::MemoryStore;
        let store = MemoryStore::new();
        let scope = Scope::Global;

        let ccs1 = CognitiveState {
            goal: "first".into(),
            ..Default::default()
        };
        let ccs2 = CognitiveState {
            goal: "second".into(),
            ..Default::default()
        };

        CommitCognitiveState::commit(&store, &scope, &ccs1)
            .await
            .unwrap();
        CommitCognitiveState::commit(&store, &scope, &ccs2)
            .await
            .unwrap();

        let loaded = CommitCognitiveState::load(&store, &scope).await.unwrap();
        assert_eq!(loaded.unwrap().goal, "second");
    }

    #[tokio::test]
    async fn load_returns_none_when_empty() {
        use skg_state_memory::MemoryStore;
        let store = MemoryStore::new();
        let scope = Scope::Global;
        let loaded = CommitCognitiveState::load(&store, &scope).await.unwrap();
        assert!(loaded.is_none());
    }
}
