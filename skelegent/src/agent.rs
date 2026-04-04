//! High-level agent builder — ergonomic entry point for skelegent.
//!
//! ```rust,ignore
//! use skelegent::prelude::*;
//!
//! let output = skelegent::agent("claude-sonnet-4-20250514")
//!     .system_prompt("You are helpful.")
//!     .build()?
//!     .execute(input, &ctx)
//!     .await?;
//!
//! // Or the one-liner:
//! let output = skelegent::agent("claude-sonnet-4-20250514")
//!     .system_prompt("You are helpful.")
//!     .run("Hello!")
//!     .await?;
//! ```

use layer0::DispatchContext;
use layer0::content::Content;
use layer0::effect::EffectKind;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{Operator, OperatorInput, OperatorOutput, TriggerType};
use skg_context_engine::ToolFilter;
use skg_tool::{ToolDyn, ToolRegistry};
use std::sync::Arc;

/// Create an agent builder with the given model identifier.
///
/// Model identifiers are resolved to providers:
/// - `"claude-*"` or `"anthropic:*"` → Anthropic (requires `ANTHROPIC_API_KEY`)
/// - `"gpt-*"` or `"openai:*"` → OpenAI (requires `OPENAI_API_KEY`)
/// - `"ollama:*"` → Ollama (local, no key needed)
///
/// # Example
///
/// ```rust,ignore
/// let output = skelegent::agent("claude-sonnet-4-20250514")
///     .system_prompt("You are helpful.")
///     .build()?
///     .execute(input, &ctx)
///     .await?;
/// ```
pub fn agent(model: &str) -> AgentBuilder {
    AgentBuilder {
        model: model.to_string(),
        system: None,
        tools: ToolRegistry::new(),
        max_turns: None,
        max_tokens: None,
        temperature: None,
        max_tool_retries: None,
        tool_filter: None,
    }
}

/// Builder for constructing and running an agent.
///
/// Returned by [`agent()`]. All configuration methods are chainable. Call
/// [`build()`](Self::build) to produce a type-erased [`Operator`], or call
/// [`run()`](Self::run) as a one-shot convenience that skips the build step.
pub struct AgentBuilder {
    model: String,
    system: Option<String>,
    tools: ToolRegistry,
    max_turns: Option<u32>,
    max_tokens: Option<u32>,
    /// Sampling temperature forwarded to [`ReactLoopConfig::temperature`].
    temperature: Option<f64>,
    /// Tool retry budget forwarded to [`ReactLoopConfig::max_tool_retries`].
    max_tool_retries: Option<u32>,
    /// Per-turn tool filter forwarded to [`ReactLoopConfig::tool_filter`].
    tool_filter: Option<ToolFilter>,
}

impl AgentBuilder {
    /// Set the system prompt.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    /// Set the tool registry, replacing any previously registered tools.
    ///
    /// Use [`.tool()`] to add individual tools without replacing the entire registry.
    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    /// Register a single tool into the agent's tool registry.
    ///
    /// Adds to any tools already registered via [`.tools()`]. Calling `.tool()` and
    /// `.tools()` in any order composes — the last write for a given tool name wins.
    pub fn tool(mut self, tool: Arc<dyn ToolDyn>) -> Self {
        self.tools.register(tool);
        self
    }

    /// Set the maximum number of turns.
    pub fn max_turns(mut self, max: u32) -> Self {
        self.max_turns = Some(max);
        self
    }

    /// Set the maximum tokens per response.
    pub fn max_tokens(mut self, max: u32) -> Self {
        self.max_tokens = Some(max);
        self
    }

    /// Set the sampling temperature passed to the model.
    ///
    /// Typical range is `0.0` (deterministic) to `1.0` (more creative). The
    /// acceptable range depends on the provider; values outside the valid range
    /// are forwarded as-is and the provider will return an error.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the maximum number of times to retry a tool call that fails with
    /// [`skg_tool::ToolError::InvalidInput`].
    ///
    /// When a tool rejects its input, the model receives the error and the tool's
    /// schema so it can correct the call. Each call ID tracks its own retry budget
    /// independently. Defaults to `2` when not set.
    pub fn max_tool_retries(mut self, retries: u32) -> Self {
        self.max_tool_retries = Some(retries);
        self
    }

