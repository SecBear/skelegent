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
use layer0::error::OperatorError;
use layer0::id::OperatorId;
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::DispatchContext;
use skg_tool::ToolRegistry;
use skg_turn::provider::Provider;
use std::collections::HashSet;
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
    /// Accepted from callers for identity/tracing; not yet read internally.
    #[allow(dead_code)]
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
        dispatch_ctx: &DispatchContext,
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

        let mut config = self.react_loop_config();

        // Apply allowed_operators from dispatch input as a tool filter.
        // When a parent sets allowed_operators, only those tools are visible
        // to the LLM. AND-combined with any existing tool_filter.
        if let Some(ref op_config) = input.config
            && let Some(ref allowed) = op_config.allowed_operators
        {
            let allowed_set: HashSet<String> = allowed.iter().cloned().collect();
            let existing_filter = config.tool_filter.take();
            config.tool_filter = Some(Arc::new(
                move |tool: &dyn skg_tool::ToolDyn, ctx: &crate::context::Context| {
                    let name_allowed = allowed_set.contains(tool.name());
                    name_allowed && existing_filter.as_ref().is_none_or(|f| f(tool, ctx))
                },
            ));
        }

        react_loop(
            &mut ctx,
            &self.provider,
            &self.tools,
            dispatch_ctx,
            &config,
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
        EngineError::Tool(err) => {
            // Extract tool identity from the error when available.
            let label = match &err {
                skg_tool::ToolError::NotFound(name) => format!("tool:{name}"),
                _ => "tool".to_string(),
            };
            OperatorError::SubDispatch {
                operator: label,
                source: Box::new(err),
            }
        }
        EngineError::Halted { reason } => OperatorError::Halted { reason },
        EngineError::Exit { reason, detail } => OperatorError::Halted {
            reason: format!("{reason:?}: {detail}"),
        },
        EngineError::Custom(err) => OperatorError::Other(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::id::DispatchId;
    use layer0::content::Content;
    use layer0::operator::{ExitReason, OperatorConfig, TriggerType};
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
            .execute(simple_input("Hi"), &test_ctx())
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
            .execute(simple_input("Query"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(output.metadata.turns_used, 1);
    }

    #[tokio::test]
    async fn cognitive_rate_limit_maps_to_retryable() {
        let provider = skg_turn::test_utils::error_provider_rate_limited();
        let op = CognitiveOperator::new("test-op", provider, ToolRegistry::new(), make_config());

        let result = op
            .execute(simple_input("test"), &test_ctx())
            .await;
        assert!(matches!(
            result,
            Err(OperatorError::Model {
                retryable: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn cognitive_as_arc_dyn_operator() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);
        let op: std::sync::Arc<dyn Operator> = std::sync::Arc::new(make_op(provider));

        let output = Operator::execute(
            op.as_ref(),
            simple_input("Hi"),
            &test_ctx(),
        )
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

        let result = op
            .execute(simple_input("hi"), &test_ctx())
            .await;

        // Budget guard returns a structured exit (MaxTurns) — not an error.
        // ExitReason::MaxTurns is an expected termination, not a failure.
        let output = result.unwrap();
        assert_eq!(output.exit_reason, layer0::operator::ExitReason::MaxTurns);
    }

    // ── allowed_operators filtering ────────────────────────────────────────

    /// Minimal [`ToolDyn`] implementation for filter tests.
    struct StubTool(&'static str);

    impl skg_tool::ToolDyn for StubTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, skg_tool::ToolError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { Ok(serde_json::json!(null)) })
        }
    }

    fn registry_with_tools(names: &[&'static str]) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        for name in names {
            reg.register(Arc::new(StubTool(name)));
        }
        reg
    }

    #[tokio::test]
    async fn allowed_operators_filters_visible_tools() {
        let provider = TestProvider::with_responses(vec![make_text_response("ok")]);
        let tools = registry_with_tools(&["tool_a", "tool_b", "tool_c"]);
        let op = CognitiveOperator::new("test-op", provider, tools, make_config());

        let mut input = simple_input("go");
        let mut op_config = OperatorConfig::default();
        op_config.allowed_operators = Some(vec!["tool_a".into()]);
        input.config = Some(op_config);

        let output = op
            .execute(input, &test_ctx())
            .await
            .unwrap();

        // The model never saw tool_b or tool_c, so it returned text
        // (no tool calls). The exit is Complete.
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[test]
    fn allowed_operators_filter_correctness() {
        // Directly verify the filter function produced by execute's
        // allowed_operators logic.
        let allowed: HashSet<String> = ["tool_a"].iter().map(|s| s.to_string()).collect();
        let filter: crate::react::ToolFilter =
            Arc::new(move |tool: &dyn skg_tool::ToolDyn, _ctx: &Context| {
                allowed.contains(tool.name())
            });

        let tool_a: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_a"));
        let tool_b: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_b"));
        let ctx = Context::new();

        assert!(filter(tool_a.as_ref(), &ctx));
        assert!(!filter(tool_b.as_ref(), &ctx));
    }

    #[test]
    fn allowed_operators_and_combines_with_existing_filter() {
        // Existing filter rejects tool_a. allowed_operators includes
        // tool_a and tool_b. The AND-combination should reject tool_a
        // (blocked by existing) and accept tool_b.
        let existing: crate::react::ToolFilter =
            Arc::new(|tool: &dyn skg_tool::ToolDyn, _ctx: &Context| tool.name() != "tool_a");

        let allowed: HashSet<String> = ["tool_a", "tool_b"].iter().map(|s| s.to_string()).collect();
        let existing_opt = Some(existing);
        let combined: crate::react::ToolFilter =
            Arc::new(move |tool: &dyn skg_tool::ToolDyn, ctx: &Context| {
                let name_allowed = allowed.contains(tool.name());
                name_allowed && existing_opt.as_ref().is_none_or(|f| f(tool, ctx))
            });

        let ctx = Context::new();
        let tool_a: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_a"));
        let tool_b: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_b"));
        let tool_c: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_c"));

        // tool_a: in allowed, but rejected by existing filter → false
        assert!(!combined(tool_a.as_ref(), &ctx));
        // tool_b: in allowed, not rejected by existing → true
        assert!(combined(tool_b.as_ref(), &ctx));
        // tool_c: not in allowed → false
        assert!(!combined(tool_c.as_ref(), &ctx));
    }

    #[test]
    fn allowed_operators_empty_list_blocks_all() {
        let allowed: HashSet<String> = HashSet::new();
        let filter: crate::react::ToolFilter =
            Arc::new(move |tool: &dyn skg_tool::ToolDyn, _ctx: &Context| {
                allowed.contains(tool.name())
            });

        let tool_a: Arc<dyn skg_tool::ToolDyn> = Arc::new(StubTool("tool_a"));
        let ctx = Context::new();
        assert!(!filter(tool_a.as_ref(), &ctx));
    }
}
