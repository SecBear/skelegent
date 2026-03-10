//! Zettelkasten-inspired memory note type for the ACC memory store.
//!
//! [`MemoryNote`] is the atomic unit of long-term memory in the cognitive
//! architecture. Each note is self-contained: it carries its own keywords,
//! tags, and a prose description to support both exact-key lookup and
//! semantic/keyword-based recall.

use crate::ops::cognitive::CognitiveError;
use crate::rules::compaction::strip_json_fences;
use layer0::content::Content;
use layer0::context::{Message, Role};
use serde::{Deserialize, Serialize};
use skg_turn::infer::InferRequest;

// ── MemoryNote ────────────────────────────────────────────────────────────────

/// A Zettelkasten-inspired note stored in long-term memory.
///
/// Notes are written when the ACC determines that information should survive
/// beyond the current cognitive state. They are retrieved during the recall
/// phase and filtered by the qualification gate before being included in the
/// updated CCS.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MemoryNote {
    /// Stable, unique identifier for this note.
    pub key: String,
    /// Full content of the note.
    pub content: String,
    /// Keywords used for retrieval (broad, stemmed terms).
    pub keywords: Vec<String>,
    /// Categorical tags for filtering (e.g. `"infra"`, `"decision"`).
    pub tags: Vec<String>,
    /// Short human-readable description of what this note captures.
    pub description: String,
    /// The conversation turn this note was derived from, if any.
    pub source_turn: Option<u32>,
    /// Unix timestamp (seconds since epoch, fractional) when the note was created.
    pub created_at: f64,
}

impl MemoryNote {
    /// Create a minimal note with only `key` and `content`.
    ///
    /// All other fields default: empty collections, empty description,
    /// no source turn, and `created_at` = 0.0. Use the builder methods
    /// to populate additional fields.
    pub fn new(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            content: content.into(),
            ..Default::default()
        }
    }

    /// Set the keywords for retrieval.
    pub fn with_keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    /// Set the categorical tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Set the short description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the source conversation turn.
    pub fn with_source_turn(mut self, turn: u32) -> Self {
        self.source_turn = Some(turn);
        self
    }

    /// Set the creation timestamp.
    pub fn with_created_at(mut self, ts: f64) -> Self {
        self.created_at = ts;
        self
    }
}

// ── ConstructNoteConfig ──────────────────────────────────────────────────────

/// Default system prompt for constructing a MemoryNote from interaction messages.
/// A-MEM §3.1: Extract key, content, keywords, tags, description from messages.
pub const DEFAULT_CONSTRUCT_NOTE_PROMPT: &str = r#"You are a memory note extractor. Given conversation messages, extract the key information as a structured MemoryNote.

Output ONLY a JSON object matching this schema:
{
  "key": "string - unique identifier, lowercase kebab-case prefixed with 'note:'",
  "content": "string - full content of the note",
  "keywords": ["string - broad retrieval keywords"],
  "tags": ["string - categorical labels, e.g. decision, constraint, infra"],
  "description": "string - one concise sentence summary",
  "source_turn": null,
  "created_at": 0.0
}

Rules:
1. key must be lowercase, kebab-case, prefixed with 'note:'
2. content captures the essential information from the messages
3. keywords: broad, stemmed terms useful for retrieval
4. description: one concise sentence
5. Output ONLY valid JSON — no markdown fences, no explanation."#;

/// Configuration for constructing a structured MemoryNote from interaction messages.
/// A-MEM §3.1: Extract key, content, keywords, tags, description from messages.
#[derive(Debug, Clone)]
pub struct ConstructNoteConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_CONSTRUCT_NOTE_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the response.
    pub max_tokens: u32,
}

impl Default for ConstructNoteConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 1024,
        }
    }
}

