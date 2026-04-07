//! Typestate builder for [`AgentOperator`].
//!
//! Provides an ergonomic fluent API for constructing an [`AgentOperator`]
//! with compile-time enforcement that a provider is supplied before building.
//!
//! # Usage
//!
//! ```rust,ignore
//! use skg_context_engine::{AgentBuilder, AgentOperator, ReactLoopConfig};
//! use skg_tool::ToolRegistry;
//!
//! let op = AgentBuilder::new()
//!     .system_prompt("You are a helpful assistant.")
//!     .max_tokens(2048)
//!     .provider(my_provider)
//!     .build();
//! // op: AgentOperator<MyProvider>
//!
//! // AgentOperator::builder() also works when P can be inferred:
//! let op: AgentOperator<MyProvider> = AgentOperator::builder()
//!     .provider(my_provider)
//!     .build();
//! ```
//!
//! The provider step is required — `build()` is only available on
//! `AgentBuilder<WithProvider<P>>`:
//!
//! ```rust,compile_fail
//! # use skg_context_engine::builder::AgentBuilder;
//! // This fails to compile: build() does not exist on AgentBuilder<NoProvider>.
//! let op = AgentBuilder::new().build();
//! ```

use std::sync::Arc;

use layer0::Dispatcher;
use skg_tool::{ToolDyn, ToolRegistry};
use skg_turn::provider::Provider;

use crate::AgentOperator;
use crate::pipeline::Pipeline;
use crate::runtime::{BudgetGuard, BudgetGuardConfig, ReactLoopConfig};

// ── PipelineFactory type ──────────────────────────────────────────────────────

/// Factory that produces a fresh [`Pipeline`] for each execution.
///
/// Pipelines contain `Box<dyn ErasedMiddleware>` and cannot be cloned, so
/// the operator needs a factory to create fresh instances per `execute()` call.
pub type PipelineFactory = Arc<dyn Fn() -> Pipeline + Send + Sync>;

// ── Typestate markers ─────────────────────────────────────────────────────────

/// Typestate marker: no provider has been set yet.
///
/// An `AgentBuilder<NoProvider>` does not expose [`build()`](AgentBuilder::build);
/// that method is only available after calling [`.provider()`](AgentBuilder::provider).
pub struct NoProvider;

/// Typestate marker: a provider of type `P` has been set.
///
/// When the builder is in this state, [`build()`](AgentBuilder::build) is available.
pub struct WithProvider<P>(P);

// ── Builder struct ────────────────────────────────────────────────────────────

/// Typestate builder for [`AgentOperator`].
///
/// Construct via [`AgentOperator::builder()`] or [`AgentBuilder::new()`].
/// Configure fluently, then supply a provider with [`.provider()`] and call
/// [`.build()`] to obtain an [`AgentOperator`].
///
/// `max_turns` is enforced via a [`BudgetGuard`] middleware injected into the
/// before-send phase at build time. Any pipeline supplied via [`.pipeline()`]
/// has the budget guard prepended (fires first).
pub struct AgentBuilder<State> {
    state: State,
    operator_id: Option<String>,
    /// Standalone system prompt; overrides `config.system_prompt` when non-empty.
    system_prompt: String,
    tools: ToolRegistry,
    config: ReactLoopConfig,
    pipeline_factory: Option<PipelineFactory>,
    /// Stored separately because `ReactLoopConfig` has no `max_turns` field;
    /// wired into a `BudgetGuard` middleware at [`build()`](AgentBuilder::build) time.
    max_turns: Option<u32>,
    /// Optional dispatcher forwarded to [`AgentOperator::with_dispatcher`] at
    /// build time. Construction-time capability, not per-request data.
    dispatcher: Option<Arc<dyn Dispatcher>>,
}

// ── NoProvider state ──────────────────────────────────────────────────────────

