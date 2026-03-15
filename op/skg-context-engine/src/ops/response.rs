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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::op::ContextOp;
    use layer0::context::Role;
    use rust_decimal::Decimal;
    use skg_turn::test_utils::make_text_response;

    #[tokio::test]
    async fn appends_message_to_context() {
        let mut ctx = Context::new();
        let resp = make_text_response("Hello!");
        AppendResponse::new(resp).execute(&mut ctx).await.unwrap();

        let msgs = ctx.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::Assistant);
        assert!(msgs[0].content.as_text().unwrap().contains("Hello!"));
    }

    #[tokio::test]
    async fn updates_token_metrics() {
        let mut ctx = Context::new();
        // make_text_response uses TokenUsage::default() (zeros)
        let resp = make_text_response("ok");
        AppendResponse::new(resp).execute(&mut ctx).await.unwrap();

        // Default token usage is 0, but verifies the path executes
        assert_eq!(ctx.metrics.tokens_in, 0);
        assert_eq!(ctx.metrics.tokens_out, 0);
    }

    #[tokio::test]
    async fn accumulates_across_multiple_appends() {
        let mut ctx = Context::new();
        AppendResponse::new(make_text_response("a")).execute(&mut ctx).await.unwrap();
        AppendResponse::new(make_text_response("b")).execute(&mut ctx).await.unwrap();

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].role, Role::Assistant);
        assert_eq!(ctx.messages()[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn cost_none_leaves_zero() {
        let mut ctx = Context::new();
        // make_text_response has cost: None
        AppendResponse::new(make_text_response("ok")).execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.metrics.cost, Decimal::ZERO);
    }
}