    /// Set a per-turn tool filter predicate.
    ///
    /// Called before each inference turn. Tools for which the predicate returns
    /// `false` are hidden from the model for that turn but remain in the registry
    /// for dispatch (in case the model references a tool that was visible in a
    /// prior turn and the response is still in-flight).
    pub fn tool_filter(mut self, filter: ToolFilter) -> Self {
        self.tool_filter = Some(filter);
        self
    }

    // TODO: .on_event(impl Fn(DispatchEvent) + Send + Sync + 'static) — store a
    // streaming event callback for future use. Skipped until DispatchEvent is
    // stabilized and the streaming pipeline supports per-operator callbacks.

    /// Build the agent, resolving the model string to a provider.
    ///
    /// The model string is preserved in [`ReactLoopConfig::model`] so the
    /// provider uses the exact model version requested. Returns a type-erased
    /// [`Operator`] that can be executed with `.execute(input, &ctx)` or
    /// registered with a [`Dispatcher`](layer0::Dispatcher).
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - The model string doesn't match any known provider prefix
    /// - The required API key environment variable is not set
    /// - The provider feature is not enabled
    pub fn build(self) -> Result<Box<dyn Operator>, AgentBuildError> {
        resolve_model(self)
    }

    /// Run the agent with a user message. Convenience for `build()?.execute()`.
    ///
    /// Creates a minimal [`DispatchContext`] internally. For full control over
    /// dispatch context or to reuse the operator across multiple calls, use
    /// [`build()`](Self::build) and call `.execute()` directly.
    pub async fn run(self, message: &str) -> Result<OperatorOutput, ProtocolError> {
        let op = self
            .build()
            .map_err(|e| ProtocolError::internal(e.to_string()))?;
        let input = OperatorInput::new(Content::text(message), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("agent"), OperatorId::new("agent"));
        let output = op.execute(input, &ctx).await?;
        reject_operational_effects(&output.effects)?;
        // Observational effects (Log, Signal, etc.) are safe to drop.
        Ok(output)
    }
}

/// Inspect effects and return `Err` for any that mutate external state.
///
/// Operational effects (`WriteMemory`, `DeleteMemory`, `Delegate`, `Handoff`)
/// cannot be silently dropped — doing so produces plausible-looking but wrong
/// output. The caller must use [`OrchestratedRunner`] to handle them.
/// Observational effects (`Log`, `Signal`, `Observation`, etc.) are advisory
/// and safe to drop.
fn reject_operational_effects(effects: &[layer0::Effect]) -> Result<(), ProtocolError> {
    let operational = effects.iter().any(|e| {
        matches!(
            &e.kind,
            EffectKind::WriteMemory { .. }
                | EffectKind::DeleteMemory { .. }
                | EffectKind::Delegate { .. }
                | EffectKind::Handoff { .. }
        )
    });
    if operational {
        return Err(ProtocolError::internal(
            "AgentBuilder::run() cannot execute operational effects \
             (WriteMemory, DeleteMemory, Delegate, Handoff). \
             Use OrchestratedRunner instead.",
        ));
    }
    Ok(())
}

/// Error building an agent from a model string.
#[derive(Debug)]
pub enum AgentBuildError {
    /// The model string didn't match any known provider pattern.
    UnknownModel(String),
    /// The required API key environment variable is not set.
    MissingApiKey {
        /// The environment variable that was expected.
        env_var: &'static str,
    },
    /// The required provider feature is not enabled.
    FeatureNotEnabled {
        /// The feature that needs to be enabled.
        feature: &'static str,
    },
}

impl std::fmt::Display for AgentBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownModel(m) => write!(
                f,
                "Unknown model: {m}. Expected claude-*, gpt-*, openai:*, anthropic:*, or ollama:*"
            ),
            Self::MissingApiKey { env_var } => write!(f, "Missing API key: set {env_var}"),
            Self::FeatureNotEnabled { feature } => write!(f, "Feature not enabled: {feature}"),
        }
    }
}

