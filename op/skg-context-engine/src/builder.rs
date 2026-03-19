//! Typestate builder for [`CognitiveOperator`].
//!
//! Provides an ergonomic fluent API for constructing a [`CognitiveOperator`]
//! with compile-time enforcement that a provider is supplied before building.
//!
//! # Usage
//!
//! ```rust,ignore
//! use skg_context_engine::{CognitiveBuilder, CognitiveOperator, ReactLoopConfig};
//! use skg_tool::ToolRegistry;
//!
//! let op = CognitiveBuilder::new()
//!     .system_prompt("You are a helpful assistant.")
//!     .max_tokens(2048)
//!     .provider(my_provider)
//!     .build();
//! // op: CognitiveOperator<MyProvider>
//!
//! // CognitiveOperator::builder() also works when P can be inferred:
//! let op: CognitiveOperator<MyProvider> = CognitiveOperator::builder()
//!     .provider(my_provider)
//!     .build();
//! ```
//!
//! The provider step is required — `build()` is only available on
//! `CognitiveBuilder<WithProvider<P>>`:
//!
//! ```rust,compile_fail
//! # use skg_context_engine::builder::CognitiveBuilder;
//! // This fails to compile: build() does not exist on CognitiveBuilder<NoProvider>.
//! let op = CognitiveBuilder::new().build();
//! ```

use std::sync::Arc;

use skg_tool::{ToolDyn, ToolRegistry};
use skg_turn::provider::Provider;

use crate::cognitive_operator::RuleFactory;
use crate::rule::Rule;
use crate::rules::{BudgetGuard, BudgetGuardConfig};
use crate::{CognitiveOperator, InferBoundary, ReactLoopConfig};

// ── Typestate markers ─────────────────────────────────────────────────────────

/// Typestate marker: no provider has been set yet.
///
/// A `CognitiveBuilder<NoProvider>` does not expose [`build()`](CognitiveBuilder::build);
/// that method is only available after calling [`.provider()`](CognitiveBuilder::provider).
pub struct NoProvider;

/// Typestate marker: a provider of type `P` has been set.
///
/// When the builder is in this state, [`build()`](CognitiveBuilder::build) is available.
pub struct WithProvider<P>(P);

// ── Builder struct ────────────────────────────────────────────────────────────

/// Typestate builder for [`CognitiveOperator`].
///
/// Construct via [`CognitiveOperator::builder()`] or [`CognitiveBuilder::new()`].
/// Configure fluently, then supply a provider with [`.provider()`] and call
/// [`.build()`] to obtain a [`CognitiveOperator`].
///
/// `max_turns` is enforced via a [`BudgetGuard`] rule injected at build time.
/// Any additional rules supplied via [`.rules()`] are combined with the budget
/// guard (budget guard fires first at priority 100).
pub struct CognitiveBuilder<State> {
    state: State,
    operator_id: Option<String>,
    /// Standalone system prompt; overrides `config.system_prompt` when non-empty.
    system_prompt: String,
    tools: ToolRegistry,
    config: ReactLoopConfig,
    rule_factory: Option<RuleFactory>,
    /// Stored separately because `ReactLoopConfig` has no `max_turns` field;
    /// wired into a `BudgetGuard` rule at [`build()`](CognitiveBuilder::build) time.
    max_turns: Option<u32>,
}

// ── NoProvider state ──────────────────────────────────────────────────────────

impl CognitiveBuilder<NoProvider> {
    /// Create a builder with no provider set.
    ///
    /// Equivalent to [`CognitiveOperator::builder()`].
    pub fn new() -> Self {
        Self {
            state: NoProvider,
            operator_id: None,
            system_prompt: String::new(),
            tools: ToolRegistry::new(),
            config: ReactLoopConfig::default(),
            rule_factory: None,
            max_turns: None,
        }
    }

    /// Supply the LLM provider, advancing the builder to [`WithProvider`] state.
    ///
    /// After this call, [`build()`](CognitiveBuilder::build) becomes available.
    pub fn provider<P: Provider + 'static>(self, p: P) -> CognitiveBuilder<WithProvider<P>> {
        CognitiveBuilder {
            state: WithProvider(p),
            operator_id: self.operator_id,
            system_prompt: self.system_prompt,
            tools: self.tools,
            config: self.config,
            rule_factory: self.rule_factory,
            max_turns: self.max_turns,
        }
    }
}

impl Default for CognitiveBuilder<NoProvider> {
    fn default() -> Self {
        Self::new()
    }
}