impl AgentBuilder<NoProvider> {
    /// Create a builder with no provider set.
    ///
    /// Equivalent to [`AgentOperator::builder()`].
    pub fn new() -> Self {
        Self {
            state: NoProvider,
            operator_id: None,
            system_prompt: String::new(),
            tools: ToolRegistry::new(),
            config: ReactLoopConfig::default(),
            pipeline_factory: None,
            max_turns: None,
            dispatcher: None,
        }
    }

    /// Supply the LLM provider, advancing the builder to [`WithProvider`] state.
    ///
    /// After this call, [`build()`](AgentBuilder::build) becomes available.
    pub fn provider<P: Provider + 'static>(self, p: P) -> AgentBuilder<WithProvider<P>> {
        AgentBuilder {
            state: WithProvider(p),
            operator_id: self.operator_id,
            system_prompt: self.system_prompt,
            tools: self.tools,
            config: self.config,
            pipeline_factory: self.pipeline_factory,
            max_turns: self.max_turns,
            dispatcher: self.dispatcher,
        }
    }
}

impl Default for AgentBuilder<NoProvider> {
    fn default() -> Self {
        Self::new()
    }
}

// ── WithProvider state ────────────────────────────────────────────────────────

impl<P: Provider + 'static> AgentBuilder<WithProvider<P>> {
    /// Finalize the builder and produce an [`AgentOperator`].
    ///
    /// This method is only available on `AgentBuilder<WithProvider<P>>`.
    /// Attempting to call it on `AgentBuilder<NoProvider>` is a compile error:
    ///
    /// ```rust,compile_fail
    /// # use skg_context_engine::builder::AgentBuilder;
    /// // build() does not exist on AgentBuilder<NoProvider>
    /// let _op = AgentBuilder::new().build();
    /// ```
    ///
    /// The system prompt set via [`.system_prompt()`] takes precedence over
    /// `config.system_prompt` when non-empty.
    ///
    /// When `max_turns` was set via [`.max_turns()`], a [`BudgetGuard`]
    /// middleware is prepended to the before-send phase of the pipeline.
    /// Any pipeline supplied via [`.pipeline()`] has the guard prepended so
    /// it always fires first.
    pub fn build(self) -> AgentOperator<P> {
        let operator_id = self.operator_id.unwrap_or_else(|| "agent".into());

        // system_prompt wins over whatever is in config when non-empty, so
        // callers can do .config(full_config).system_prompt("override") safely.
        let mut config = self.config;
        if !self.system_prompt.is_empty() {
            config.system_prompt = self.system_prompt;
        }

        let pipeline_factory: Option<PipelineFactory> =
            match (self.max_turns, self.pipeline_factory) {
                (Some(max_turns), Some(user_factory)) => Some(Arc::new(move || {
                    let mut pipeline = user_factory();
                    // Budget guard fires first (prepended); user middleware follows.
                    pipeline.before_send.insert(
                        0,
                        Box::new(BudgetGuard::with_config(BudgetGuardConfig {
                            max_turns: Some(max_turns),
                            ..Default::default()
                        })),
                    );
                    pipeline
                })),
                (Some(max_turns), None) => Some(Arc::new(move || {
                    let mut pipeline = Pipeline::new();
                    pipeline.push_before(Box::new(BudgetGuard::with_config(BudgetGuardConfig {
                        max_turns: Some(max_turns),
                        ..Default::default()
                    })));
                    pipeline
                })),
                (None, Some(factory)) => Some(Arc::new(move || factory())),
                // No pipeline configured — AgentOperator::new() behavior.
                (None, None) => None,
            };

        let op = AgentOperator::new(operator_id, self.state.0, self.tools, config);

        // Wire in pipeline factory and optional dispatcher.
        let op = match pipeline_factory {
            Some(f) => op.with_pipeline(move || f()),
            None => op,
        };
        match self.dispatcher {
            Some(d) => op.with_dispatcher(d),
            None => op,
        }
    }

    /// Run the agent with a user message. Convenience for `build().execute(input, ctx)`.
    ///
    /// Creates a minimal [`layer0::DispatchContext`] internally. For full control over
    /// dispatch context or to reuse the operator across multiple calls, call
    /// [`build()`](Self::build) and invoke `.execute()` directly.
    pub async fn run(
        self,
        message: &str,
    ) -> Result<layer0::operator::OperatorOutput, layer0::error::ProtocolError> {
        use layer0::DispatchContext;
        use layer0::content::Content;
        use layer0::id::{DispatchId, OperatorId};
        use layer0::operator::{Operator, OperatorInput, TriggerType};

        let op = self.build();
        let input = OperatorInput::new(Content::text(message), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("agent"), OperatorId::new("agent"));
        let output = op.execute(input, &ctx).await?;
        if output.has_unhandled_intents() {
            eprintln!(
                "warning: OperatorOutput contains {} intent(s) that will not be executed. \
                 Use an EffectHandler or OrchestratedRunner to process intents.",
                output.intents.len(),
            );
        }
        Ok(output)
    }
}

