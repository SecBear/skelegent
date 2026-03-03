#![deny(missing_docs)]
//! Ollama local model provider for neuron-turn.
//!
//! Implements the [`neuron_turn::Provider`] trait for Ollama's `/api/chat` endpoint.
//! Ollama runs models locally, so there are no auth headers and cost is always zero.

mod types;

use neuron_turn::provider::{Provider, ProviderError};
use neuron_turn::types::*;
use rust_decimal::Decimal;
use types::*;
use uuid::Uuid;

/// Ollama local model provider.
pub struct OllamaProvider {
    client: reqwest::Client,
    api_url: String,
    keep_alive: Option<String>,
}

impl OllamaProvider {
    /// Create a new Ollama provider pointed at the default local endpoint.
    ///
    /// Defaults to `http://localhost:11434/api/chat`.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_url: "http://localhost:11434/api/chat".into(),
            keep_alive: None,
        }
    }

    /// Override the API URL (for remote Ollama instances or custom ports).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    /// Set the `keep_alive` duration for how long Ollama keeps the model loaded.
    ///
    /// Examples: `"5m"`, `"0"` (unload immediately), `"-1"` (keep forever).
    pub fn with_keep_alive(mut self, duration: impl Into<String>) -> Self {
        self.keep_alive = Some(duration.into());
        self
    }

    fn build_request(&self, request: &ProviderRequest) -> OllamaRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "llama3.2:1b".into());

        let mut messages: Vec<OllamaMessage> = Vec::new();

        // System prompt becomes a system message.
        if let Some(ref system) = request.system {
            messages.push(OllamaMessage {
                role: "system".into(),
                content: system.clone(),
                tool_calls: None,
            });
        }

        // Map ProviderMessages to Ollama messages.
        for m in &request.messages {
            match m.role {
                Role::System => {
                    let text = extract_text(&m.content);
                    messages.push(OllamaMessage {
                        role: "system".into(),
                        content: text,
                        tool_calls: None,
                    });
                }
                Role::User => {
                    // Separate tool results from other content.
                    let mut tool_results = Vec::new();
                    let mut other_text = Vec::new();
                    for part in &m.content {
                        match part {
                            ContentPart::ToolResult { content, .. } => {
                                tool_results.push(content.clone());
                            }
                            ContentPart::Text { text } => {
                                other_text.push(text.clone());
                            }
                            _ => {}
                        }
                    }

                    // Emit tool result messages as role="tool".
                    for content in tool_results {
                        messages.push(OllamaMessage {
                            role: "tool".into(),
                            content,
                            tool_calls: None,
                        });
                    }

                    // Emit user message if there is text content.
                    if !other_text.is_empty() {
                        messages.push(OllamaMessage {
                            role: "user".into(),
                            content: other_text.join("\n"),
                            tool_calls: None,
                        });
                    }
                }
                Role::Assistant => {
                    // Check for tool use parts.
                    let mut tool_calls = Vec::new();
                    let mut text_parts = Vec::new();
                    for part in &m.content {
                        match part {
                            ContentPart::ToolUse { name, input, .. } => {
                                tool_calls.push(OllamaToolCall {
                                    function: OllamaFunctionCall {
                                        name: name.clone(),
                                        arguments: input.clone(),
                                    },
                                });
                            }
                            ContentPart::Text { text } => {
                                text_parts.push(text.clone());
                            }
                            _ => {}
                        }
                    }

                    let content = text_parts.join("\n");
                    let tool_calls_field = if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    };

                    messages.push(OllamaMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls: tool_calls_field,
                    });
                }
            }
        }

        let tools: Vec<OllamaTool> = request
            .tools
            .iter()
            .map(|t| OllamaTool {
                tool_type: "function".into(),
                function: OllamaFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // Build options from temperature and max_tokens.
        let options = if request.temperature.is_some() || request.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                ..Default::default()
            })
        } else {
            None
        };

        OllamaRequest {
            model,
            messages,
            stream: false,
            tools,
            keep_alive: self.keep_alive.clone(),
            options,
        }
    }

    fn parse_response(&self, response: OllamaResponse) -> ProviderResponse {
        let mut content: Vec<ContentPart> = Vec::new();

        // Extract text content from the message.
        if !response.message.content.is_empty() {
            content.push(ContentPart::Text {
                text: response.message.content.clone(),
            });
        }

        // Extract tool calls, synthesizing UUIDs since Ollama does not provide IDs.
        let has_tool_calls = response
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty());

        if let Some(tool_calls) = &response.message.tool_calls {
            for tc in tool_calls {
                content.push(ContentPart::ToolUse {
                    id: Uuid::new_v4().to_string(),
                    name: tc.function.name.clone(),
                    input: tc.function.arguments.clone(),
                });
            }
        }

        // Determine stop reason.
        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            match response.done_reason.as_deref() {
                Some("stop") => StopReason::EndTurn,
                Some("length") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            }
        };

        // Map Ollama timing fields to token usage.
        let usage = TokenUsage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
            cache_read_tokens: None,
            cache_creation_tokens: None,
        };

        ProviderResponse {
            content,
            stop_reason,
            usage,
            model: response.model,
            cost: Some(Decimal::ZERO),
            truncated: None,
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OllamaProvider {
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send {
        let api_request = self.build_request(&request);
        let http_request = self
            .client
            .post(&self.api_url)
            .header("content-type", "application/json")
            .json(&api_request);

        async move {
            let http_response = http_request
                .send()
                .await
                .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

            let status = http_response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited);
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = http_response.text().await.unwrap_or_default();
                return Err(ProviderError::AuthFailed(body));
            }
            if !status.is_success() {
                let body = http_response.text().await.unwrap_or_default();
                return Err(ProviderError::RequestFailed(format!(
                    "HTTP {status}: {body}"
                )));
            }

            let api_response: OllamaResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            Ok(self.parse_response(api_response))
        }
    }
}