impl std::error::Error for AgentBuildError {}

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
))]
fn resolve_model(builder: AgentBuilder) -> Result<Box<dyn Operator>, AgentBuildError> {
    use skg_context_engine::{CognitiveBuilder, ReactLoopConfig};

    // Preserve model string so the provider sends the exact version requested.
    // Previously this was None (model dropped after provider selection — the bug).
    let config = ReactLoopConfig {
        system_prompt: builder.system.unwrap_or_default(),
        model: Some(builder.model.clone()),
        max_tokens: Some(builder.max_tokens.unwrap_or(4096)),
        temperature: builder.temperature,
        max_tool_retries: builder.max_tool_retries.unwrap_or(2),
        tool_filter: builder.tool_filter,
        ..Default::default()
    };
    let max_turns = builder.max_turns.unwrap_or(10);
    let tools = builder.tools;
    let model = builder.model.as_str();

    let op: Box<dyn Operator> = if model.starts_with("claude-") || model.starts_with("anthropic:") {
        #[cfg(feature = "provider-anthropic")]
        {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| AgentBuildError::MissingApiKey {
                    env_var: "ANTHROPIC_API_KEY",
                })?;
            let provider = skg_provider_anthropic::AnthropicProvider::new(api_key);
            Box::new(
                CognitiveBuilder::new()
                    .config(config)
                    .tools(tools)
                    .max_turns(max_turns)
                    .provider(provider)
                    .build(),
            )
        }
        #[cfg(not(feature = "provider-anthropic"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-anthropic",
            });
        }
    } else if model.starts_with("gpt-")
        || model.starts_with("openai:")
        || model.starts_with("o1-")
        || model.starts_with("o3-")
    {
        #[cfg(feature = "provider-openai")]
        {
            let api_key =
                std::env::var("OPENAI_API_KEY").map_err(|_| AgentBuildError::MissingApiKey {
                    env_var: "OPENAI_API_KEY",
                })?;
            let provider = skg_provider_openai::OpenAIProvider::new(api_key);
            Box::new(
                CognitiveBuilder::new()
                    .config(config)
                    .tools(tools)
                    .max_turns(max_turns)
                    .provider(provider)
                    .build(),
            )
        }
        #[cfg(not(feature = "provider-openai"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-openai",
            });
        }
    } else if model.starts_with("ollama:") {
        #[cfg(feature = "provider-ollama")]
        {
            let provider = skg_provider_ollama::OllamaProvider::new();
            Box::new(
                CognitiveBuilder::new()
                    .config(config)
                    .tools(tools)
                    .max_turns(max_turns)
                    .provider(provider)
                    .build(),
            )
        }
        #[cfg(not(feature = "provider-ollama"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-ollama",
            });
        }
    } else {
        return Err(AgentBuildError::UnknownModel(model.to_string()));
    };

    Ok(op)
}

