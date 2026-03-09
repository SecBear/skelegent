//! Convert layer0 messages to Responses API input items.

use layer0::content::{Content, ContentBlock};
use layer0::context::{Message, Role};
use neuron_turn::types::ToolSchema;
use serde_json::json;

use crate::types::CodexTool;

/// Convert a system prompt + layer0 messages into Responses API input items.
///
/// The Responses API uses a flat list of typed input items rather than
/// a message array. Each item has a `type` field that determines its shape.
pub fn messages_to_input(messages: &[Message]) -> Vec<serde_json::Value> {
    let mut input = Vec::new();

    for msg in messages {
        match &msg.role {
            Role::System => {
                // System messages become developer input items.
                let text = content_to_text(&msg.content);
                if !text.is_empty() {
                    input.push(json!({
                        "role": "developer",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
            Role::User => {
                convert_user_message(&msg.content, &mut input);
            }
            Role::Assistant => {
                convert_assistant_message(&msg.content, &mut input);
            }
            Role::Tool { call_id, .. } => {
                let text = content_to_text(&msg.content);
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": text
                }));
            }
            _ => {
                // Future role variants — treat as user.
                let text = content_to_text(&msg.content);
                if !text.is_empty() {
                    input.push(json!({
                        "role": "user",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
        }
    }

    input
}

/// Convert tool schemas to Responses API tool definitions.
pub fn tools_to_codex(tools: &[ToolSchema]) -> Vec<CodexTool> {
    tools
        .iter()
        .map(|t| CodexTool {
            tool_type: "function".into(),
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.input_schema.clone(),
        })
        .collect()
}

fn convert_user_message(content: &Content, input: &mut Vec<serde_json::Value>) {
    match content {
        Content::Text(text) => {
            if !text.is_empty() {
                input.push(json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}]
                }));
            }
        }
        Content::Blocks(blocks) => {
            let mut tool_results = Vec::new();
            let mut content_parts = Vec::new();

            for block in blocks {
                match block {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        tool_results.push((tool_use_id.clone(), content.clone()));
                    }
                    ContentBlock::Text { text } => {
                        if !text.is_empty() {
                            content_parts.push(json!({"type": "input_text", "text": text}));
                        }
                    }
                    ContentBlock::Image { source, media_type } => {
                        if let Some(url) = image_to_url(source, media_type) {
                            content_parts.push(json!({
                                "type": "input_image",
                                "image_url": url,
                                "detail": "auto"
                            }));
                        }
                    }
                    _ => {}
                }
            }

            // Emit tool results as function_call_output items.
            for (call_id, output) in tool_results {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output
                }));
            }

            // Emit remaining content as user message.
            if !content_parts.is_empty() {
                input.push(json!({
                    "role": "user",
                    "content": content_parts
                }));
            }
        }
        _ => {
            let text = content_to_text(content);
            if !text.is_empty() {
                input.push(json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}]
                }));
            }
        }
    }
}

fn convert_assistant_message(content: &Content, input: &mut Vec<serde_json::Value>) {
    match content {
        Content::Text(text) => {
            if !text.is_empty() {
                input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text, "annotations": []}],
                    "status": "completed"
                }));
            }
        }
        Content::Blocks(blocks) => {
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();

            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(text.clone());
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        // Responses API uses call_id + item id.
                        // We encode both in the id field as "call_id|item_id".
                        let (call_id, item_id) = split_tool_id(id);
                        tool_calls.push(json!({
                            "type": "function_call",
                            "id": item_id,
                            "call_id": call_id,
                            "name": name,
                            "arguments": serde_json::to_string(input).unwrap_or_default()
                        }));
                    }
                    _ => {}
                }
            }

            // Emit text as a message item.
            let combined = text_parts.join("");
            if !combined.is_empty() {
                input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": combined, "annotations": []}],
                    "status": "completed"
                }));
            }

            // Emit tool calls as function_call items.
            for tc in tool_calls {
                input.push(tc);
            }
        }
        _ => {}
    }
}

/// Split a compound tool ID ("call_id|item_id") into its parts.
/// If no separator, use the id as both call_id and synthesize an item_id.
fn split_tool_id(id: &str) -> (String, String) {
    if let Some(pos) = id.find('|') {
        (id[..pos].to_string(), id[pos + 1..].to_string())
    } else {
        (id.to_string(), format!("fc_{id}"))
    }
}

fn content_to_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn image_to_url(source: &layer0::content::ImageSource, media_type: &str) -> Option<String> {
    match source {
        layer0::content::ImageSource::Base64 { data } => {
            Some(format!("data:{media_type};base64,{data}"))
        }
        layer0::content::ImageSource::Url { url } => Some(url.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::Message;

    #[test]
    fn user_text_converts() {
        let msg = Message::new(layer0::context::Role::User, Content::text("hello"));
        let items = messages_to_input(&[msg]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn assistant_text_converts() {
        let msg = Message::new(layer0::context::Role::Assistant, Content::text("hi"));
        let items = messages_to_input(&[msg]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["content"][0]["text"], "hi");
    }

    #[test]
    fn tool_result_converts() {
        let msg = Message::new(
            layer0::context::Role::Tool {
                name: "search".into(),
                call_id: "call_123".into(),
            },
            Content::text("result data"),
        );
        let items = messages_to_input(&[msg]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_123");
        assert_eq!(items[0]["output"], "result data");
    }

    #[test]
    fn split_compound_tool_id() {
        let (call_id, item_id) = split_tool_id("call_abc|fc_xyz");
        assert_eq!(call_id, "call_abc");
        assert_eq!(item_id, "fc_xyz");
    }

    #[test]
    fn split_simple_tool_id() {
        let (call_id, item_id) = split_tool_id("call_abc");
        assert_eq!(call_id, "call_abc");
        assert_eq!(item_id, "fc_call_abc");
    }
}