// ── Shared setters (both states) ──────────────────────────────────────────────

impl<S> AgentBuilder<S> {
    /// Set the operator ID used in dispatch traces and tool-call metadata.
    ///
    /// Defaults to `"agent"` when not set.
    pub fn operator_id(mut self, id: impl Into<String>) -> Self {
        self.operator_id = Some(id.into());
        self
    }

    /// Set the system prompt.
    ///
    /// When non-empty, this overrides `config.system_prompt` at build time.
    /// Equivalent to calling `.config(ReactLoopConfig { system_prompt: ..., ..Default::default() })`
    /// but composable with [`.config()`] — this value always wins.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Replace the entire tool registry.
    ///
    /// Use [`.tool()`] to add individual tools without replacing the registry.
    pub fn tools(mut self, registry: ToolRegistry) -> Self {
        self.tools = registry;
        self
    }

    /// Register a single tool, adding it to the existing registry.
    ///
    /// Composable with [`.tools()`]: last registration for a given name wins.
    pub fn tool(mut self, tool: Arc<dyn ToolDyn>) -> Self {
        self.tools.register(tool);
        self
    }

    /// Set the maximum number of ReAct turns.
    ///
    /// Enforced via a [`BudgetGuard`] middleware injected before each inference
    /// call. When combined with [`.pipeline()`], the budget guard fires first.
    pub fn max_turns(mut self, max: u32) -> Self {
        self.max_turns = Some(max);
        self
    }

    /// Set the maximum output tokens per inference call.
    ///
    /// Maps to [`ReactLoopConfig::max_tokens`].
    pub fn max_tokens(mut self, max: u32) -> Self {
        self.config.max_tokens = Some(max);
        self
    }

    /// Set the sampling temperature forwarded to the provider.
    ///
    /// Maps to [`ReactLoopConfig::temperature`].
    pub fn temperature(mut self, temp: f64) -> Self {
        self.config.temperature = Some(temp);
        self
    }

    /// Set the pipeline factory for injecting middleware into each execution context.
    ///
    /// The factory is called once per `execute()` invocation because pipelines
    /// contain boxed trait objects and are not `Clone`. When [`.max_turns()`]
    /// is also set, a [`BudgetGuard`] middleware is prepended to the
    /// before-send phase and user middleware follows.
    pub fn pipeline(mut self, factory: PipelineFactory) -> Self {
        self.pipeline_factory = Some(factory);
        self
    }

    /// Replace the entire [`ReactLoopConfig`].
    ///
    /// Useful for callers that already have a pre-built config. Settings
    /// applied via individual builder methods after this call take precedence:
    /// [`.system_prompt()`], [`.max_tokens()`], [`.temperature()`] all write
    /// into `self.config` or override it at build time.
    pub fn config(mut self, config: ReactLoopConfig) -> Self {
        self.config = config;
        self
    }