impl ConstructNoteConfig {
    /// Build an [`InferRequest`] that prompts the LLM to extract a [`MemoryNote`]
    /// from the provided messages.
    pub fn build_request(&self, messages: &[Message]) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_CONSTRUCT_NOTE_PROMPT)
            .to_string();

        let mut parts: Vec<String> = Vec::new();
        parts.push("## Messages".to_string());
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

        let user_msg = Message::new(Role::User, Content::text(parts.join("\n")));
        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response into a [`MemoryNote`].
    ///
    /// Strips markdown code fences if present, then deserializes the JSON.
    /// Returns [`CognitiveError::ParseFailed`] on failure.
    pub fn parse_response(&self, response: &str) -> Result<MemoryNote, CognitiveError> {
        let trimmed = strip_json_fences(response);
        serde_json::from_str(trimmed).map_err(|e| CognitiveError::ParseFailed(e.to_string()))
    }
}

// ── NoteLink + LinkGenerationConfig ──────────────────────────────────────────

/// Proposed link between two memory notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteLink {
    /// Key of the source note. Filled in by the caller after `parse_response`.
    pub from_key: String,
    /// Key of the target note.
    pub to_key: String,
    /// Semantic relation type (e.g. "related", "contradicts", "elaborates").
    pub relation: String,
    /// Confidence in the link, in [0.0, 1.0].
    pub strength: f64,
}

/// Default system prompt for link generation.
pub const DEFAULT_LINK_GENERATION_PROMPT: &str = r#"You are a memory link generator. Given a new memory note and a list of existing notes, identify semantic links between the new note and any existing notes.

Output ONLY a JSON object in this exact format:
{"links": [{"to_key": "string", "relation": "string", "strength": 0.0}]}

Rules:
1. Only link to notes from the provided existing list — do not invent keys.
2. relation: a short label such as 'related', 'contradicts', 'elaborates', 'depends_on'.
3. strength: confidence in [0.0, 1.0].
4. The links array may be empty if no relevant connections exist.
5. Output ONLY valid JSON — no markdown fences, no explanation."#;

/// Configuration for generating links between a new note and existing notes.
#[derive(Debug, Clone)]
pub struct LinkGenerationConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_LINK_GENERATION_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the response.
    pub max_tokens: u32,
}

impl Default for LinkGenerationConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 512,
        }
    }
}

impl LinkGenerationConfig {
    /// Build an [`InferRequest`] for link generation.
    ///
    /// The user message includes the new note and existing notes as JSON.
    /// `from_key` is not set in the parsed response — the caller must fill it
    /// from `new_note.key` after calling `parse_response`.
    pub fn build_request(
        &self,
        new_note: &MemoryNote,
        existing_notes: &[MemoryNote],
    ) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_LINK_GENERATION_PROMPT)
            .to_string();

        let mut parts: Vec<String> = vec![
            "## New Note".to_string(),
            serde_json::to_string_pretty(new_note).unwrap_or_else(|_| "{}".to_string()),
            String::new(),
            "## Existing Notes".to_string(),
        ];
        for note in existing_notes {
            parts.push(serde_json::to_string_pretty(note).unwrap_or_else(|_| "{}".to_string()));
            parts.push(String::new());
        }

        let user_msg = Message::new(Role::User, Content::text(parts.join("\n")));
        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response into a list of [`NoteLink`]s.
    ///
    /// Strips fences, parses `{"links": [{"to_key", "relation", "strength"}]}`.
    /// `from_key` is left empty — the caller must set it from `new_note.key`.
    pub fn parse_response(&self, response: &str) -> Result<Vec<NoteLink>, CognitiveError> {
        #[derive(Deserialize)]
        struct LinksResponse {
            links: Vec<LinkEntry>,
        }
        #[derive(Deserialize)]
        struct LinkEntry {
            to_key: String,
            relation: String,
            strength: f64,
        }

        let trimmed = strip_json_fences(response);
        let parsed: LinksResponse = serde_json::from_str(trimmed)
            .map_err(|e| CognitiveError::ParseFailed(e.to_string()))?;
        Ok(parsed
            .links
            .into_iter()
            .map(|e| NoteLink {
                from_key: String::new(),
                to_key: e.to_key,
                relation: e.relation,
                strength: e.strength,
            })
            .collect())
    }
}

