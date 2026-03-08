//! Integration test: real Anthropic Haiku call through the full stack.

use layer0::content::Content;
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
// TODO: migrate to context-engine
use neuron_provider_anthropic::AnthropicProvider;
use neuron_tool::ToolRegistry;
use std::sync::Arc;

#[tokio::test]
#[ignore] // Requires ANTHROPIC_API_KEY environment variable
async fn real_haiku_simple_completion() {
    // TODO: rewrite using neuron-context-engine
}

#[tokio::test]
#[ignore] // Requires ANTHROPIC_API_KEY environment variable
async fn neuron_turn_is_object_safe_as_arc_dyn_operator() {
    // TODO: rewrite using neuron-context-engine
}
