//! Unified inference types that speak [`Message`] natively.
//!
//! Operators build an [`InferRequest`] using layer0 [`Message`]s directly —
//! no manual conversion to wire-format types. Each [`Provider`] implementation
//! converts to its wire format internally.
//!
//! [`Message`]: layer0::context::Message
//! [`Provider`]: crate::provider::Provider

use std::collections::HashMap;

use layer0::content::Content;
use layer0::context::{Message, Role};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{StopReason, TokenUsage, ToolSchema};

/// Request for a single inference call, using layer0 [`Message`] types.
///
/// Operators build this directly — no conversion to `ProviderMessage` needed.
/// The provider implementation handles wire-format conversion internally.
#[derive(Debug, Clone)]
pub struct InferRequest {
    /// Model to use. `None` = provider default.
    pub model: Option<String>,

    /// Conversation messages in layer0 format.
    pub messages: Vec<Message>,

    /// Available tools the model may call.
    pub tools: Vec<ToolSchema>,

    /// Maximum output tokens.
    pub max_tokens: Option<u32>,

    /// Sampling temperature.
    pub temperature: Option<f64>,

    /// System prompt (injected by the provider as appropriate for its API).
    pub system: Option<String>,

    /// Provider-specific configuration, keyed by provider name.
    ///
    /// Each entry is a JSON object whose fields are interpreted by the named
    /// provider. Using a per-provider key prevents collisions when the same
    /// request is inspected by multiple providers or middleware layers.
    ///
    /// Example: `request.with_provider_option("anthropic", json!({ "thinking": … }))`
    pub provider_options: HashMap<String, serde_json::Value>,

    /// Tool choice constraint.
    pub tool_choice: Option<crate::types::ToolChoice>,

    /// Requested response format.
    pub response_format: Option<crate::types::ResponseFormat>,
}
impl InferRequest {
    /// Create a minimal inference request with just messages.
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            model: None,
            messages,
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
            system: None,
            provider_options: HashMap::new(),
            tool_choice: None,
            response_format: None,
        }
    }

    /// Set the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Set the tools.
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set a provider-specific option block.
    ///
    /// `provider` is a short identifier (e.g., `"anthropic"`, `"openai"`). The
    /// entire `value` object is passed to that provider; it replaces any
    /// previously set value for the same key.
    pub fn with_provider_option(mut self, provider: &str, value: serde_json::Value) -> Self {
        self.provider_options.insert(provider.to_string(), value);
        self
    }

    /// Set tool choice constraint.
    pub fn with_tool_choice(mut self, choice: crate::types::ToolChoice) -> Self {
        self.tool_choice = Some(choice);
        self
    }

    /// Set the requested response format.
    pub fn with_response_format(mut self, format: crate::types::ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }
}

/// Response from a single inference call, using layer0 [`Content`] directly.
///
/// Operators receive this — no conversion from `ProviderResponse` needed.
/// The provider implementation handles wire-format conversion internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferResponse {
    /// The model's response content in layer0 format.
    pub content: Content,

    /// Tool calls requested by the model (if any).
    ///
    /// Extracted separately from content for ergonomic access.
    /// Each entry is `(call_id, tool_name, input_json)`.
    pub tool_calls: Vec<ToolCall>,

    /// Why the model stopped generating.
    pub stop_reason: StopReason,

    /// Token usage for this call.
    pub usage: TokenUsage,

    /// Actual model used (may differ from requested if aliased).
    pub model: String,

    /// Cost in USD (if the provider can calculate it).
    pub cost: Option<Decimal>,

    /// Whether the provider truncated input context.
    pub truncated: Option<bool>,
}

/// A single tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned unique ID for this tool call.
    pub id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// Input arguments as JSON.
    pub input: serde_json::Value,
}

impl InferResponse {
    /// Get the text content of the response, if any.
    pub fn text(&self) -> Option<&str> {
        self.content.as_text()
    }

    /// Consume the response and return its content.
    pub fn into_content(self) -> Content {
        self.content
    }

