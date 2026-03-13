//! Procedural memory ops for the ReMe pattern.
//!
//! Distill → Recall → Refine lifecycle for reusable procedures
//! extracted from successful tool sequences.
//!
//! # Source
//! Cao et al. (2024). "ReMe: Towards Dynamic Procedural Memory."
//! arXiv:2512.10696

use crate::ops::cognitive::CognitiveError;
use crate::rules::compaction::strip_json_fences;
use layer0::content::Content;
use layer0::context::{Message, Role};
use serde::{Deserialize, Serialize};
use skg_turn::infer::InferRequest;

// ── ProcedureStep ─────────────────────────────────────────────────────────────

/// A single step in a procedure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcedureStep {
    /// Action name (e.g., tool name).
    pub action: String,
    /// Description of what this step does.
    pub description: String,
    /// Expected input pattern.
    pub input_pattern: Option<String>,
}

impl ProcedureStep {
    /// Create a new step with the given action and description, no input pattern.
    pub fn new(action: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            description: description.into(),
            input_pattern: None,
        }
    }
}

// ── Procedure ─────────────────────────────────────────────────────────────────

/// A reusable procedure distilled from successful tool sequences.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Procedure {
    /// Stable unique identifier, e.g. `"p:deploy-to-staging"`.
    pub key: String,
    /// Human-readable description of what this procedure accomplishes.
    pub description: String,
    /// Ordered steps that make up the procedure.
    pub steps: Vec<ProcedureStep>,
    /// Number of times this procedure was used successfully.
    pub success_count: u32,
    /// Number of times this procedure failed.
    pub failure_count: u32,
    /// Keywords used for retrieval.
    pub keywords: Vec<String>,
    /// Unix timestamp (fractional seconds) of the most recent use.
    pub last_used: Option<f64>,
}

impl Procedure {
    /// Create a new procedure with the given key and description.
    pub fn new(key: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            description: description.into(),
            ..Default::default()
        }
    }

    /// Set the steps for this procedure.
    pub fn with_steps(mut self, steps: Vec<ProcedureStep>) -> Self {
        self.steps = steps;
        self
    }

    /// Set the keywords for this procedure.
    pub fn with_keywords(mut self, keywords: Vec<String>) -> Self {
        self.keywords = keywords;
        self
    }

    /// Record a successful use at the given timestamp.
    pub fn record_success(&mut self, timestamp: f64) {
        self.success_count += 1;
        self.last_used = Some(timestamp);
    }

    /// Record a failed use.
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
    }

    /// Utility score in [0, 1): `success / (success + failure + 1)`.
    ///
    /// The `+ 1` in the denominator avoids division by zero for a new procedure
    /// and applies a Laplace-style smoothing that penalises untested procedures.
    pub fn utility_score(&self) -> f64 {
        self.success_count as f64 / (self.success_count + self.failure_count + 1) as f64
    }
}

// ── DistillProcedureConfig (Task 5.1) ─────────────────────────────────────────

/// Default system prompt for distilling a procedure from a tool trace.
pub const DEFAULT_DISTILL_PROCEDURE_PROMPT: &str = r#"You are a procedural memory distiller. Given a sequence of tool-use messages representing a completed task, extract a reusable procedure.

Output ONLY a JSON object matching this schema:
{
  "key": "string - unique id, lowercase kebab-case prefixed with 'p:'",
  "description": "string - what the procedure accomplishes",
  "steps": [{"action": "string", "description": "string", "input_pattern": null}],
  "success_count": 0,
  "failure_count": 0,
  "keywords": ["string - retrieval keywords"],
  "last_used": null
}

Rules:
1. key must be lowercase, kebab-case, prefixed with 'p:'
2. steps must be in execution order
3. Extract only the essential, repeatable steps — omit one-off decisions
4. keywords: terms that describe when this procedure would be useful
5. Output ONLY valid JSON — no markdown fences, no explanation."#;

/// Configuration for distilling a procedure from a tool trace.
#[derive(Debug, Clone)]
pub struct DistillProcedureConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_DISTILL_PROCEDURE_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the response.
    pub max_tokens: u32,
}