// ── EvolveMemoryConfig ───────────────────────────────────────────────────────

/// Default system prompt for evolving an existing note with new evidence.
pub const DEFAULT_EVOLVE_MEMORY_PROMPT: &str = r#"You are a memory evolution agent. Given a new memory note and an existing note, determine whether the existing note should be updated based on the new evidence.

Output ONLY a JSON object in one of these two formats:

If an update is needed:
{"updated": true, "note": {<full updated MemoryNote JSON>}}

If no update is needed:
{"updated": false}

The MemoryNote schema:
{"key": "string", "content": "string", "keywords": [], "tags": [], "description": "string", "source_turn": null, "created_at": 0.0}

Rules:
1. Preserve the original key — do not change it.
2. Update only if the new evidence materially changes the existing note's content or scope.
3. Output ONLY valid JSON — no markdown fences, no explanation."#;

/// Configuration for evolving an existing note based on new evidence.
/// A-MEM §3.3: When new evidence arrives, linked notes may need updating.
#[derive(Debug, Clone)]
pub struct EvolveMemoryConfig {
    /// Custom system prompt. If `None`, uses [`DEFAULT_EVOLVE_MEMORY_PROMPT`].
    pub system_prompt: Option<String>,
    /// Max tokens for the response.
    pub max_tokens: u32,
}

impl Default for EvolveMemoryConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_tokens: 1024,
        }
    }
}

impl EvolveMemoryConfig {
    /// Build an [`InferRequest`] that asks the LLM whether `existing_note` should
    /// be updated given `new_note`.
    pub fn build_request(&self, new_note: &MemoryNote, existing_note: &MemoryNote) -> InferRequest {
        let system = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_EVOLVE_MEMORY_PROMPT)
            .to_string();

        let parts: Vec<String> = vec![
            "## New Note".to_string(),
            serde_json::to_string_pretty(new_note).unwrap_or_else(|_| "{}".to_string()),
            String::new(),
            "## Existing Note".to_string(),
            serde_json::to_string_pretty(existing_note).unwrap_or_else(|_| "{}".to_string()),
        ];

