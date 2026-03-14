//! [`CognitiveOperator`] — wraps [`react_loop`](crate::react_loop) as an [`Operator`].
//!
//! This is the canonical bridge between the context engine's free-function
//! composition model and layer0's object-safe `Operator` trait. Rather than
//! every consumer writing the same boilerplate wrapper, this operator provides:
//!
//! - Proper `EngineError` → `OperatorError` mapping (classified, not `to_string()`)
//! - `Context` creation with optional rule injection
//! - Structured exit handling (`ExitReason::MaxTurns`, etc.)
//!
//! Generic over `P: Provider` (not object-safe). The object-safe boundary
//! is `Operator`, which `CognitiveOperator<P>` implements via `#[async_trait]`.

use async_trait::async_trait;
use layer0::context::{Message, Role};
use layer0::dispatch::EffectEmitter;
use layer0::error::OperatorError;
use layer0::id::OperatorId;
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::{DispatchContext, DispatchId};
use skg_tool::ToolRegistry;
use skg_turn::provider::Provider;
use std::sync::Arc;

use crate::context::Context;
use crate::error::EngineError;
use crate::react::ReactLoopConfig;
use crate::{Rule, react_loop};

/// Factory that produces rules for each execution.
///
/// Rules contain `Box<dyn ErasedOp>` and are not `Clone`, so the operator
/// needs a factory to create fresh rules for each `execute()` call.
pub type RuleFactory = Arc<dyn Fn() -> Vec<Rule> + Send + Sync>;

/// Configuration for [`CognitiveOperator`].
pub struct CognitiveOperatorConfig {
    /// System prompt baked into every turn.
    pub system_prompt: String,
    /// Model identifier (e.g. `"claude-sonnet-4-20250514"`).
    pub model: Option<String>,
    /// Max output tokens per inference call.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Per-turn tool filter.
    pub tool_filter: Option<crate::react::ToolFilter>,
}

impl Default for CognitiveOperatorConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            model: None,
            max_tokens: Some(4096),
            temperature: None,
            tool_filter: None,
        }
    }
}

/// An [`Operator`] that runs a ReAct loop via the context engine.
///
/// Wraps [`react_loop()`](crate::react_loop) with proper error classification,
/// context setup, and rule injection. This is the operator you register with
/// a `Dispatcher` when you want a multi-turn, tool-using agent.
///
/// # Example
///
/// ```rust,ignore
/// use skg_context_engine::{CognitiveOperator, CognitiveOperatorConfig};
///
/// let op = CognitiveOperator::new("agent", provider, tools, config);
/// // Register with a dispatcher:
/// let orch = LocalOrch::new();
/// orch.register("agent", Arc::new(op));
/// ```
pub struct CognitiveOperator<P: Provider> {
    provider: P,
    tools: ToolRegistry,
    operator_id: OperatorId,
    config: CognitiveOperatorConfig,
    /// Factory for rules injected into each execution context.
    rule_factory: Option<RuleFactory>,
}

impl<P: Provider> CognitiveOperator<P> {
    /// Create a new cognitive operator.
    ///
    /// `operator_id` identifies this operator in dispatch traces and tool
    /// call metadata.
    pub fn new(
        operator_id: impl Into<OperatorId>,
        provider: P,
        tools: ToolRegistry,
        config: CognitiveOperatorConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            operator_id: operator_id.into(),
            config,
            rule_factory: None,
        }
    }

    /// Set a rule factory that produces rules for each execution.
    ///
    /// Rules fire automatically during context operations. Common rules:
    /// - `BudgetGuard` — enforces turn/cost/duration limits
    /// - Overwatch agents — monitor and intervene
    /// - Telemetry recorders
    ///
    /// The factory is called once per `execute()` call because rules are
    /// not `Clone` (they contain boxed trait objects).
    pub fn with_rules(mut self, factory: impl Fn() -> Vec<Rule> + Send + Sync + 'static) -> Self {
        self.rule_factory = Some(Arc::new(factory));
        self
    }

    fn react_loop_config(&self) -> ReactLoopConfig {
        ReactLoopConfig {
            system_prompt: self.config.system_prompt.clone(),
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            tool_filter: self.config.tool_filter.clone(),
        }
    }

    fn create_context(&self) -> Context {
        match &self.rule_factory {
            Some(factory) => Context::with_rules(factory()),
            None => Context::new(),
        }
    }
}