impl Default for DistillProcedureConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 2048,
        }
    }
}

impl DistillProcedureConfig {
    /// Build an [`InferRequest`] that prompts the LLM to extract a [`Procedure`]
    /// from the provided tool trace messages.
    pub fn build_request(&self, tool_trace: &[Message]) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_DISTILL_PROCEDURE_PROMPT)
            .to_string();

        let mut parts: Vec<String> = Vec::new();
        parts.push("## Tool Trace".to_string());
        for msg in tool_trace {
            let role_str = match &msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool { name, .. } => name.as_str(),
                _ => "other",
            };
            parts.push(format!("[{}] {}", role_str, msg.text_content()));
        }

        let user_msg = Message::new(Role::User, Content::text(parts.join("\n")));
        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response into a [`Procedure`].
    ///
    /// Strips markdown code fences if present, then deserializes the JSON.
    pub fn parse_response(&self, response: &str) -> Result<Procedure, CognitiveError> {
        let trimmed = strip_json_fences(response);
        serde_json::from_str(trimmed).map_err(|e| CognitiveError::ParseFailed(e.to_string()))
    }
}

// ── RecallProcedureConfig (Task 5.2) ──────────────────────────────────────────

/// Configuration for recalling relevant procedures given the current context.
#[derive(Debug, Clone)]
pub struct RecallProcedureConfig {
    /// Custom system prompt (reserved for future LLM-based query extraction).
    pub system_prompt: Option<String>,
    /// Max tokens (reserved for future LLM-based query extraction).
    pub max_tokens: u32,
}

impl Default for RecallProcedureConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 256,
        }
    }
}

impl RecallProcedureConfig {
    /// Extract a search query string from the current context messages.
    ///
    /// Returns the text content of the most recent user message, which is the
    /// most task-relevant signal for procedure lookup. Returns an empty string
    /// if no user message is present.
    pub fn build_query(&self, messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, Role::User))
            .map(|m| m.text_content())
            .unwrap_or_default()
    }

    /// Format a list of procedures as an assistant guidance message.
    ///
    /// The returned message can be injected before the next user turn to give
    /// the agent awareness of relevant known procedures.
    pub fn format_guidance(&self, procedures: &[Procedure]) -> Message {
        let mut parts: Vec<String> = Vec::new();
        parts.push("## Relevant Procedures".to_string());
        for proc in procedures {
            parts.push(String::new());
            parts.push(format!("### {} — {}", proc.key, proc.description));
            if !proc.keywords.is_empty() {
                parts.push(format!("Keywords: {}", proc.keywords.join(", ")));
            }
            parts.push(format!(
                "Utility: {:.2} ({} successes, {} failures)",
                proc.utility_score(),
                proc.success_count,
                proc.failure_count
            ));
            if !proc.steps.is_empty() {
                parts.push("Steps:".to_string());
                for (i, step) in proc.steps.iter().enumerate() {
                    parts.push(format!(
                        "  {}. [{}] {}",
                        i + 1,
                        step.action,
                        step.description
                    ));
                }
            }
        }
        Message::new(Role::Assistant, Content::text(parts.join("\n")))
    }
}

// ── RefineProcedureConfig (Task 5.3) ──────────────────────────────────────────

/// Minimum total uses before a procedure is eligible for pruning.
const MIN_PRUNE_USAGE_THRESHOLD: u32 = 3;

/// Default system prompt for merging two similar procedures.
const DEFAULT_MERGE_PROCEDURE_PROMPT: &str = r#"You are a procedure consolidator. Given two similar procedures, merge them into a single, more general procedure that captures the best of both.

Output ONLY a JSON object matching this schema:
{
  "key": "string - unique id, lowercase kebab-case prefixed with 'p:'",
  "description": "string - what the merged procedure accomplishes",
  "steps": [{"action": "string", "description": "string", "input_pattern": null}],
  "success_count": 0,
  "failure_count": 0,
  "keywords": ["string - retrieval keywords"],
  "last_used": null
}