    /// Attach a dispatcher for sub-dispatch from within the react loop.
    ///
    /// Forwarded to [`AgentOperator::with_dispatcher`] at build time.
    /// The dispatcher is injected into the `DispatchContext` extensions so
    /// downstream tools can access it via
    /// `dispatch_ctx.extensions().get::<Arc<dyn Dispatcher>>()`.
    ///
    /// Construction-time capability — not per-request data.
    pub fn dispatcher(mut self, dispatcher: Arc<dyn Dispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{OperatorInput, Outcome, TerminalOutcome, TriggerType};
    use layer0::{DispatchContext, operator::Operator};
    use skg_turn::test_utils::{TestProvider, make_text_response};

    fn test_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-op"))
    }

    fn simple_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), TriggerType::User)
    }

    /// Builder produces a working operator when given a provider, and executes it.
    #[tokio::test]
    async fn builder_creates_operator() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);

        let op = AgentBuilder::new()
            .operator_id("test-op")
            .system_prompt("You are helpful.")
            .max_tokens(1024)
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("Hi"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    /// Builder with only a provider uses the same defaults as
    /// `AgentOperator::new("agent", provider, ToolRegistry::new(), ReactLoopConfig::default())`.
    #[tokio::test]
    async fn builder_defaults_sensible() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hi")]);

        // Minimal build: just provider, everything else is default.
        let op = AgentBuilder::new().provider(provider).build();

        let output = Operator::execute(&op, simple_input("Hello"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
    }

    /// `max_turns` wires a `BudgetGuard` that exits cleanly after the limit.
    #[tokio::test]
    async fn builder_max_turns_enforced() {
        // Two responses but limit is 1 — the first completes normally; the guard
        // fires before any second inference call, which is never reached.
        let provider = TestProvider::with_responses(vec![
            make_text_response("First"),
            make_text_response("Second"),
        ]);

        let op = AgentBuilder::new()
            .system_prompt("You are helpful.")
            .max_turns(1)
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("Hello"), &test_ctx())
            .await
            .unwrap();

        // One-turn model response completes normally; guard fires before a second call.
        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
    }

    /// [`AgentBuilder::run()`] is a one-shot convenience: build + execute with a
    /// synthesized dispatch context.
    #[tokio::test]
    async fn builder_run_convenience() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello from run!")]);

        let output = AgentBuilder::new()
            .system_prompt("You are helpful.")
            .provider(provider)
            .run("Hi!")
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
        assert_eq!(output.message.as_text().unwrap(), "Hello from run!");
    }

    // ── dispatcher builder wiring ────────────────────────────────────────

    /// Minimal dispatcher that panics if dispatch is ever called.
    ///
    /// Used to verify that supplying a dispatcher via the builder does not
    /// break normal text-only execution paths.
    struct NullDispatcher;

    #[async_trait::async_trait]
    impl Dispatcher for NullDispatcher {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            _input: OperatorInput,
        ) -> Result<layer0::DispatchHandle, layer0::error::ProtocolError> {
            panic!("NullDispatcher::dispatch called — not expected");
        }
    }

    /// Builder with `.dispatcher()` still produces a working operator.
    #[tokio::test]
    async fn builder_dispatcher_executes_normally() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hello!")]);

        let op = AgentBuilder::new()
            .system_prompt("You are helpful.")
            .dispatcher(Arc::new(NullDispatcher))
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("Hi"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    /// `.dispatcher()` is usable before `.provider()` (shared setter, both states).
    #[tokio::test]
    async fn builder_dispatcher_before_provider() {
        let provider = TestProvider::with_responses(vec![make_text_response("ok")]);

        let op = AgentBuilder::new()
            .dispatcher(Arc::new(NullDispatcher)) // set before provider
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("test"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
    }

    /// Dispatcher chains correctly with max_turns (pipeline wiring).
    #[tokio::test]
    async fn builder_dispatcher_with_max_turns() {
        let provider = TestProvider::with_responses(vec![make_text_response("done")]);

        let op = AgentBuilder::new()
            .max_turns(5)
            .dispatcher(Arc::new(NullDispatcher))
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("hi"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
    }
}