#[async_trait]
impl<P: Provider + 'static> Operator for CognitiveOperator<P> {
    #[tracing::instrument(skip_all, fields(trigger = ?input.trigger))]
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
        emitter: &EffectEmitter,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut ctx = self.create_context();

        // Inject system prompt into context
        if !self.config.system_prompt.is_empty() {
            ctx.inject_system(&self.config.system_prompt)
                .await
                .map_err(OperatorError::context_assembly)?;
        }

        // Inject user message
        ctx.inject_message(Message::new(Role::User, input.message))
            .await
            .map_err(OperatorError::context_assembly)?;

        let config = self.react_loop_config();

        // Construct a DispatchContext for this execution.
        // When Operator::execute receives &DispatchContext from the
        // dispatcher (Phase 3), this construction moves to the caller.
        let dispatch_ctx = DispatchContext::new(
            DispatchId::new(format!(
                "cogop-{}-{}",
                self.operator_id,
                ctx.metrics.turns_completed
            )),
            self.operator_id.clone(),
        );

        react_loop(
            &mut ctx,
            &self.provider,
            &self.tools,
            &dispatch_ctx,
            &config,
            emitter,
        )
        .await
        .map_err(map_engine_error)
    }
}

/// Classified mapping from [`EngineError`] to [`OperatorError`].
///
/// Preserves error semantics: retryable provider errors stay retryable,
/// operator errors pass through, tool errors become sub-dispatch errors.
pub fn map_engine_error(err: EngineError) -> OperatorError {
    match err {
        EngineError::Provider(err) => {
            if err.is_retryable() {
                OperatorError::model_retryable(err)
            } else {
                OperatorError::Model {
                    source: Box::new(err),
                    retryable: false,
                }
            }
        }
        EngineError::Operator(err) => err,
        EngineError::Tool(err) => OperatorError::SubDispatch {
            operator: "tool".into(),
            source: Box::new(err),
        },
        EngineError::Halted { reason } => OperatorError::Halted { reason },
        EngineError::Custom(err) => OperatorError::Other(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::operator::{ExitReason, TriggerType};
    use skg_turn::test_utils::{TestProvider, make_text_response};

    fn test_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-op"))
    }

    fn simple_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), TriggerType::User)
    }

    fn make_config() -> CognitiveOperatorConfig {
        CognitiveOperatorConfig {
            system_prompt: "You are helpful.".into(),
            model: Some("test-model".into()),
            ..Default::default()
        }
    }

    fn make_op(provider: TestProvider) -> CognitiveOperator<TestProvider> {
        CognitiveOperator::new("test-op", provider, ToolRegistry::new(), make_config())
    }

    #[tokio::test]
    async fn cognitive_returns_complete_on_end_turn() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);
        let op = make_op(provider);

        let output = op
            .execute(simple_input("Hi"), &test_ctx(), &EffectEmitter::noop())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    #[tokio::test]
    async fn cognitive_tracks_token_metadata() {
        let provider = TestProvider::with_responses(vec![make_text_response("Response")]);
        let op = make_op(provider);

        let output = op
            .execute(simple_input("Query"), &test_ctx(), &EffectEmitter::noop())
            .await
            .unwrap();

        assert_eq!(output.metadata.turns_used, 1);
    }

    #[tokio::test]
    async fn cognitive_rate_limit_maps_to_retryable() {
        let provider = skg_turn::test_utils::error_provider_rate_limited();
        let op = CognitiveOperator::new("test-op", provider, ToolRegistry::new(), make_config());

        let result = op
            .execute(simple_input("test"), &test_ctx(), &EffectEmitter::noop())
            .await;
        assert!(matches!(result, Err(OperatorError::Model { retryable: true, .. })));
    }

    #[tokio::test]
    async fn cognitive_as_arc_dyn_operator() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);
        let op: std::sync::Arc<dyn Operator> = std::sync::Arc::new(make_op(provider));

        let output = Operator::execute(op.as_ref(), simple_input("Hi"), &test_ctx(), &EffectEmitter::noop())
            .await
            .unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[tokio::test]
    async fn cognitive_with_budget_guard() {
        use crate::rules::{BudgetGuard, BudgetGuardConfig};

        let provider = TestProvider::with_responses(vec![make_text_response("Done")]);

        let op = CognitiveOperator::new("test-op", provider, ToolRegistry::new(), make_config())
            .with_rules(|| {
                let guard = BudgetGuard::with_config(BudgetGuardConfig {
                    max_cost: None,
                    max_turns: Some(0), // Zero turns = immediate budget exit
                    max_duration: None,
                    max_tool_calls: None,
                });
                // Before<InferBoundary> fires before each inference call in react_loop.
                vec![Rule::before::<crate::boundary::InferBoundary>(
                    "budget_guard",
                    100,
                    guard,
                )]
            });

        let result = op.execute(simple_input("hi"), &test_ctx(), &EffectEmitter::noop()).await;

        // Budget guard halts before first inference via Before<InferBoundary>.
        assert!(result.is_err());
    }
}
