//! Context protocol types for operator message histories.
//!
//! This module provides the protocol-level types that describe messages,
//! metadata, and context snapshots. These are the stable interchange types
//! used across crate boundaries.
//!
//! ## Core types
//!
//! - [`Message`] — a concrete message with role, content, and metadata

use crate::content::Content;
use crate::lifecycle::CompactionPolicy;
use serde::{Deserialize, Serialize};

/// Per-message annotation attached to every message in an operator context.
///
/// All fields are public and directly settable. The [`Default`] implementation
/// uses [`CompactionPolicy::Normal`] and zeros/nones for everything else.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMeta {
    /// Compaction policy governing how this message survives context reduction.
    pub policy: CompactionPolicy,

    /// Source of the message, e.g. `"user"` or `"tool:shell"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Importance hint in the range 0.0–1.0. Higher values should survive compaction longer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,

    /// Monotonic version counter, incremented on each mutation.
    pub version: u64,
}

impl Default for MessageMeta {
    fn default() -> Self {
        Self {
            policy: CompactionPolicy::Normal,
            source: None,
            salience: None,
            version: 0,
        }
    }
}

impl MessageMeta {
    /// Create metadata with the given policy and defaults for all other fields.
    pub fn with_policy(policy: CompactionPolicy) -> Self {
        Self {
            policy,
            ..Default::default()
        }
    }

    /// Set the source.
    pub fn set_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the salience score.
    pub fn set_salience(mut self, salience: f64) -> Self {
        self.salience = Some(salience);
        self
    }
}

/// Role of a message in the context window.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// System instruction.
    System,
    /// Human message.
    User,
    /// Model response.
    Assistant,
    /// Tool/sub-operator result.
    Tool {
        /// Name of the tool/operator.
        name: String,
        /// Provider-specific call ID for correlation.
        call_id: String,
    },
}

/// A message in an operator's context window.
///
/// Concrete type — not generic. Every message has a role, content,
/// and per-message metadata (compaction policy, salience, source).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Who produced this message.
    pub role: Role,
    /// The message payload.
    pub content: Content,
    /// Per-message annotation (compaction policy, salience, source, version).
    pub meta: MessageMeta,
}

impl Message {
    /// Create a new message with default metadata.
    pub fn new(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta::default(),
        }
    }

    /// Create a message with `CompactionPolicy::Pinned`.
    pub fn pinned(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta {
                policy: CompactionPolicy::Pinned,
                ..Default::default()
            },
        }
    }

    /// Rough token estimate: chars/4 for text, 1000 for images, +4 overhead per message.
    pub fn estimated_tokens(&self) -> usize {
        use crate::content::ContentBlock;
        let content_tokens = match &self.content {
            Content::Text(s) => s.len() / 4,
            Content::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len() / 4,
                    ContentBlock::ToolUse { input, .. } => input.to_string().len() / 4,
                    ContentBlock::ToolResult { content, .. } => content.len() / 4,
                    ContentBlock::Image { .. } => 1000,
                    ContentBlock::File { .. } => 1000,
                    ContentBlock::Data { data, .. } => data.to_string().len() / 4,
                    ContentBlock::Custom { data, .. } => data.to_string().len() / 4,
                })
                .sum(),
        };
        content_tokens + 4 // per-message overhead
    }

    /// Extract all text content for similarity computation.
    pub fn text_content(&self) -> String {
        use crate::content::ContentBlock;
        match &self.content {
            Content::Text(s) => s.clone(),
            Content::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}
