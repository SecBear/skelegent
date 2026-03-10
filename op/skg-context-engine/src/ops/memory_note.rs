//! Zettelkasten-inspired memory note type for the ACC memory store.
//!
//! [`MemoryNote`] is the atomic unit of long-term memory in the cognitive
//! architecture. Each note is self-contained: it carries its own keywords,
//! tags, and a prose description to support both exact-key lookup and
//! semantic/keyword-based recall.

use serde::{Deserialize, Serialize};

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
}