Rules:
1. Preserve the most useful steps from both procedures
2. Generalise where the two differ; keep specifics only if both share them
3. Combine keywords from both
4. Output ONLY valid JSON — no markdown fences, no explanation."#;

/// Configuration for refining the procedure store: pruning low-utility entries
/// and merging similar ones.
#[derive(Debug, Clone)]
pub struct RefineProcedureConfig {
    /// Procedures with `utility_score` below this threshold are candidates for
    /// pruning (once they have sufficient usage history).
    pub min_utility: f64,
    /// Similarity threshold above which two procedures are candidates for merging.
    /// (Caller is responsible for computing similarity; this value is exposed for
    /// comparison logic outside this crate.)
    pub merge_threshold: f64,
}

impl Default for RefineProcedureConfig {
    fn default() -> Self {
        Self {
            min_utility: 0.2,
            merge_threshold: 0.85,
        }
    }
}

impl RefineProcedureConfig {
    /// Returns `true` if the procedure should be pruned.
    ///
    /// A procedure is prunable when:
    /// - Its `utility_score` is below `min_utility`, AND
    /// - It has been used at least [`MIN_PRUNE_USAGE_THRESHOLD`] times total,
    ///   ensuring we don't discard untested procedures prematurely.
    pub fn should_prune(&self, procedure: &Procedure) -> bool {
        let total = procedure.success_count + procedure.failure_count;
        procedure.utility_score() < self.min_utility && total > MIN_PRUNE_USAGE_THRESHOLD
    }

    /// Build an [`InferRequest`] that prompts the LLM to merge two similar procedures.
    pub fn build_merge_request(&self, a: &Procedure, b: &Procedure) -> InferRequest {
        let parts: Vec<String> = vec![
            "## Procedure A".to_string(),
            serde_json::to_string_pretty(a).unwrap_or_else(|_| "{}".to_string()),
            String::new(),
            "## Procedure B".to_string(),
            serde_json::to_string_pretty(b).unwrap_or_else(|_| "{}".to_string()),
        ];

        let user_msg = Message::new(Role::User, Content::text(parts.join("\n")));
        InferRequest::new(vec![user_msg])
            .with_system(DEFAULT_MERGE_PROCEDURE_PROMPT.to_string())
            .with_max_tokens(2048)
    }

