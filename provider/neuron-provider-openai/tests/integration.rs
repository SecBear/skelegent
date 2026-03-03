//! Integration test: real OpenAI call through the full stack.

use layer0::content::Content;
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
use neuron_context::SlidingWindow;
use neuron_hooks::HookRegistry;
use neuron_op_react::{ReactConfig, ReactOperator};
use neuron_provider_openai::OpenAIProvider;
use neuron_tool::ToolRegistry;
use std::sync::Arc;

#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn real_gpt4o_mini_simple_completion() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");

    let provider = OpenAIProvider::new(api_key);
    let tools = ToolRegistry::new();
    let strategy = Box::new(SlidingWindow::new());
    let hooks = HookRegistry::new();
    let store = Arc::new(neuron_state_memory::MemoryStore::new()) as Arc<dyn layer0::StateReader>;

    let config = ReactConfig {
        system_prompt: "You are a helpful assistant. Be very concise.".into(),
        default_model: "gpt-4o-mini".into(),
        default_max_tokens: 128,
        default_max_turns: 5,
    };

    let op = ReactOperator::new(provider, tools, strategy, hooks, store, config);

    let input = OperatorInput::new(
        Content::text("Say hello in exactly 3 words."),
        TriggerType::User,
    );

    let output = op.execute(input).await.unwrap();

    assert_eq!(output.exit_reason, ExitReason::Complete);
    assert!(output.message.as_text().is_some());
    let text = output.message.as_text().unwrap();
    assert!(!text.is_empty());
    assert!(output.metadata.tokens_in > 0);
    assert!(output.metadata.tokens_out > 0);
    assert!(output.metadata.cost > rust_decimal::Decimal::ZERO);
    assert_eq!(output.metadata.turns_used, 1);
    assert!(output.effects.is_empty());
}

#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn openai_provider_is_object_safe_as_arc_dyn_operator() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");

    let provider = OpenAIProvider::new(api_key);
    let tools = ToolRegistry::new();
    let strategy = Box::new(SlidingWindow::new());
    let hooks = HookRegistry::new();
    let store = Arc::new(neuron_state_memory::MemoryStore::new()) as Arc<dyn layer0::StateReader>;

    let config = ReactConfig {
        system_prompt: "You are a helpful assistant.".into(),
        default_model: "gpt-4o-mini".into(),
        default_max_tokens: 64,
        default_max_turns: 3,
    };

    // Prove ReactOperator<P> can be used as Arc<dyn Operator>
    let op: Arc<dyn Operator> = Arc::new(ReactOperator::new(
        provider, tools, strategy, hooks, store, config,
    ));

    let input = OperatorInput::new(Content::text("Say hi."), TriggerType::User);
    let output = op.execute(input).await.unwrap();
    assert_eq!(output.exit_reason, ExitReason::Complete);
}
