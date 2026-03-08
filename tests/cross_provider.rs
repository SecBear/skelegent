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
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
use neuron_context_engine::{Context, ReactLoopConfig, react_loop};
use neuron_op_single_shot::{SingleShotConfig, SingleShotOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_provider_ollama::OllamaProvider;
use neuron_provider_openai::OpenAIProvider;
use neuron_tool::ToolRegistry;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Null state reader: no history, no state.
struct NullStateReader;

#[async_trait::async_trait]
impl layer0::StateReader for NullStateReader {
    async fn read(
        &self,
        _scope: &layer0::effect::Scope,
        _key: &str,
    ) -> Result<Option<serde_json::Value>, layer0::StateError> {
        Ok(None)
    }
    async fn list(
        &self,
        _scope: &layer0::effect::Scope,
        _prefix: &str,
    ) -> Result<Vec<String>, layer0::StateError> {
        Ok(vec![])
    }
    async fn search(
        &self,
        _scope: &layer0::effect::Scope,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<layer0::state::SearchResult>, layer0::StateError> {
        Ok(vec![])
    }
}

fn react_config(model: &str) -> ReactLoopConfig {
    ReactLoopConfig {
        system_prompt: "You are a concise assistant. Follow instructions exactly.".into(),
        model: Some(model.into()),
        max_tokens: Some(256),
        temperature: None,
    }
}

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
    // TODO: migrate to context-engine
    // This test used ReactOperator with AnthropicProvider.
    // Rewrite to use react_loop() with Context::new() and inject_message().
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OpenAI tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn openai_react_simple_prompt() {
    // TODO: migrate to context-engine
    // This test used ReactOperator with OpenAIProvider.
    // Rewrite to use react_loop() with Context::new() and inject_message().
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Ollama tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
#[ignore]
async fn ollama_react_simple_prompt() {
    // TODO: migrate to context-engine
    // This test used ReactOperator with OllamaProvider.
    // Rewrite to use react_loop() with Context::new() and inject_message().
}