    /// Parse a provider response into a merged [`Procedure`].
    pub fn parse_merge_response(&self, response: &str) -> Result<Procedure, CognitiveError> {
        let trimmed = strip_json_fences(response);
        serde_json::from_str(trimmed).map_err(|e| CognitiveError::ParseFailed(e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Procedure tests ────────────────────────────────────────────────────────

    #[test]
    fn procedure_utility_score() {
        let mut p = Procedure::new("p:test", "test");
        p.success_count = 3;
        p.failure_count = 1;
        // 3 / (3 + 1 + 1) = 3/5 = 0.6
        assert!((p.utility_score() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn procedure_utility_score_zero_uses() {
        let p = Procedure::new("p:new", "brand new");
        // 0 / (0 + 0 + 1) = 0.0
        assert_eq!(p.utility_score(), 0.0);
    }

    #[test]
    fn procedure_record_success_failure() {
        let mut p = Procedure::new("p:test", "test");
        p.record_success(1000.0);
        p.record_success(2000.0);
        p.record_failure();
        assert_eq!(p.success_count, 2);
        assert_eq!(p.failure_count, 1);
        assert_eq!(p.last_used, Some(2000.0));
    }

    #[test]
    fn procedure_roundtrip_json() {
        let p = Procedure::new("p:x", "desc")
            .with_steps(vec![ProcedureStep::new("action", "does something")])
            .with_keywords(vec!["kw".to_string()]);
        let json = serde_json::to_value(&p).unwrap();
        let restored: Procedure = serde_json::from_value(json).unwrap();
        assert_eq!(restored, p);
    }

    // ── DistillProcedureConfig tests ───────────────────────────────────────────

    #[test]
    fn distill_builds_request() {
        let config = DistillProcedureConfig::default();
        let trace = vec![Message::new(Role::User, Content::text("run tool X"))];
        let req = config.build_request(&trace);
        assert!(req.system.is_some());
        assert!(req.messages[0].text_content().contains("run tool X"));
    }

    #[test]
    fn distill_parses_response() {
        let config = DistillProcedureConfig::default();
        let json = r#"{"key":"p:deploy","description":"Deploy procedure","steps":[{"action":"build","description":"build artifact","input_pattern":null}],"success_count":0,"failure_count":0,"keywords":["deploy"],"last_used":null}"#;
        let p = config.parse_response(json).unwrap();
        assert_eq!(p.key, "p:deploy");
        assert_eq!(p.description, "Deploy procedure");
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.steps[0].action, "build");
    }

    #[test]
    fn distill_parses_fenced_response() {
        let config = DistillProcedureConfig::default();
        let json = "```json\n{\"key\":\"p:x\",\"description\":\"d\",\"steps\":[],\"success_count\":0,\"failure_count\":0,\"keywords\":[],\"last_used\":null}\n```";
        let p = config.parse_response(json).unwrap();
        assert_eq!(p.key, "p:x");
    }

    // ── RecallProcedureConfig tests ────────────────────────────────────────────

    #[test]
    fn recall_formats_guidance() {
        let config = RecallProcedureConfig::default();
        let p = Procedure::new("p:test", "test procedure")
            .with_steps(vec![ProcedureStep::new("step1", "does step1")]);
        let msg = config.format_guidance(&[p]);
        let text = msg.text_content();
        assert!(text.contains("p:test"), "key missing from guidance");
        assert!(
            text.contains("test procedure"),
            "description missing from guidance"
        );
        assert!(text.contains("step1"), "step action missing from guidance");
    }

    #[test]
    fn recall_formats_empty_guidance() {
        let config = RecallProcedureConfig::default();
        let msg = config.format_guidance(&[]);
        assert!(msg.text_content().contains("Relevant Procedures"));
    }

    #[test]
    fn recall_build_query_returns_last_user_message() {
        let config = RecallProcedureConfig::default();
        let messages = vec![
            Message::new(Role::User, Content::text("first")),
            Message::new(Role::Assistant, Content::text("response")),
            Message::new(Role::User, Content::text("deploy to staging")),
        ];
        let query = config.build_query(&messages);
        assert_eq!(query, "deploy to staging");
    }

    // ── RefineProcedureConfig tests ────────────────────────────────────────────

    #[test]
    fn refine_should_prune() {
        let config = RefineProcedureConfig {
            min_utility: 0.5,
            merge_threshold: 0.8,
        };
        let mut p = Procedure::new("p:old", "old procedure");
        p.success_count = 1;
        p.failure_count = 10;
        // utility = 1/(1+10+1) = 1/12 ≈ 0.083 < 0.5, total = 11 > MIN_PRUNE_USAGE_THRESHOLD(3)
        assert!(config.should_prune(&p));
    }

    #[test]
    fn refine_should_not_prune_low_usage() {
        let config = RefineProcedureConfig {
            min_utility: 0.5,
            merge_threshold: 0.8,
        };
        let p = Procedure::new("p:new", "new procedure");
        // success=0, failure=0, utility=0.0 < 0.5, but total uses = 0 <= threshold
        assert!(!config.should_prune(&p));
    }

    #[test]
    fn refine_should_not_prune_high_utility() {
        let config = RefineProcedureConfig {
            min_utility: 0.5,
            merge_threshold: 0.8,
        };
        let mut p = Procedure::new("p:good", "good procedure");
        p.success_count = 9;
        p.failure_count = 1;
        // utility = 9/11 ≈ 0.818 > 0.5, total = 10 > threshold
        assert!(!config.should_prune(&p));
    }

    #[test]
    fn refine_build_merge_request_includes_both_procedures() {
        let config = RefineProcedureConfig::default();
        let a = Procedure::new("p:a", "proc a");
        let b = Procedure::new("p:b", "proc b");
        let req = config.build_merge_request(&a, &b);
        assert!(req.system.is_some());
        let text = req.messages[0].text_content();
        assert!(text.contains("p:a"), "proc a missing");
        assert!(text.contains("p:b"), "proc b missing");
    }
}
