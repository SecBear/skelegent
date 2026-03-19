//! Universal content types that cross every protocol boundary.

use serde::{Deserialize, Serialize};

/// The universal content type. Crosses every boundary.
/// Intentionally simple — complex structured content uses
/// ContentBlock variants, not nested Content.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    /// Plain text content.
    Text(String),
    /// Structured content blocks.
    Blocks(Vec<ContentBlock>),
}

/// A single block of structured content.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Plain text block.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },

    /// Image content block.
    #[serde(rename = "image")]
    Image {
        /// The image source (base64 or URL).
        source: ContentSource,
        /// The MIME type of the image.
        media_type: String,
    },

    /// File content — documents, audio, video, code files.
    #[serde(rename = "file")]
    File {
        /// The file data source — inline bytes or URL reference.
        source: ContentSource,
        /// MIME type (e.g., `application/pdf`, `audio/mp3`).
        media_type: String,
        /// Optional filename hint for the recipient.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },

    /// Structured data content — JSON objects, form data, API responses.
    #[serde(rename = "data")]
    Data {
        /// Structured data payload.
        data: serde_json::Value,
        /// Optional media type hint (e.g., `application/json`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },

    /// A tool use request from the model.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique identifier for this tool use.
        id: String,
        /// Name of the tool to invoke.
        name: String,
        /// Tool input parameters.
        input: serde_json::Value,
    },

    /// Result from a tool execution.
    #[serde(rename = "tool_result")]
    ToolResult {
        /// The tool_use id this result corresponds to.
        tool_use_id: String,
        /// The result content.
        content: String,
        /// Whether the tool execution errored.
        is_error: bool,
    },

    /// A thinking/reasoning block from the model.
    #[serde(rename = "thinking")]
    Thinking {
        /// The thinking text.
        thinking: String,
        /// Opaque signature for forwarding across turns (Anthropic requirement).
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },

    /// Escape hatch for future content types.
    /// If a new modality is invented, it goes here first.
    /// When it stabilizes, it graduates to a named variant.
    #[serde(rename = "custom")]
    Custom {
        /// The custom content type identifier.
        content_type: String,
        /// Arbitrary payload.
        data: serde_json::Value,
    },
}

/// Source for binary content — images, files, or other media.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentSource {
    /// Base64-encoded binary data.
    Base64 {
        /// The base64-encoded data.
        data: String,
    },
    /// URL pointing to content.
    Url {
        /// The URL of the content.
        url: String,
    },
}

impl Content {
    /// Create a text content value.
    pub fn text(s: impl Into<String>) -> Self {
        Content::Text(s.into())
    }

    /// Extract plain text content, ignoring non-text blocks.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Content::Text(s) => Some(s),
            Content::Blocks(blocks) => {
                // Return first text block's content
                blocks.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
            }
        }
    }
}
