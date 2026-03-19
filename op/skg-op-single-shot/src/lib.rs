#![deny(missing_docs)]
//! Single-shot operator — one model call, no tools, return immediately.
//!
//! Implements `layer0::Operator` for the simplest case: send a single
//! prompt to a model and return the result. No tool use, no ReAct loop,
//! no hooks, no state reader. Used for classification, summarization,
//! extraction, and other single-inference tasks.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::duration::DurationMs;
use layer0::error::OperatorError;
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorMetadata, OperatorOutput};
use rust_decimal::Decimal;
use skg_turn::infer::InferRequest;
use skg_turn::provider::Provider;
use std::time::Instant;

/// Static configuration for a SingleShotOperator instance.
pub struct SingleShotConfig {
    /// Base system prompt.
    pub system_prompt: String,
    /// Default model identifier.
    pub default_model: String,
    /// Default max tokens per response.
    pub default_max_tokens: u32,
}

impl Default for SingleShotConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            default_model: String::new(),
            default_max_tokens: 4096,
        }
    }
}

/// A single-shot Operator: one model call, no tools, return immediately.
///
/// Generic over `P: Provider` (not object-safe). The object-safe boundary
/// is `layer0::Operator`, which `SingleShotOperator<P>` implements via
/// `#[async_trait]`.
pub struct SingleShotOperator<P: Provider> {
    provider: P,
    config: SingleShotConfig,
}

impl<P: Provider> SingleShotOperator<P> {
    /// Create a new SingleShotOperator with a provider and configuration.
    pub fn new(provider: P, config: SingleShotConfig) -> Self {
        Self { provider, config }
    }

    /// Resolve model and max_tokens from per-request overrides or defaults.
    fn resolve_model(&self, input: &OperatorInput) -> Option<String> {
        input
            .config
            .as_ref()
            .and_then(|c| c.model.clone())
            .or_else(|| {
                if self.config.default_model.is_empty() {
                    None
                } else {
                    Some(self.config.default_model.clone())
                }
            })
    }

    /// Resolve the system prompt, appending any per-request addendum.
    fn resolve_system(&self, input: &OperatorInput) -> String {
        match input
            .config
            .as_ref()
            .and_then(|c| c.system_addendum.as_ref())
        {
            Some(addendum) => format!("{}\n{}", self.config.system_prompt, addendum),
            None => self.config.system_prompt.clone(),
        }
    }
}

#[async_trait]
impl<P: Provider + 'static> Operator for SingleShotOperator<P> {
    #[tracing::instrument(skip_all, fields(trigger = ?input.trigger))]
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let start = Instant::now();
        tracing::info!("single-shot executing");

        let model = self.resolve_model(&input);
        let system = self.resolve_system(&input);
        let max_tokens = self.config.default_max_tokens;

        // Build single user message from trigger content
        let user_msg = Message::new(Role::User, input.message.clone());

        // Build inference request
        let mut request = InferRequest::new(vec![user_msg]);
        if let Some(m) = model {
            request = request.with_model(m);
        }
        if !system.is_empty() {
            request = request.with_system(system);
        }
        request = request.with_max_tokens(max_tokens);

        // Single model call
        let response = self.provider.infer(request).await.map_err(|e| {
            if e.is_retryable() {
                OperatorError::model_retryable(e)
            } else {
                OperatorError::Model {
                    source: Box::new(e),
                    retryable: false,
                }
            }
        })?;

        let duration = DurationMs::from(start.elapsed());

        // Build metadata
        let mut metadata = OperatorMetadata::default();
        metadata.tokens_in = response.usage.input_tokens;
        metadata.tokens_out = response.usage.output_tokens;
        metadata.cost = response.cost.unwrap_or(Decimal::ZERO);
        metadata.turns_used = 1;
        metadata.sub_dispatches = vec![];
        metadata.duration = duration;

        // Response content is already layer0 Content
        let message: Content = response.content;

        // Always ExitReason::Complete for single-shot
        let mut output = OperatorOutput::new(message, ExitReason::Complete);
        output.metadata = metadata;

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::id::{DispatchId, OperatorId};
    use skg_turn::infer::InferResponse;
    use skg_turn::test_utils::{TestProvider, error_provider_rate_limited, make_text_response};
    use skg_turn::types::{StopReason, TokenUsage};
    use std::sync::Arc;

    fn test_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"))
    }

    // -- Helpers --

    fn simple_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), layer0::operator::TriggerType::User)
    }

    fn make_op(provider: TestProvider) -> SingleShotOperator<TestProvider> {
        SingleShotOperator::new(provider, SingleShotConfig::default())
    }

    // -- Tests --

    #[tokio::test]
    async fn single_shot_returns_completion() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Hi"), &test_ctx()).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    #[tokio::test]
    async fn single_shot_always_one_turn() {
        let provider = TestProvider::with_responses(vec![make_text_response("Response")]);
        let op = make_op(provider);

        let output = op
            .execute(simple_input("Query"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(output.metadata.turns_used, 1);
    }

    #[tokio::test]
    async fn single_shot_no_tools_in_request() {
        let provider = TestProvider::with_responses(vec![make_text_response("Done")]);
        let op = make_op(provider);

        op.execute(simple_input("Test"), &test_ctx()).await.unwrap();

        let requests = op.provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].tools.is_empty(),
            "single-shot must send no tools"
        );
    }

    #[tokio::test]
    async fn single_shot_rate_limit_maps_to_retryable() {
        let provider = error_provider_rate_limited();
        let op = SingleShotOperator::new(provider, SingleShotConfig::default());

        let result = op.execute(simple_input("test"), &test_ctx()).await;
        assert!(matches!(
            result,
            Err(OperatorError::Model {
                retryable: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn single_shot_cost_passed_through() {
        let cost = Decimal::new(42, 4); // $0.0042
        let response = InferResponse {
            content: Content::text("result"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
            },
            model: "mock".into(),
            cost: Some(cost),
            truncated: None,
        };
        let provider = TestProvider::with_responses(vec![response]);
        let op = make_op(provider);

        let output = op.execute(simple_input("test"), &test_ctx()).await.unwrap();

        assert_eq!(output.metadata.cost, cost);
        assert_eq!(output.metadata.tokens_in, 100);
        assert_eq!(output.metadata.tokens_out, 50);
    }

    #[tokio::test]
    async fn single_shot_as_arc_dyn_operator() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);
        let op: Arc<dyn Operator> = Arc::new(SingleShotOperator::new(
            provider,
            SingleShotConfig::default(),
        ));

        let ctx = test_ctx();
        let output = Operator::execute(op.as_ref(), simple_input("Hi"), &ctx)
            .await
            .unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }
}