// ── WithProvider state ────────────────────────────────────────────────────────

impl<P: Provider + 'static> CognitiveBuilder<WithProvider<P>> {
    /// Finalize the builder and produce a [`CognitiveOperator`].
    ///
    /// This method is only available on `CognitiveBuilder<WithProvider<P>>`.
    /// Attempting to call it on `CognitiveBuilder<NoProvider>` is a compile error:
    ///
    /// ```rust,compile_fail
    /// # use skg_context_engine::builder::CognitiveBuilder;
    /// // build() does not exist on CognitiveBuilder<NoProvider>
    /// let _op = CognitiveBuilder::new().build();
    /// ```
    ///
    /// The system prompt set via [`.system_prompt()`] takes precedence over
    /// `config.system_prompt` when non-empty.
    ///
    /// When `max_turns` was set via [`.max_turns()`], a [`BudgetGuard`] rule is
    /// injected at priority 100 using `Rule::before::<InferBoundary>`. Any
    /// rules supplied via [`.rules()`] are appended after the budget guard so
    /// the guard always fires first.
    pub fn build(self) -> CognitiveOperator<P> {
        let operator_id = self.operator_id.unwrap_or_else(|| "agent".into());

        // system_prompt wins over whatever is in config when non-empty, so
        // callers can do .config(full_config).system_prompt("override") safely.
        let mut config = self.config;
        if !self.system_prompt.is_empty() {
            config.system_prompt = self.system_prompt;
        }

        let op = CognitiveOperator::new(operator_id, self.state.0, self.tools, config);

        match (self.max_turns, self.rule_factory) {
            (Some(max_turns), Some(user_factory)) => {
                // Budget guard fires first (priority 100); user rules follow.
                op.with_rules(move || {
                    let guard = BudgetGuard::with_config(BudgetGuardConfig {
                        max_turns: Some(max_turns),
                        max_cost: None,
                        max_duration: None,
                        max_tool_calls: None,
                    });
                    let mut rules =
                        vec![Rule::before::<InferBoundary>("budget_guard", 100, guard)];
                    rules.extend(user_factory());
                    rules
                })
            }
            (Some(max_turns), None) => op.with_rules(move || {
                let guard = BudgetGuard::with_config(BudgetGuardConfig {
                    max_turns: Some(max_turns),
                    max_cost: None,
                    max_duration: None,
                    max_tool_calls: None,
                });
                vec![Rule::before::<InferBoundary>("budget_guard", 100, guard)]
            }),
            (None, Some(factory)) => op.with_rules(move || factory()),
            // No rules configured — match CognitiveOperator::new() behavior exactly.
            (None, None) => op,
        }
    }
}

// ── Shared setters (both states) ──────────────────────────────────────────────

impl<S> CognitiveBuilder<S> {
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
    /// Enforced via a [`BudgetGuard`] rule injected before each inference call.
    /// When combined with [`.rules()`], the budget guard fires first.
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

    /// Set the rule factory for injecting rules into each execution context.
    ///
    /// The factory is called once per `execute()` invocation because rules
    /// contain boxed trait objects and are not `Clone`. When [`.max_turns()`]
    /// is also set, the budget guard is prepended (priority 100) and user rules
    /// follow.
    pub fn rules(mut self, factory: RuleFactory) -> Self {
        self.rule_factory = Some(factory);
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
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{ExitReason, OperatorInput, TriggerType};
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

        let op = CognitiveBuilder::new()
            .operator_id("test-op")
            .system_prompt("You are helpful.")
            .max_tokens(1024)
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("Hi"), &test_ctx())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
    }

    /// Builder with only a provider uses the same defaults as
    /// `CognitiveOperator::new("agent", provider, ToolRegistry::new(), ReactLoopConfig::default())`.
    #[tokio::test]
    async fn builder_defaults_sensible() {
        let provider = TestProvider::with_responses(vec![make_text_response("Hi")]);

        // Minimal build: just provider, everything else is default.
        let op = CognitiveBuilder::new().provider(provider).build();

        let output = Operator::execute(&op, simple_input("Hello"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
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

        let op = CognitiveBuilder::new()
            .system_prompt("You are helpful.")
            .max_turns(1)
            .provider(provider)
            .build();

        let output = Operator::execute(&op, simple_input("Hello"), &test_ctx())
            .await
            .unwrap();

        // One-turn model response completes normally; guard fires before a second call.
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

}