fn extract_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_simple_request() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: Some("llama3.2:1b".into()),
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: Some(256),
            temperature: None,
            system: Some("Be helpful.".into()),
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "llama3.2:1b");
        assert!(!api_request.stream);
        // System prompt becomes the first message.
        assert_eq!(api_request.messages.len(), 2);
        assert_eq!(api_request.messages[0].role, "system");
        assert_eq!(api_request.messages[0].content, "Be helpful.");
        assert_eq!(api_request.messages[1].role, "user");
        assert_eq!(api_request.messages[1].content, "Hello");
        // max_tokens maps to options.num_predict.
        assert_eq!(api_request.options.as_ref().unwrap().num_predict, Some(256));
    }

    #[test]
    fn parse_simple_response() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: "Hello!".into(),
                tool_calls: None,
            },
            done: true,
            done_reason: Some("stop".into()),
            total_duration: Some(500_000_000),
            load_duration: Some(100_000_000),
            prompt_eval_count: Some(10),
            prompt_eval_duration: Some(200_000_000),
            eval_count: Some(5),
            eval_duration: Some(200_000_000),
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.cost, Some(Decimal::ZERO));
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn parse_tool_use_response() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: "bash".into(),
                        arguments: json!({"command": "ls"}),
                    },
                }]),
            },
            done: true,
            done_reason: Some("stop".into()),
            total_duration: Some(500_000_000),
            load_duration: None,
            prompt_eval_count: Some(20),
            prompt_eval_duration: None,
            eval_count: Some(30),
            eval_duration: None,
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::ToolUse { id, name, input } => {
                assert!(!id.is_empty(), "synthesized ID must not be empty");
                assert_eq!(name, "bash");
                assert_eq!(input, &json!({"command": "ls"}));
                // Arguments are a JSON object, not a string.
                assert!(input.is_object());
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn tool_schema_serializes() {
        let tool = OllamaTool {
            tool_type: "function".into(),
            function: OllamaFunction {
                name: "get_weather".into(),
                description: "Get current weather".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }),
            },
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "get_weather");
        assert!(json["function"]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn synthesized_tool_ids_are_unique() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![
                    OllamaToolCall {
                        function: OllamaFunctionCall {
                            name: "tool_a".into(),
                            arguments: json!({"x": 1}),
                        },
                    },
                    OllamaToolCall {
                        function: OllamaFunctionCall {
                            name: "tool_b".into(),
                            arguments: json!({"y": 2}),
                        },
                    },
                ]),
            },
            done: true,
            done_reason: Some("stop".into()),
            total_duration: None,
            load_duration: None,
            prompt_eval_count: None,
            prompt_eval_duration: None,
            eval_count: None,
            eval_duration: None,
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.content.len(), 2);
        let ids: Vec<String> = response
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "each tool call must have a unique ID");
    }

    #[test]
    fn timing_metadata_parsed() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: "done".into(),
                tool_calls: None,
            },
            done: true,
            done_reason: Some("stop".into()),
            total_duration: Some(1_000_000_000),
            load_duration: Some(200_000_000),
            prompt_eval_count: Some(42),
            prompt_eval_duration: Some(300_000_000),
            eval_count: Some(17),
            eval_duration: Some(500_000_000),
        };

        let response = provider.parse_response(api_response);
        // prompt_eval_count -> input_tokens.
        assert_eq!(response.usage.input_tokens, 42);
        // eval_count -> output_tokens.
        assert_eq!(response.usage.output_tokens, 17);
        // Local inference is free.
        assert_eq!(response.cost, Some(Decimal::ZERO));
    }

    #[test]
    fn default_model_is_llama() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text { text: "Hi".into() }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "llama3.2:1b");
    }

    #[test]
    fn keep_alive_serialized() {
        let provider = OllamaProvider::new().with_keep_alive("5m");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text { text: "Hi".into() }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.keep_alive, Some("5m".into()));
    }

    #[test]
    fn tool_result_becomes_tool_role_message() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: None,
            messages: vec![
                ProviderMessage {
                    role: Role::Assistant,
                    content: vec![ContentPart::ToolUse {
                        id: "call_1".into(),
                        name: "bash".into(),
                        input: json!({"command": "ls"}),
                    }],
                },
                ProviderMessage {
                    role: Role::User,
                    content: vec![ContentPart::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: "file.txt".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        // Assistant message with tool_calls.
        assert_eq!(api_request.messages[0].role, "assistant");
        assert!(api_request.messages[0].tool_calls.is_some());
        // Tool result becomes role="tool" message.
        assert_eq!(api_request.messages[1].role, "tool");
        assert_eq!(api_request.messages[1].content, "file.txt");
    }

    #[test]
    fn missing_timing_fields_default_to_zero() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: "hi".into(),
                tool_calls: None,
            },
            done: true,
            done_reason: Some("stop".into()),
            total_duration: None,
            load_duration: None,
            prompt_eval_count: None,
            prompt_eval_duration: None,
            eval_count: None,
            eval_duration: None,
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.usage.input_tokens, 0);
        assert_eq!(response.usage.output_tokens, 0);
    }

    #[test]
    fn parse_length_stop_reason() {
        let provider = OllamaProvider::new();
        let api_response = OllamaResponse {
            model: "llama3.2:1b".into(),
            message: OllamaMessage {
                role: "assistant".into(),
                content: "trunca...".into(),
                tool_calls: None,
            },
            done: true,
            done_reason: Some("length".into()),
            total_duration: None,
            load_duration: None,
            prompt_eval_count: None,
            prompt_eval_duration: None,
            eval_count: None,
            eval_duration: None,
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::MaxTokens);
    }

    #[test]
    fn with_url_overrides_api_url() {
        let provider = OllamaProvider::new().with_url("http://remote:11434/api/chat");
        assert_eq!(provider.api_url, "http://remote:11434/api/chat");
    }

    #[test]
    fn build_request_with_tools() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Help".into(),
                }],
            }],
            tools: vec![ToolSchema {
                name: "bash".into(),
                description: "Run a command".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: None,
            temperature: Some(0.5),
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.tools.len(), 1);
        assert_eq!(api_request.tools[0].function.name, "bash");
        assert_eq!(api_request.options.as_ref().unwrap().temperature, Some(0.5));
    }

    #[test]
    fn ollama_default_impl() {
        let provider = OllamaProvider::default();
        assert_eq!(provider.api_url, "http://localhost:11434/api/chat");
    }

    #[test]
    fn build_request_no_options_when_no_temp_or_tokens() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text { text: "Hi".into() }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert!(api_request.options.is_none());
    }

    #[test]
    fn build_request_system_message_from_system_role() {
        let provider = OllamaProvider::new();
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::System,
                content: vec![ContentPart::Text {
                    text: "You are helpful.".into(),
                }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.messages[0].role, "system");
        assert_eq!(api_request.messages[0].content, "You are helpful.");
    }
}
