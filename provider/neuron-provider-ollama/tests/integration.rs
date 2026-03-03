//! Integration tests for the Ollama provider.
//!
//! These tests require a running Ollama instance with the `llama3.2:1b` model pulled.
//! Run with: `cargo test --test integration -- --ignored`

use neuron_provider_ollama::OllamaProvider;
use neuron_turn::provider::Provider;
use neuron_turn::types::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires local Ollama running with llama3.2:1b"]
async fn simple_completion() {
    let provider = OllamaProvider::new();
    let request = ProviderRequest {
        model: Some("llama3.2:1b".into()),
        messages: vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "Say hello in one word.".into(),
            }],
        }],
        tools: vec![],
        max_tokens: Some(32),
        temperature: Some(0.0),
        system: Some("Respond concisely.".into()),
        extra: json!(null),
    };

    let response = provider.complete(request).await.unwrap();
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    assert!(!response.content.is_empty());
    assert!(response.usage.input_tokens > 0);
    assert!(response.usage.output_tokens > 0);
    assert_eq!(response.cost, Some(rust_decimal::Decimal::ZERO));
}

#[tokio::test]
#[ignore = "requires local Ollama running with llama3.2:1b"]
async fn tool_use_completion() {
    let provider = OllamaProvider::new();
    let request = ProviderRequest {
        model: Some("llama3.2:1b".into()),
        messages: vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "What is the weather in San Francisco?".into(),
            }],
        }],
        tools: vec![ToolSchema {
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
        }],
        max_tokens: Some(256),
        temperature: Some(0.0),
        system: None,
        extra: json!(null),
    };

    let response = provider.complete(request).await.unwrap();
    // The model may or may not call the tool, but the response should parse.
    assert!(!response.content.is_empty());
}
