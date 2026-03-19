//! Integration tests for the Ollama provider.
//!
//! These tests require a running Ollama instance with the `llama3.2:1b` model pulled.
//! Run with: `cargo test --test integration -- --ignored`

use layer0::content::Content;
use layer0::context::{Message, Role};
use serde_json::json;
use skg_provider_ollama::OllamaProvider;
use skg_turn::infer::InferRequest;
use skg_turn::provider::Provider;
use skg_turn::types::{StopReason, ToolSchema};

#[tokio::test]
#[ignore = "requires local Ollama running with llama3.2:1b"]
async fn simple_completion() {
    let provider = OllamaProvider::new();
    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("Say hello in one word."),
    )])
    .with_model("llama3.2:1b")
    .with_system("Respond concisely.")
    .with_max_tokens(32)
    .with_temperature(0.0);

    let response = provider.infer(request).await.unwrap();
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    assert!(!response.content.as_text().unwrap_or("").is_empty());
    assert!(response.usage.input_tokens > 0);
    assert!(response.usage.output_tokens > 0);
    assert_eq!(response.cost, Some(rust_decimal::Decimal::ZERO));
}

#[tokio::test]
#[ignore = "requires local Ollama running with llama3.2:1b"]
async fn tool_use_completion() {
    let provider = OllamaProvider::new();
    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("What is the weather in San Francisco?"),
    )])
    .with_model("llama3.2:1b")
    .with_tools(vec![ToolSchema {
        name: "get_weather".into(),
        description: "Get the current weather for a location.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city name"
                }
            },
            "required": ["location"]
        }),
        extra: None,
    }])
    .with_max_tokens(256)
    .with_temperature(0.0);

    let response = provider.infer(request).await.unwrap();
    // The model may or may not call the tool, but the response should parse.
    assert!(
        !response.content.as_text().unwrap_or("").is_empty() || !response.tool_calls.is_empty()
    );
}
