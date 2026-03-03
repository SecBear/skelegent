#![deny(missing_docs)]
//! Single-shot operator — one model call, no tools, return immediately.
//!
//! Implements `layer0::Operator` for the simplest case: send a single
//! prompt to a model and return the result. No tool use, no ReAct loop,
//! no hooks, no state reader. Used for classification, summarization,
//! extraction, and other single-inference tasks.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::duration::DurationMs;
use layer0::error::OperatorError;
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorMetadata, OperatorOutput};
use neuron_turn::convert::{content_to_user_message, parts_to_content};
use neuron_turn::provider::Provider;
use neuron_turn::types::*;
use rust_decimal::Decimal;
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
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let start = Instant::now();

        let model = self.resolve_model(&input);
        let system = self.resolve_system(&input);
        let max_tokens = self.config.default_max_tokens;

        // Build single user message
        let messages = vec![content_to_user_message(&input.message)];

        // Build request with no tools
        let request = ProviderRequest {
            model,
            messages,
            tools: vec![],
            max_tokens: Some(max_tokens),
            temperature: None,
            system: if system.is_empty() {
                None
            } else {
                Some(system)
            },
            extra: input.metadata.clone(),
        };

        // Single model call
        let response = self.provider.complete(request).await.map_err(|e| {
            if e.is_retryable() {
                OperatorError::Retryable(e.to_string())
            } else {
                OperatorError::Model(e.to_string())
            }
        })?;

        let duration = DurationMs::from(start.elapsed());

        // Build metadata
        let mut metadata = OperatorMetadata::default();
        metadata.tokens_in = response.usage.input_tokens;
        metadata.tokens_out = response.usage.output_tokens;
        metadata.cost = response.cost.unwrap_or(Decimal::ZERO);
        metadata.turns_used = 1;
        metadata.tools_called = vec![];
        metadata.duration = duration;

        // Convert response content to layer0 Content
        let message: Content = parts_to_content(&response.content);

        // Always ExitReason::Complete for single-shot
        let mut output = OperatorOutput::new(message, ExitReason::Complete);
        output.metadata = metadata;
        output.effects = vec![];

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_turn::provider::ProviderError;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // -- Mock Provider --

    struct MockProvider {
        responses: Mutex<VecDeque<Result<ProviderResponse, ProviderError>>>,
        requests: Mutex<Vec<ProviderRequest>>,
        call_count: AtomicUsize,
    }

    impl MockProvider {
        fn new(responses: Vec<ProviderResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(Ok).collect()),
                requests: Mutex::new(vec![]),
                call_count: AtomicUsize::new(0),
            }
        }

        fn with_error(error: ProviderError) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from([Err(error)])),
                requests: Mutex::new(vec![]),
                call_count: AtomicUsize::new(0),
            }
        }

        fn captured_requests(&self) -> Vec<ProviderRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Provider for MockProvider {
        fn complete(
            &self,
            request: ProviderRequest,
        ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send
        {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            let result = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("MockProvider: no more responses queued");
            async move { result }
        }
    }

    // -- Helpers --

    fn simple_text_response(text: &str) -> ProviderResponse {
        ProviderResponse {
            content: vec![ContentPart::Text {
                text: text.to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            model: "mock-model".into(),
            cost: Some(Decimal::new(1, 4)), // $0.0001
            truncated: None,
        }
    }

    fn simple_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), layer0::operator::TriggerType::User)
    }

    fn make_op(provider: MockProvider) -> SingleShotOperator<MockProvider> {
        SingleShotOperator::new(provider, SingleShotConfig::default())
    }

    // -- Tests --

    #[tokio::test]
    async fn single_shot_returns_completion() {
        let provider = MockProvider::new(vec![simple_text_response("Hello!")]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Hi")).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    #[tokio::test]
    async fn single_shot_always_one_turn() {
        let provider = MockProvider::new(vec![simple_text_response("Response")]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Query")).await.unwrap();

        assert_eq!(output.metadata.turns_used, 1);
    }

    #[tokio::test]
    async fn single_shot_no_tools_in_request() {
        let provider = MockProvider::new(vec![simple_text_response("Done")]);
        let op = make_op(provider);

        op.execute(simple_input("Test")).await.unwrap();

        let requests = op.provider.captured_requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].tools.is_empty(),
            "single-shot must send no tools"
        );
    }

    #[tokio::test]
    async fn single_shot_rate_limit_maps_to_retryable() {
        let provider = MockProvider::with_error(ProviderError::RateLimited);
        let op = make_op(provider);

        let result = op.execute(simple_input("test")).await;
        assert!(matches!(result, Err(OperatorError::Retryable(_))));
    }

    #[tokio::test]
    async fn single_shot_cost_passed_through() {
        let cost = Decimal::new(42, 4); // $0.0042
        let response = ProviderResponse {
            content: vec![ContentPart::Text {
                text: "result".to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
            model: "mock".into(),
            cost: Some(cost),
            truncated: None,
        };
        let provider = MockProvider::new(vec![response]);
        let op = make_op(provider);

        let output = op.execute(simple_input("test")).await.unwrap();

        assert_eq!(output.metadata.cost, cost);
        assert_eq!(output.metadata.tokens_in, 100);
        assert_eq!(output.metadata.tokens_out, 50);
    }

    #[tokio::test]
    async fn single_shot_as_arc_dyn_operator() {
        let provider = MockProvider::new(vec![simple_text_response("Hello!")]);
        let op: Arc<dyn Operator> = Arc::new(SingleShotOperator::new(
            provider,
            SingleShotConfig::default(),
        ));

        let output = op.execute(simple_input("Hi")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }
}
