//! hello-claude — minimal integration example.
//!
//! Reads the Anthropic OAuth token from OMP's agent.db,
//! sends a single inference request via SingleShotOperator,
//! and prints the response.

use layer0::content::Content;
use layer0::operator::{Operator, OperatorInput, TriggerType};
use layer0::{DispatchContext, DispatchId, OperatorId};
use skg_auth_omp::OmpAuthProvider;
use skg_op_single_shot::{SingleShotConfig, SingleShotOperator};
use skg_provider_anthropic::AnthropicProvider;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build auth provider from OMP's credential store.
    let auth = Arc::new(OmpAuthProvider::from_env().expect("OMP agent.db not found"));
    let provider = AnthropicProvider::with_auth(auth);

    // 2. Build operator.
    let config = SingleShotConfig {
        system_prompt: "You are a helpful assistant. Be concise.".into(),
        default_model: "claude-sonnet-4-20250514".into(),
        default_max_tokens: 1024,
    };
    let op = SingleShotOperator::new(provider, config);

    // 3. Build input.
    let input = OperatorInput::new(
        Content::text(
            "What is the meaning of the word 'skelegent'? Make one up if you don't know.",
        ),
        TriggerType::User,
    );

    // 4. Execute and print.
    let ctx = DispatchContext::new(DispatchId::new("hello"), OperatorId::new("single-shot"));
    let output = op.execute(input, &ctx).await?;
    println!(
        "Response: {}",
        output.message.as_text().unwrap_or("(no text)")
    );
    println!(
        "Tokens: in={}, out={}",
        output.metadata.tokens_in, output.metadata.tokens_out
    );
    println!("Cost: ${}", output.metadata.cost);

    Ok(())
}
