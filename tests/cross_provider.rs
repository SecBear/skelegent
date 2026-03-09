//! Cross-provider integration tests.
//!
//! Run with API keys set:
//! ```bash
//! ANTHROPIC_API_KEY=... OPENAI_API_KEY=... cargo test --test cross_provider -- --ignored
//! ```
//!
//! All tests require live API keys and are `#[ignore]` by default.
//! They verify that OperatorOutput structure is consistent across providers.

use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
use neuron_op_single_shot::{SingleShotConfig, SingleShotOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_provider_ollama::OllamaProvider;
use neuron_provider_openai::OpenAIProvider;
use neuron_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
use std::sync::{Arc, Mutex};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn single_shot_config(model: &str) -> SingleShotConfig {
    SingleShotConfig {
        system_prompt: "You are a concise assistant. Follow instructions exactly.".into(),
        default_model: model.into(),
        default_max_tokens: 256,
    }
}

fn simple_input(text: &str) -> OperatorInput {
    OperatorInput::new(Content::text(text), TriggerType::User)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Anthropic tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn anthropic_react_simple_prompt() {
    // TODO: migrate to context-engine using react_loop() + Context::new()
}

#[tokio::test]
#[ignore]
async fn anthropic_single_shot() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
    let provider = AnthropicProvider::new(api_key);
    let config = single_shot_config("claude-haiku-4-5-20251001");

    let op = SingleShotOperator::new(provider, config);

    let output = op
        .execute(simple_input("Say hello in exactly 3 words."))
        .await
        .expect("Anthropic SingleShotOperator should succeed");

    assert_eq!(output.exit_reason, ExitReason::Complete);
    assert!(output.message.as_text().is_some());
    assert!(output.metadata.tokens_in > 0);
    assert!(output.metadata.tokens_out > 0);
    assert!(output.metadata.cost >= rust_decimal::Decimal::ZERO);
    assert_eq!(output.metadata.turns_used, 1);
    assert!(output.metadata.sub_dispatches.is_empty());
}

#[tokio::test]
#[ignore]
async fn anthropic_streaming_text() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
    let provider = AnthropicProvider::new(api_key);

    let request = StreamRequest {
        model: Some("claude-haiku-4-5-20251001".into()),
        messages: vec![Message::new(
            Role::User,
            Content::text("Say hello in exactly 3 words."),
        )],
        tools: vec![],
        max_tokens: Some(64),
        temperature: Some(0.0),
        system: None,
        extra: serde_json::Value::Null,
    };

    let events: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = Arc::clone(&events);

    let response = provider
        .infer_stream(request, move |event| {
            events_clone.lock().unwrap().push(event);
        })
        .await
        .expect("streaming should succeed");

    // Verify response has text
    let text = response.text().expect("response should have text");
    assert!(!text.is_empty(), "response text should not be empty");
    println!("Streaming response: {text}");

    // Verify events were emitted
    let captured = events.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(_))),
        "should have received at least one TextDelta event"
    );
    assert!(
        captured.iter().any(|e| matches!(e, StreamEvent::Usage(_))),
        "should have received a Usage event"
    );
    assert!(
        captured.iter().any(|e| matches!(e, StreamEvent::Done(_))),
        "should have received a Done event"
    );

    // Verify usage is populated
    assert!(response.usage.input_tokens > 0);
    assert!(response.usage.output_tokens > 0);
    println!(
        "Tokens: {} in, {} out",
        response.usage.input_tokens, response.usage.output_tokens
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OpenAI tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn openai_react_simple_prompt() {
    // TODO: migrate to context-engine using react_loop() + Context::new()
}

#[tokio::test]
#[ignore]
async fn openai_single_shot() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let provider = OpenAIProvider::new(api_key);
    let config = single_shot_config("gpt-4o-mini");

    let op = SingleShotOperator::new(provider, config);

    let output = op
        .execute(simple_input("Say hello in exactly 3 words."))
        .await
        .expect("OpenAI SingleShotOperator should succeed");

    assert_eq!(output.exit_reason, ExitReason::Complete);
    assert!(output.message.as_text().is_some());
    assert!(output.metadata.tokens_in > 0);
    assert!(output.metadata.tokens_out > 0);
    assert!(output.metadata.cost >= rust_decimal::Decimal::ZERO);
    assert_eq!(output.metadata.turns_used, 1);
}

#[tokio::test]
#[ignore]
async fn openai_streaming_text() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let provider = OpenAIProvider::new(api_key);

    let request = StreamRequest {
        model: Some("gpt-4o-mini".into()),
        messages: vec![Message::new(
            Role::User,
            Content::text("Say hello in exactly 3 words."),
        )],
        tools: vec![],
        max_tokens: Some(64),
        temperature: Some(0.0),
        system: None,
        extra: serde_json::Value::Null,
    };

    let events: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = Arc::clone(&events);

    let response = provider
        .infer_stream(request, move |event| {
            events_clone.lock().unwrap().push(event);
        })
        .await
        .expect("streaming should succeed");

    let text = response.text().expect("response should have text");
    assert!(!text.is_empty());
    println!("OpenAI streaming response: {text}");

    let captured = events.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(_)))
    );
    assert!(captured.iter().any(|e| matches!(e, StreamEvent::Done(_))));
    assert!(response.usage.input_tokens > 0);
    assert!(response.usage.output_tokens > 0);
    println!(
        "Tokens: {} in, {} out",
        response.usage.input_tokens, response.usage.output_tokens
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Ollama tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn ollama_react_simple_prompt() {
    // TODO: migrate to context-engine using react_loop() + Context::new()
}

#[tokio::test]
#[ignore]
async fn ollama_streaming_text() {
    let provider = OllamaProvider::new();

    let request = StreamRequest {
        model: Some("llama3.2:1b".into()),
        messages: vec![Message::new(
            Role::User,
            Content::text("Say hello in exactly 3 words."),
        )],
        tools: vec![],
        max_tokens: Some(64),
        temperature: Some(0.0),
        system: None,
        extra: serde_json::Value::Null,
    };

    let events: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = Arc::clone(&events);

    let response = provider
        .infer_stream(request, move |event| {
            events_clone.lock().unwrap().push(event);
        })
        .await
        .expect("streaming should succeed");

    let text = response.text().expect("response should have text");
    assert!(!text.is_empty());
    println!("Ollama streaming response: {text}");

    let captured = events.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(_)))
    );
    assert!(captured.iter().any(|e| matches!(e, StreamEvent::Done(_))));
    println!(
        "Tokens: {} in, {} out",
        response.usage.input_tokens, response.usage.output_tokens
    );
}
