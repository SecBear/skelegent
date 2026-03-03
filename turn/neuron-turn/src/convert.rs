//! Bidirectional conversion between layer0 types and internal types.

use crate::types::{ContentPart, ImageSource, ProviderMessage, Role};
use layer0::content::{Content, ContentBlock};

/// Convert a layer0 `ContentBlock` to an internal `ContentPart`.
pub fn content_block_to_part(block: &ContentBlock) -> ContentPart {
    match block {
        ContentBlock::Text { text } => ContentPart::Text { text: text.clone() },
        ContentBlock::Image { source, media_type } => ContentPart::Image {
            source: image_source_to_internal(source),
            media_type: media_type.clone(),
        },
        ContentBlock::ToolUse { id, name, input } => ContentPart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentPart::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ContentBlock::Custom { content_type, data } => {
            // Design decision: Custom blocks are JSON-stringified with a type prefix
            ContentPart::Text {
                text: format!(
                    "[custom:{}] {}",
                    content_type,
                    serde_json::to_string(data).unwrap_or_default()
                ),
            }
        }
        // Handle non_exhaustive future variants
        _ => ContentPart::Text {
            text: "[unknown content block]".into(),
        },
    }
}

/// Convert an internal `ContentPart` to a layer0 `ContentBlock`.
pub fn content_part_to_block(part: &ContentPart) -> ContentBlock {
    match part {
        ContentPart::Text { text } => ContentBlock::Text { text: text.clone() },
        ContentPart::Image { source, media_type } => ContentBlock::Image {
            source: image_source_to_layer0(source),
            media_type: media_type.clone(),
        },
        ContentPart::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
    }
}

/// Convert layer0 `Content` to a list of internal `ContentPart`s.
pub fn content_to_parts(content: &Content) -> Vec<ContentPart> {
    match content {
        Content::Text(text) => vec![ContentPart::Text { text: text.clone() }],
        Content::Blocks(blocks) => blocks.iter().map(content_block_to_part).collect(),
        // Handle non_exhaustive
        _ => vec![ContentPart::Text {
            text: "[unknown content]".into(),
        }],
    }
}

/// Convert internal `ContentPart`s to a layer0 `Content`.
pub fn parts_to_content(parts: &[ContentPart]) -> Content {
    if parts.len() == 1
        && let ContentPart::Text { text } = &parts[0]
    {
        return Content::Text(text.clone());
    }
    Content::Blocks(parts.iter().map(content_part_to_block).collect())
}

/// Convert layer0 `Content` to an internal `ProviderMessage` with User role.
pub fn content_to_user_message(content: &Content) -> ProviderMessage {
    ProviderMessage {
        role: Role::User,
        content: content_to_parts(content),
    }
}

fn image_source_to_internal(source: &layer0::content::ImageSource) -> ImageSource {
    match source {
        layer0::content::ImageSource::Base64 { data } => ImageSource::Base64 { data: data.clone() },
        layer0::content::ImageSource::Url { url } => ImageSource::Url { url: url.clone() },
        // Handle non_exhaustive
        _ => ImageSource::Url { url: String::new() },
    }
}

fn image_source_to_layer0(source: &ImageSource) -> layer0::content::ImageSource {
    match source {
        ImageSource::Base64 { data } => layer0::content::ImageSource::Base64 { data: data.clone() },
        ImageSource::Url { url } => layer0::content::ImageSource::Url { url: url.clone() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let part = content_block_to_part(&block);
        let back = content_part_to_block(&part);
        assert_eq!(block, back);
    }

    #[test]
    fn tool_use_roundtrip() {
        let block = ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "bash".into(),
            input: json!({"cmd": "ls"}),
        };
        let part = content_block_to_part(&block);
        let back = content_part_to_block(&part);
        assert_eq!(block, back);
    }

    #[test]
    fn tool_result_roundtrip() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "output".into(),
            is_error: false,
        };
        let part = content_block_to_part(&block);
        let back = content_part_to_block(&part);
        assert_eq!(block, back);
    }

    #[test]
    fn image_roundtrip() {
        let block = ContentBlock::Image {
            source: layer0::content::ImageSource::Url {
                url: "https://example.com/img.png".into(),
            },
            media_type: "image/png".into(),
        };
        let part = content_block_to_part(&block);
        let back = content_part_to_block(&part);
        assert_eq!(block, back);
    }

    #[test]
    fn custom_block_becomes_text() {
        let block = ContentBlock::Custom {
            content_type: "thinking".into(),
            data: json!({"thought": "hmm"}),
        };
        let part = content_block_to_part(&block);
        match &part {
            ContentPart::Text { text } => {
                assert!(text.contains("[custom:thinking]"));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn content_text_to_parts() {
        let content = Content::text("hello");
        let parts = content_to_parts(&content);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            ContentPart::Text {
                text: "hello".into()
            }
        );
    }

    #[test]
    fn parts_to_content_single_text() {
        let parts = vec![ContentPart::Text {
            text: "hello".into(),
        }];
        let content = parts_to_content(&parts);
        assert_eq!(content, Content::text("hello"));
    }

    #[test]
    fn parts_to_content_multiple_blocks() {
        let parts = vec![
            ContentPart::Text {
                text: "hello".into(),
            },
            ContentPart::Text {
                text: "world".into(),
            },
        ];
        let content = parts_to_content(&parts);
        match content {
            Content::Blocks(blocks) => assert_eq!(blocks.len(), 2),
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn content_to_user_message_builds_correctly() {
        let content = Content::text("hi");
        let msg = content_to_user_message(&content);
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
    }
}
