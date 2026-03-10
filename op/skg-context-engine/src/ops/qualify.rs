//! ACC qualification gate — filters recalled artifacts by decision-relevance.
//!
//! `QualifyRecall` takes the current [`CognitiveState`] and a set of candidate
//! artifacts, asks the LLM which ones are relevant to the current cognitive
//! goal, and returns only the approved keys.
//!
//! # DIY-first
//!
//! Call [`QualifyRecallConfig::build_request`] to produce an [`InferRequest`]
//! you send to any provider, then [`QualifyRecallConfig::parse_response`] to
//! extract the approved keys from the response.

use crate::ops::cognitive::{CognitiveError, CognitiveState};
use crate::rules::compaction::strip_json_fences;
use layer0::content::Content;
use layer0::context::{Message, Role};
use serde::Deserialize;
use skg_turn::infer::InferRequest;

// ── RecalledArtifact ──────────────────────────────────────────────────────────

/// A candidate artifact offered for ACC qualification.
///
/// `key` is the stable identifier used to reference the artifact across the
/// system. `snippet` is the short text shown to the LLM for relevance scoring —
/// it should be concise enough to fit inside the qualification prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct RecalledArtifact {
    /// Stable, unique identifier for this artifact.
    pub key: String,
    /// Short excerpt used by the LLM to assess relevance.
    pub snippet: String,
}

impl RecalledArtifact {
    /// Create a new recalled artifact.
    pub fn new(key: impl Into<String>, snippet: impl Into<String>) -> Self {
        Self { key: key.into(), snippet: snippet.into() }
    }
}

// ── Prompt ────────────────────────────────────────────────────────────────────

/// Default system prompt for the ACC qualification gate.
pub const DEFAULT_QUALIFY_PROMPT: &str = r#"You are an ACC qualification filter. Your job is to decide which recalled artifacts are decision-relevant given the agent's current cognitive state.

You will receive:
1. The current Compressed Cognitive State (CCS) as JSON.
2. A list of candidate artifacts, each with a key and a short snippet.

Your task:
- Read the CCS to understand the agent's current goal, constraints, and context.
- For each candidate, determine if its snippet is relevant to advancing the current goal or resolving an active uncertainty.
- Approve only candidates that provide decision-relevant information.

Output ONLY a JSON object in this exact format:
{"approved": ["key1", "key2"]}

Rules:
1. Include only keys from the candidate list. Do not invent keys.
2. The approved list may be empty if no candidates are relevant.
3. Output ONLY valid JSON — no markdown fences, no explanation."#;

// ── QualifyRecallConfig ───────────────────────────────────────────────────────

/// Configuration for the ACC qualification gate.
///
/// DIY-first: call [`build_request`](Self::build_request) to get an
/// [`InferRequest`] you send to any provider, then
/// [`parse_response`](Self::parse_response) to extract approved keys.
#[derive(Debug, Clone)]
pub struct QualifyRecallConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_QUALIFY_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the qualification response.
    pub max_tokens: u32,
}

impl Default for QualifyRecallConfig {
    fn default() -> Self {
        Self { system_prompt: None, max_tokens: 512 }
    }
}

impl QualifyRecallConfig {
    /// Build an [`InferRequest`] for the qualification gate.
    ///
    /// The user message includes the current CCS as JSON and the list of
    /// candidate artifacts. The LLM is asked to return `{"approved": [...]}`.
    pub fn build_request(
        &self,
        ccs: &CognitiveState,
        candidates: &[RecalledArtifact],
    ) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_QUALIFY_PROMPT)
            .to_string();

        let mut parts: Vec<String> = Vec::new();

        parts.push("## Current Cognitive State".to_string());
        let ccs_json = serde_json::to_string_pretty(ccs).unwrap_or_else(|_| "{}".to_string());
        parts.push(ccs_json);

        parts.push(String::new());
        parts.push("## Candidate Artifacts".to_string());
        for artifact in candidates {
            parts.push(format!("key: {}", artifact.key));
            parts.push(format!("snippet: {}", artifact.snippet));
            parts.push(String::new());
        }

        let user_text = parts.join("\n");
        let user_msg = Message::new(Role::User, Content::text(user_text));

        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response into a list of approved artifact keys.
    ///
    /// Strips markdown code fences if present, then deserializes
    /// `{"approved": [...]}`. Returns [`CognitiveError::ParseFailed`] if the
    /// text cannot be parsed as a valid qualification response.
    pub fn parse_response(&self, response: &str) -> Result<Vec<String>, CognitiveError> {
        let trimmed = strip_json_fences(response);
        let parsed: QualifyResponse = serde_json::from_str(trimmed)
            .map_err(|e| CognitiveError::ParseFailed(e.to_string()))?;
        Ok(parsed.approved)
    }
}

// ── Internal deserialization type ─────────────────────────────────────────────

#[derive(Deserialize)]
struct QualifyResponse {
    approved: Vec<String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ccs() -> CognitiveState {
        CognitiveState {
            goal: "migrate production database".into(),
            episodic_trace: "User confirmed backup window".into(),
            ..Default::default()
        }
    }

    fn sample_candidates() -> Vec<RecalledArtifact> {
        vec![
            RecalledArtifact::new("art:1", "Database backup completed at 02:00 UTC"),
            RecalledArtifact::new("art:2", "Unrelated marketing report Q3"),
        ]
    }

    #[test]
    fn build_request_includes_ccs_and_candidates() {
        let config = QualifyRecallConfig::default();
        let ccs = sample_ccs();
        let candidates = sample_candidates();

        let request = config.build_request(&ccs, &candidates);

        assert!(request.system.is_some());
        let sys = request.system.as_ref().unwrap();
        assert!(sys.contains("approved"));

        let text = request.messages[0].text_content();
        assert!(text.contains("migrate production database"), "CCS goal missing");
        assert!(text.contains("art:1"), "candidate key art:1 missing");
        assert!(text.contains("Database backup completed"), "candidate snippet missing");
        assert!(text.contains("art:2"), "candidate key art:2 missing");
    }

    #[test]
    fn parse_response_handles_clean_json() {
        let config = QualifyRecallConfig::default();
        let response = r#"{"approved": ["art:1", "art:3"]}"#;
        let approved = config.parse_response(response).unwrap();
        assert_eq!(approved, vec!["art:1", "art:3"]);
    }

    #[test]
    fn parse_response_handles_fenced_json() {
        let config = QualifyRecallConfig::default();
        let response = "```json\n{\"approved\": [\"art:1\"]}\n```";
        let approved = config.parse_response(response).unwrap();
        assert_eq!(approved, vec!["art:1"]);
    }

    #[test]
    fn parse_response_handles_empty_approved_list() {
        let config = QualifyRecallConfig::default();
        let response = r#"{"approved": []}"#;
        let approved = config.parse_response(response).unwrap();
        assert!(approved.is_empty());
    }

    #[test]
    fn parse_response_rejects_invalid_json() {
        let config = QualifyRecallConfig::default();
        let result = config.parse_response("not json at all");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to parse CCS"));
    }
}