#[cfg(not(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
)))]
fn resolve_model(builder: AgentBuilder) -> Result<Box<dyn Operator>, AgentBuildError> {
    let model = builder.model.as_str();

    if model.starts_with("claude-") || model.starts_with("anthropic:") {
        return Err(AgentBuildError::FeatureNotEnabled {
            feature: "provider-anthropic",
        });
    }

    if model.starts_with("gpt-")
        || model.starts_with("openai:")
        || model.starts_with("o1-")
        || model.starts_with("o3-")
    {
        return Err(AgentBuildError::FeatureNotEnabled {
            feature: "provider-openai",
        });
    }

    if model.starts_with("ollama:") {
        return Err(AgentBuildError::FeatureNotEnabled {
            feature: "provider-ollama",
        });
    }

    Err(AgentBuildError::UnknownModel(model.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::operator::{Outcome, TerminalOutcome};
    use skg_context_engine::{
        CognitiveBuilder, CognitiveOperator, ReactLoopConfig,
        rules::{BudgetGuard, BudgetGuardConfig},
    };
    use skg_turn::test_utils::{FunctionProvider, make_text_response};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Build a FunctionProvider that counts calls and returns a fixed text response.
    fn counting_provider(text: &str) -> (impl skg_turn::provider::Provider, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let count_inner = count.clone();
        let response = make_text_response(text);
        let provider = FunctionProvider::new(move |_req| {
            count_inner.fetch_add(1, Ordering::Relaxed);
            Ok(response.clone())
        });
        (provider, count)
    }

    #[tokio::test]
    async fn cognitive_operator_budget_exit_halts() {
        let (provider, call_count) = counting_provider("unused");

        let op = CognitiveOperator::new(
            "test",
            provider,
            ToolRegistry::new(),
            ReactLoopConfig {
                system_prompt: "system".into(),
                ..Default::default()
            },
        )
        .with_rules(|| {
            let guard = BudgetGuard::with_config(BudgetGuardConfig {
                max_cost: None,
                max_turns: Some(0),
                max_duration: None,
                max_tool_calls: None,
            });
            // BeforeAny fires during inject_system/inject_message context.run() calls.
            vec![skg_context_engine::rule::Rule::new(
                "budget_guard",
                skg_context_engine::rule::Trigger::BeforeAny,
                100,
                guard,
            )]
        });

        let input = OperatorInput::new(Content::text("hi"), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let result = op.execute(input, &ctx).await;

        // Budget guard fires before first inference, surfacing as an error.
        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn cognitive_operator_execute_returns_complete() {
        let (provider, call_count) = counting_provider("done");
        let op = CognitiveOperator::new(
            "test",
            provider,
            ToolRegistry::new(),
            ReactLoopConfig {
                system_prompt: "You are helpful.".into(),
                ..Default::default()
            },
        );

        let input = OperatorInput::new(Content::text("Hello!"), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let output = op.execute(input, &ctx).await.unwrap();
        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }

    /// Verify that temperature set on [`ReactLoopConfig`] is forwarded to the provider's
    /// [`InferRequest`]. We don't test `AgentBuilder::temperature()` end-to-end (that would
    /// require a real provider feature), but we verify the config→request path is correct.
    #[tokio::test]
    async fn agent_builder_temperature() {
        use skg_turn::infer::InferRequest;
        use std::sync::Mutex;

        let captured: Arc<Mutex<Option<InferRequest>>> = Arc::new(Mutex::new(None));
        let captured_inner = captured.clone();
        let provider = FunctionProvider::new(move |req| {
            *captured_inner.lock().unwrap() = Some(req);
            Ok(make_text_response("done"))
        });

        let op = CognitiveOperator::new(
            "test",
            provider,
            ToolRegistry::new(),
            ReactLoopConfig {
                system_prompt: "system".into(),
                temperature: Some(0.7),
                ..Default::default()
            },
        );

        let input = OperatorInput::new(Content::text("Hello"), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let _ = op.execute(input, &ctx).await.unwrap();

        let guard = captured.lock().unwrap();
        let req = guard.as_ref().expect("provider was not called");
        assert_eq!(
            req.temperature,
            Some(0.7),
            "temperature must be forwarded to InferRequest"
        );
    }

    /// Verify that a tool registered via [`AgentBuilder::tool()`] (or directly on a
    /// [`ToolRegistry`]) is included in the [`InferRequest`] schema list sent to the model.
    #[tokio::test]
    async fn agent_builder_single_tool() {
        use layer0::DispatchContext as DC;
        use skg_tool::ToolError;
        use std::pin::Pin;

        struct PingTool;

        impl ToolDyn for PingTool {
            fn name(&self) -> &str {
                "ping"
            }
            fn description(&self) -> &str {
                "pings"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object" })
            }
            fn call(
                &self,
                _input: serde_json::Value,
                _ctx: &DC,
            ) -> Pin<
                Box<
                    dyn std::future::Future<Output = Result<serde_json::Value, ToolError>>
                        + Send
                        + '_,
                >,
            > {
                Box::pin(async { Ok(serde_json::json!("pong")) })
            }
        }

        let captured: Arc<std::sync::Mutex<Option<skg_turn::infer::InferRequest>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_inner = captured.clone();
        let provider = FunctionProvider::new(move |req| {
            *captured_inner.lock().unwrap() = Some(req);
            Ok(make_text_response("done"))
        });

        // Register the tool via the registry directly (mirrors what .tool() does).
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(PingTool));

        let op = CognitiveOperator::new(
            "test",
            provider,
            registry,
            ReactLoopConfig {
                system_prompt: "system".into(),
                ..Default::default()
            },
        );

        let input = OperatorInput::new(Content::text("Hello"), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let _ = op.execute(input, &ctx).await.unwrap();

        let guard = captured.lock().unwrap();
        let req = guard.as_ref().expect("provider was not called");
        assert!(
            req.tools.iter().any(|t| t.name == "ping"),
            "tool 'ping' must appear in InferRequest.tools; got: {:?}",
            req.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
        );
    }

    /// Verify that the model string set in [`ReactLoopConfig::model`] is forwarded to
    /// [`InferRequest::model`]. This is the path that `AgentBuilder::build()` uses —
    /// the model string must survive from `agent("model")` through to the provider call.
    #[tokio::test]
    async fn agent_preserves_model() {
        use skg_turn::infer::InferRequest;
        use std::sync::Mutex;

        let captured: Arc<Mutex<Option<InferRequest>>> = Arc::new(Mutex::new(None));
        let captured_inner = captured.clone();
        let provider = FunctionProvider::new(move |req| {
            *captured_inner.lock().unwrap() = Some(req);
            Ok(make_text_response("done"))
        });

        let op = CognitiveOperator::new(
            "test",
            provider,
            ToolRegistry::new(),
            ReactLoopConfig {
                system_prompt: "sys".into(),
                model: Some("claude-sonnet-4-20250514".to_string()),
                ..Default::default()
            },
        );

        let input = OperatorInput::new(Content::text("hi"), TriggerType::User);
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let _ = op.execute(input, &ctx).await.unwrap();

        let guard = captured.lock().unwrap();
        let req = guard.as_ref().expect("provider was not called");
        assert_eq!(
            req.model.as_deref(),
            Some("claude-sonnet-4-20250514"),
            "model must be forwarded to InferRequest; was None (model-dropping bug)",
        );
    }

    /// Verify that [`CognitiveBuilder::run()`] works as an end-to-end convenience shortcut:
    /// build + execute in one call.
    #[tokio::test]
    async fn agent_run_convenience() {
        let (provider, call_count) = counting_provider("Hello from run!");

        let output = CognitiveBuilder::new()
            .system_prompt("You are helpful.")
            .provider(provider)
            .run("Hi!")
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }
    // ── Effect classification tests ────────────────────────────────────────────────
    //
    // These tests exercise reject_operational_effects(), which is the exact
    // function that AgentBuilder::run() delegates to after execute().

    #[test]
    fn run_rejects_operational_effects() {
        use layer0::Effect;
        use layer0::effect::{EffectKind, MemoryScope, Scope};
        use serde_json::json;

        // WriteMemory is an operational effect — run() must return Err.
        let effects = vec![Effect::new(EffectKind::WriteMemory {
            scope: Scope::Global,
            key: "state-key".into(),
            value: json!({"x": 1}),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        })];
        let result = super::reject_operational_effects(&effects);
        assert!(
            result.is_err(),
            "WriteMemory is an operational effect; run() must reject it"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("OrchestratedRunner"),
            "error must direct caller to OrchestratedRunner, got: {msg}"
        );
    }

    #[test]
    fn run_allows_observational_effects() {
        use layer0::Effect;
        use layer0::effect::EffectKind;

        // Log is observational — run() must not error on observational-only effects.
        let effects = vec![Effect::new(EffectKind::Log {
            level: "info".into(),
            message: "agent completed successfully".into(),
        })];
        let result = super::reject_operational_effects(&effects);
        assert!(
            result.is_ok(),
            "Log is observational; run() must not reject it"
        );
    }
}