    /// Whether the model is requesting tool use.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Build an assistant [`Message`] from this response.
    ///
    /// Includes both text content and tool-use blocks so the message
    /// can be appended to context for multi-turn conversations.
    pub fn to_message(&self) -> Message {
        use layer0::content::ContentBlock;

        let mut blocks = Vec::new();

        // Add text content
        match &self.content {
            Content::Text(text) if !text.is_empty() => {
                blocks.push(ContentBlock::Text { text: text.clone() });
            }
            Content::Blocks(bs) => {
                blocks.extend(bs.iter().cloned());
            }
            _ => {}
        }

        // Add tool use blocks
        for tc in &self.tool_calls {
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        let content = if blocks.len() == 1 {
            if let ContentBlock::Text { text } = &blocks[0] {
                Content::Text(text.clone())
            } else {
                Content::Blocks(blocks)
            }
        } else if blocks.is_empty() {
            Content::Text(String::new())
        } else {
            Content::Blocks(blocks)
        };

        Message::new(Role::Assistant, content)
    }

    /// Build a tool-result [`Message`] for feeding back to the model.
    ///
    /// Creates a message with role `Tool` containing the result content.
    pub fn tool_result_message(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        result: impl Into<String>,
        is_error: bool,
    ) -> Message {
        let content = Content::Blocks(vec![layer0::content::ContentBlock::ToolResult {
            tool_use_id: call_id.into(),
            content: result.into(),
            is_error,
        }]);
        Message::new(
            Role::Tool {
                name: tool_name.into(),
                call_id: String::new(), // The call_id is in the ToolResult block
            },
            content,
        )
    }
}

impl From<InferResponse> for Content {
    fn from(response: InferResponse) -> Self {
        response.content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_request_builder() {
        let req = InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
            .with_model("test-model")
            .with_system("Be helpful")
            .with_max_tokens(1024)
            .with_temperature(0.7);

        assert_eq!(req.model.as_deref(), Some("test-model"));
        assert_eq!(req.system.as_deref(), Some("Be helpful"));
        assert_eq!(req.max_tokens, Some(1024));
        assert_eq!(req.temperature, Some(0.7));
        assert_eq!(req.messages.len(), 1);
    }

    #[test]
    fn infer_response_text() {
        let resp = InferResponse {
            content: Content::text("Hello!"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        assert_eq!(resp.text(), Some("Hello!"));
        assert!(!resp.has_tool_calls());
    }

    #[test]
    fn infer_response_with_tool_calls() {
        let resp = InferResponse {
            content: Content::text("Let me search for that."),
            tool_calls: vec![ToolCall {
                id: "tc_1".into(),
                name: "search".into(),
                input: serde_json::json!({"query": "weather"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        assert!(resp.has_tool_calls());
        assert_eq!(resp.tool_calls[0].name, "search");
    }

    #[test]
    fn to_message_text_only() {
        let resp = InferResponse {
            content: Content::text("Hello!"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let msg = resp.to_message();
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content.as_text(), Some("Hello!"));
    }

    #[test]
    fn to_message_with_tool_calls() {
        let resp = InferResponse {
            content: Content::text("Searching..."),
            tool_calls: vec![ToolCall {
                id: "tc_1".into(),
                name: "search".into(),
                input: serde_json::json!({"q": "test"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let msg = resp.to_message();
        assert_eq!(msg.role, Role::Assistant);
        // Should have both text and tool_use blocks
        match &msg.content {
            Content::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
            }
            _ => panic!("expected Blocks content"),
        }
    }

    #[test]
    fn tool_result_message_construction() {
        let msg = InferResponse::tool_result_message("tc_1", "search", "Found 5 results", false);
        match &msg.role {
            Role::Tool { name, .. } => assert_eq!(name, "search"),
            _ => panic!("expected Tool role"),
        }
    }

    #[test]
    fn into_content_returns_inner() {
        let resp = InferResponse {
            content: Content::text("Hello!"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let content = resp.into_content();
        assert_eq!(content.as_text(), Some("Hello!"));
    }

    #[test]
    fn from_infer_response_for_content() {
        let resp = InferResponse {
            content: Content::text("Converted"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let content: Content = resp.into();
        assert_eq!(content.as_text(), Some("Converted"));
    }

    #[test]
    fn provider_options_round_trip() {
        // Set anthropic options, verify they're accessible by key.
        let opts = serde_json::json!({
            "thinking": { "type": "enabled", "budget_tokens": 1024 },
            "system_cache_control": { "type": "ephemeral" }
        });
        let req = InferRequest::new(vec![])
            .with_provider_option("anthropic", opts.clone());
        assert_eq!(req.provider_options.get("anthropic"), Some(&opts));
        assert!(req.provider_options.get("openai").is_none());
    }

    #[test]
    fn tool_choice_serde_round_trip() {
        use crate::types::ToolChoice;
        for choice in [
            ToolChoice::Auto,
            ToolChoice::Any,
            ToolChoice::Tool { name: "search".into() },
            ToolChoice::None,
        ] {
            let json = serde_json::to_value(&choice).unwrap();
            let back: ToolChoice = serde_json::from_value(json).unwrap();
            assert_eq!(choice, back);
        }
    }

    #[test]
    fn response_format_serde_round_trip() {
        use crate::types::ResponseFormat;
        let formats = [
            ResponseFormat::Text,
            ResponseFormat::Json,
            ResponseFormat::JsonSchema {
                name: "my_schema".into(),
                schema: serde_json::json!({"type": "object"}),
                strict: true,
            },
        ];
        for fmt in formats {
            let json = serde_json::to_value(&fmt).unwrap();
            let back: ResponseFormat = serde_json::from_value(json).unwrap();
            assert_eq!(fmt, back);
        }
    }

    #[test]
    fn infer_request_builder_methods() {
        use crate::types::{ToolChoice, ResponseFormat};
        let req = InferRequest::new(vec![])
            .with_provider_option("anthropic", serde_json::json!({ "thinking": { "type": "enabled", "budget_tokens": 2048 } }))
            .with_tool_choice(ToolChoice::Any)
            .with_response_format(ResponseFormat::Json);
        assert_eq!(
            req.provider_options.get("anthropic").and_then(|o| o.get("thinking")),
            Some(&serde_json::json!({ "type": "enabled", "budget_tokens": 2048 }))
        );
        assert_eq!(req.tool_choice, Some(ToolChoice::Any));
        assert_eq!(req.response_format, Some(ResponseFormat::Json));
    }

    #[test]
    fn content_block_thinking_round_trip() {
        use layer0::content::ContentBlock;
        let block = ContentBlock::Thinking {
            thinking: "I need to think...".into(),
            signature: Some("sig123".into()),
        };
        let json = serde_json::to_value(&block).unwrap();
        let back: ContentBlock = serde_json::from_value(json).unwrap();
        assert_eq!(block, back);
    }
}
