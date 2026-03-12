//! Append an inference response to context.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use skg_turn::infer::InferResponse;

/// Append an [`InferResponse`] as an assistant message to the context.
///
/// This is explicitly a separate operation from inference itself. The model
/// produces a response; what you do with it is a choice. `AppendResponse`
/// is the choice to add it to the conversation history.
///
/// Also updates metrics with the response's token usage and cost.
pub struct AppendResponse {
    /// The response to append.
    pub response: InferResponse,
}

impl AppendResponse {
    /// Create from an [`InferResponse`].
    pub fn new(response: InferResponse) -> Self {
        Self { response }
    }
}

#[async_trait]
impl ContextOp for AppendResponse {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        // Append assistant message
        ctx.push_message(self.response.to_message());

        // Update metrics
        ctx.metrics.tokens_in += self.response.usage.input_tokens;
        ctx.metrics.tokens_out += self.response.usage.output_tokens;
        if let Some(cost) = self.response.cost {
            ctx.metrics.cost += cost;
        }

        Ok(())
    }
}
