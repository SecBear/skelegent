//! Integration test: real OpenAI call through the full stack.

use layer0::content::Content;
use layer0::operator::{ExitReason, Operator, OperatorInput, TriggerType};
// TODO: migrate to context-engine
use neuron_provider_openai::OpenAIProvider;
use neuron_tool::ToolRegistry;
use std::sync::Arc;

#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn real_gpt4o_mini_simple_completion() {
    // TODO: rewrite using neuron-context-engine
}

#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn openai_provider_is_object_safe_as_arc_dyn_operator() {
    // TODO: rewrite using neuron-context-engine
}
