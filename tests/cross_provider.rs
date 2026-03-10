//! Cross-provider integration tests.
//!
//! Run with API keys set:
//! ```bash
//! ANTHROPIC_API_KEY=... OPENAI_API_KEY=... cargo test --test cross_provider -- --ignored
//! ```
//!
//! All tests require live API keys and are `#[ignore]` by default.
//! They verify that OperatorOutput structure is consistent across providers.

use layer0::OperatorId;
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
use skg_context_engine::{Context, ReactLoopConfig, react_loop};
use skg_op_single_shot::{SingleShotConfig, SingleShotOperator};
use skg_provider_anthropic::AnthropicProvider;
use skg_provider_ollama::OllamaProvider;
use skg_provider_openai::OpenAIProvider;
use skg_tool::{ToolCallContext, ToolRegistry};
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
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
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
    let provider = AnthropicProvider::new(api_key);

    let mut ctx = Context::new();
    ctx.inject_message(Message::new(
        Role::User,
        Content::text("Say hello in exactly 3 words."),
    ))
    .await
    .unwrap();

    let tools = ToolRegistry::new();
    let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
    let config = ReactLoopConfig {
        system_prompt: "You are a concise assistant. Follow instructions exactly.".into(),
        model: Some("claude-haiku-4-5-20251001".into()),
        max_tokens: Some(256),
        temperature: None,
        tool_filter: None,
    };

    let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config)
        .await
        .expect("react_loop should succeed");

    let text = output.message.as_text().unwrap_or("");
    println!("Anthropic react_loop response: {text}");
    assert!(!text.is_empty(), "response should not be empty");
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
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let provider = OpenAIProvider::new(api_key);

    let mut ctx = Context::new();
    ctx.inject_message(Message::new(
        Role::User,
        Content::text("Say hello in exactly 3 words."),
    ))
    .await
    .unwrap();

    let tools = ToolRegistry::new();
    let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
    let config = ReactLoopConfig {
        system_prompt: "You are a concise assistant. Follow instructions exactly.".into(),
        model: Some("gpt-4o-mini".into()),
        max_tokens: Some(256),
        temperature: None,
        tool_filter: None,
    };

    let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config)
        .await
        .expect("react_loop should succeed");

    let text = output.message.as_text().unwrap_or("");
    println!("OpenAI react_loop response: {text}");
    assert!(!text.is_empty(), "response should not be empty");
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
    let provider = OllamaProvider::new();

    let mut ctx = Context::new();
    ctx.inject_message(Message::new(
        Role::User,
        Content::text("Say hello in exactly 3 words."),
    ))
    .await
    .unwrap();

    let tools = ToolRegistry::new();
    let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
    let config = ReactLoopConfig {
        system_prompt: "You are a concise assistant. Follow instructions exactly.".into(),
        model: Some("llama3.2:1b".into()),
        max_tokens: Some(256),
        temperature: None,
        tool_filter: None,
    };

    let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config)
        .await
        .expect("react_loop should succeed");

    let text = output.message.as_text().unwrap_or("");
    println!("Ollama react_loop response: {text}");
    assert!(!text.is_empty(), "response should not be empty");
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context engineering integration tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn anthropic_summarize_live() {
    use skg_context_engine::rules::compaction::summarize;

    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
    let provider = AnthropicProvider::new(api_key);

    // Build a conversation worth summarizing
    let messages = vec![
        Message::new(
            Role::User,
            Content::text("I'm building an agent framework in Rust called skelegent."),
        ),
        Message::new(
            Role::Assistant,
            Content::text("That sounds interesting! What are the key design decisions?"),
        ),
        Message::new(
            Role::User,
            Content::text(
                "We use a 6-layer architecture: layer0 (types), turn (inference), tool, context-engine, orch, and skelegent (top-level). Provider is RPITIT, not object-safe.",
            ),
        ),
        Message::new(
            Role::Assistant,
            Content::text(
                "The RPITIT approach for Provider gives you zero-cost generics. What about context management?",
            ),
        ),
        Message::new(
            Role::User,
            Content::text(
                "Context engineering is composable ops. ContextOp trait, rules with triggers, pure functions like sliding_window and policy_trim, plus async strategies like summarize that take a Provider.",
            ),
        ),
    ];

    let summary = summarize(&messages, &provider)
        .await
        .expect("summarize should succeed");

    // Verify the summary is a valid message
    assert_eq!(summary.role, Role::Assistant);
    let text = summary.text_content();
    assert!(!text.is_empty(), "summary should not be empty");
    println!(
        "Summary ({} chars): {}",
        text.len(),
        &text[..text.len().min(200)]
    );

    // Summary should be pinned (survives further compaction)
    assert_eq!(
        summary.meta.policy,
        layer0::lifecycle::CompactionPolicy::Pinned,
        "summary should be pinned"
    );
}

#[tokio::test]
#[ignore]
async fn anthropic_extract_cognitive_state_live() {
    use skg_context_engine::rules::compaction::extract_cognitive_state;

    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
    let provider = AnthropicProvider::new(api_key);

    let messages = vec![
        Message::new(
            Role::User,
            Content::text(
                "We decided to use SQLite for the state store. The API design is complete but testing is still pending.",
            ),
        ),
        Message::new(
            Role::Assistant,
            Content::text(
                "Good choice. SQLite gives you FTS5 for text search. What about vector search?",
            ),
        ),
        Message::new(
            Role::User,
            Content::text("Vector search will be optional behind a feature flag using sqlite-vec."),
        ),
    ];

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "decisions": { "type": "array", "items": { "type": "string" } },
            "open_questions": { "type": "array", "items": { "type": "string" } },
            "current_status": { "type": "string" }
        }
    });

    let state = extract_cognitive_state(&messages, &provider, &schema)
        .await
        .expect("extract_cognitive_state should succeed");

    println!(
        "Cognitive state: {}",
        serde_json::to_string_pretty(&state).unwrap()
    );

    // Should be a JSON object
    assert!(state.is_object(), "cognitive state should be a JSON object");
    // Should have at least some of the schema fields
    let obj = state.as_object().unwrap();
    assert!(
        obj.contains_key("decisions")
            || obj.contains_key("open_questions")
            || obj.contains_key("current_status"),
        "cognitive state should contain at least one schema field"
    );
}