        let user_msg = Message::new(Role::User, Content::text(parts.join("\n")));
        InferRequest::new(vec![user_msg])
            .with_system(system)
            .with_max_tokens(self.max_tokens)
    }

    /// Parse a provider response.
    ///
    /// Returns `None` if the response indicates no update is needed.
    /// Returns `Some(MemoryNote)` with the updated note if `updated: true`.
    pub fn parse_response(&self, response: &str) -> Result<Option<MemoryNote>, CognitiveError> {
        #[derive(Deserialize)]
        struct EvolveResponse {
            updated: bool,
            note: Option<MemoryNote>,
        }

        let trimmed = strip_json_fences(response);
        let parsed: EvolveResponse = serde_json::from_str(trimmed)
            .map_err(|e| CognitiveError::ParseFailed(e.to_string()))?;
        if parsed.updated {
            parsed
                .note
                .ok_or_else(|| {
                    CognitiveError::ParseFailed(
                        "updated=true but 'note' field is missing".to_string(),
                    )
                })
                .map(Some)
        } else {
            Ok(None)
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let note = MemoryNote::new("note:42", "The backup window is 02:00–04:00 UTC.")
            .with_keywords(["backup", "window", "UTC"])
            .with_tags(["infra", "constraint"])
            .with_description("Backup schedule constraint")
            .with_source_turn(7)
            .with_created_at(1_700_000_000.5);

        let json = serde_json::to_value(&note).unwrap();
        let restored: MemoryNote = serde_json::from_value(json).unwrap();
        assert_eq!(restored, note);
    }

    #[test]
    fn default_is_empty() {
        let note = MemoryNote::default();
        assert!(note.key.is_empty());
        assert!(note.content.is_empty());
        assert!(note.keywords.is_empty());
        assert!(note.tags.is_empty());
        assert!(note.description.is_empty());
        assert!(note.source_turn.is_none());
        assert_eq!(note.created_at, 0.0);
    }

    #[test]
    fn builder_pattern() {
        let note = MemoryNote::new("note:1", "content")
            .with_keywords(["kw1", "kw2"])
            .with_tags(["tag1"])
            .with_description("desc")
            .with_source_turn(3)
            .with_created_at(42.0);

        assert_eq!(note.key, "note:1");
        assert_eq!(note.content, "content");
        assert_eq!(note.keywords, vec!["kw1", "kw2"]);
        assert_eq!(note.tags, vec!["tag1"]);
        assert_eq!(note.description, "desc");
        assert_eq!(note.source_turn, Some(3));
        assert_eq!(note.created_at, 42.0);
    }

    #[test]
    fn all_fields_present_in_json() {
        let note = MemoryNote::new("note:x", "body")
            .with_keywords(["kw"])
            .with_tags(["t"])
            .with_description("d")
            .with_source_turn(1)
            .with_created_at(1.0);

        let json = serde_json::to_value(&note).unwrap();
        assert!(json.get("key").is_some());
        assert!(json.get("content").is_some());
        assert!(json.get("keywords").is_some());
        assert!(json.get("tags").is_some());
        assert!(json.get("description").is_some());
        assert!(json.get("source_turn").is_some());
        assert!(json.get("created_at").is_some());
    }
    // ── ConstructNoteConfig tests ─────────────────────────────────────────────

    #[test]
    fn construct_note_builds_request() {
        let config = ConstructNoteConfig::default();
        let messages = vec![Message::new(Role::User, Content::text("deploy to prod"))];
        let req = config.build_request(&messages);
        assert!(req.system.is_some());
        assert!(req.messages[0].text_content().contains("deploy to prod"));
    }

    #[test]
    fn construct_note_parses_response() {
        let config = ConstructNoteConfig::default();
        let json = r#"{"key":"note:test","content":"test content","keywords":["test"],"tags":["infra"],"description":"desc","source_turn":null,"created_at":0.0}"#;
        let note = config.parse_response(json).unwrap();
        assert_eq!(note.key, "note:test");
        assert_eq!(note.content, "test content");
    }

    // ── LinkGenerationConfig tests ────────────────────────────────────────────

    #[test]
    fn link_generation_builds_request_with_notes() {
        let config = LinkGenerationConfig::default();
        let new_note = MemoryNote::new("note:new", "new content");
        let existing = vec![MemoryNote::new("note:existing", "existing content")];
        let req = config.build_request(&new_note, &existing);
        assert!(req.system.is_some());
        let text = req.messages[0].text_content();
        assert!(text.contains("note:new"), "new note key missing");
        assert!(text.contains("note:existing"), "existing note key missing");
    }

    #[test]
    fn link_generation_parses_links() {
        let config = LinkGenerationConfig::default();
        let json = r#"{"links":[{"to_key":"note:b","relation":"related","strength":0.8}]}"#;
        let links = config.parse_response(json).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].to_key, "note:b");
        assert_eq!(links[0].relation, "related");
        assert!((links[0].strength - 0.8).abs() < 1e-9);
        // from_key is left empty for the caller to fill
        assert!(links[0].from_key.is_empty());
    }

    // ── EvolveMemoryConfig tests ──────────────────────────────────────────────

    #[test]
    fn evolve_memory_parses_updated_note() {
        let config = EvolveMemoryConfig::default();
        let json = r#"{"updated":true,"note":{"key":"note:x","content":"updated","keywords":[],"tags":[],"description":"","source_turn":null,"created_at":0.0}}"#;
        let result = config.parse_response(json).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().key, "note:x");
    }

    #[test]
    fn evolve_memory_parses_no_update() {
        let config = EvolveMemoryConfig::default();
        let json = r#"{"updated":false}"#;
        let result = config.parse_response(json).unwrap();
        assert!(result.is_none());
    }
}
