//! High-level agent builder — ergonomic entry point for skelegent.
//!
//! ```rust,ignore
//! use skelegent::prelude::*;
//!
//! let output = skelegent::agent("claude-sonnet-4-20250514")
//!     .system("You are helpful.")
//!     .build()?
//!     .run("Hello!")
//!     .await?;
//! ```

use layer0::content::Content;
use layer0::dispatch::EffectEmitter;
use layer0::error::OperatorError;
use layer0::operator::{Operator, OperatorInput, OperatorOutput, TriggerType};
use skg_tool::ToolRegistry;
#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
))]
use skg_turn::provider::Provider;

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
///     .system("You are helpful.")
///     .build()?
///     .run("Hello!")
///     .await?;
/// ```
pub fn agent(model: &str) -> AgentBuilder {
    AgentBuilder {
        model: model.to_string(),
        system: None,
        tools: ToolRegistry::new(),
        max_turns: None,
        max_tokens: None,
    }
}

/// Builder for constructing and running an agent.
pub struct AgentBuilder {
    model: String,
    system: Option<String>,
    tools: ToolRegistry,
    max_turns: Option<u32>,
    max_tokens: Option<u32>,
}

impl AgentBuilder {
    /// Set the system prompt.
    pub fn system(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    /// Set the tool registry.
    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
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

    /// Build the agent, resolving the model string to a provider.
    ///
    /// Returns a [`BuiltAgent`] that can be run with `.run()`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - The model string doesn't match any known provider prefix
    /// - The required API key environment variable is not set
    /// - The provider feature is not enabled
    pub fn build(self) -> Result<BuiltAgent, AgentBuildError> {
        resolve_model(
            &self.model,
            self.system.unwrap_or_default(),
            self.tools,
            self.max_turns.unwrap_or(10),
            self.max_tokens.unwrap_or(4096),
        )
    }
}

/// A fully constructed agent ready to run.
pub struct BuiltAgent {
    operator: OperatorBox,
}

impl BuiltAgent {
    /// Run the agent with a user message.
    pub async fn run(&self, message: &str) -> Result<OperatorOutput, OperatorError> {
        let input = OperatorInput::new(Content::text(message), TriggerType::User);
        let output = self.operator.execute(input, &EffectEmitter::noop()).await?;
        if output.has_unhandled_effects() {
            eprintln!(
                "warning: OperatorOutput contains {} effect(s) that will not be executed. \
                 Use an EffectInterpreter or OrchestratedRunner to process effects.",
                output.effects.len(),
            );
        }
        Ok(output)
    }
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

/// Type-erased operator box — wraps `CognitiveOperator<P>` for any provider.
///
/// This exists because `Provider` is RPITIT (not object-safe), so we can't
/// use `Box<dyn Operator>` directly through the provider enum. Instead,
/// we construct the concrete `CognitiveOperator<P>` during `build()` and
/// erase it behind `Box<dyn Operator>`.
type OperatorBox = Box<dyn Operator>;

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
))]
/// Build a `CognitiveOperator` from resolved provider and config.
fn build_cognitive_operator<P: Provider + 'static>(
    provider: P,
    system_prompt: String,
    tools: ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
) -> Box<dyn Operator> {
    use skg_context_engine::{
        CognitiveOperator, CognitiveOperatorConfig, InferBoundary, Rule,
        rules::{BudgetGuard, BudgetGuardConfig},
    };

    let op = CognitiveOperator::new(
        "agent",
        provider,
        tools,
        CognitiveOperatorConfig {
            system_prompt,
            model: None, // model already selected by provider
            max_tokens: Some(max_tokens),
            ..Default::default()
        },
    )
    .with_rules(move || {
        let guard = BudgetGuard::with_config(BudgetGuardConfig {
            max_cost: None,
            max_turns: Some(max_turns),
            max_duration: None,
            max_tool_calls: None,
        });
        vec![Rule::before::<InferBoundary>("budget_guard", 100, guard)]
    });
    Box::new(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skg_context_engine::{
        CognitiveOperator, CognitiveOperatorConfig, InferBoundary, Rule,
        rules::{BudgetGuard, BudgetGuardConfig},
    };
    use skg_turn::infer::{InferRequest, InferResponse};
    use skg_turn::provider::{Provider, ProviderError};
    use skg_turn::types::{StopReason, TokenUsage};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Default)]
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    impl CountingProvider {
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Provider for CountingProvider {
        fn infer(
            &self,
            _request: InferRequest,
        ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(InferResponse {
                    content: Content::text("done"),
                    tool_calls: vec![],
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                    model: "test".into(),
                    cost: None,
                    truncated: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn built_agent_budget_exit_returns_structured_output() {
        let provider = CountingProvider::default();

        let op = CognitiveOperator::new(
            "test",
            provider.clone(),
            ToolRegistry::new(),
            CognitiveOperatorConfig {
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
            vec![Rule::before::<InferBoundary>("budget_guard", 100, guard)]
        });

        let input = OperatorInput::new(Content::text("hi"), TriggerType::User);
        let output = op
            .execute(input, &EffectEmitter::noop())
            .await
            .expect("budget exits should return operator output");

        assert_eq!(output.exit_reason, layer0::operator::ExitReason::MaxTurns);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn built_agent_run_returns_complete() {
        let provider = CountingProvider::default();
        let op: Box<dyn Operator> = Box::new(CognitiveOperator::new(
            "test",
            provider.clone(),
            ToolRegistry::new(),
            CognitiveOperatorConfig {
                system_prompt: "You are helpful.".into(),
                ..Default::default()
            },
        ));
        let agent = BuiltAgent { operator: op };

        let output = agent.run("Hello!").await.unwrap();
        assert_eq!(output.exit_reason, layer0::operator::ExitReason::Complete);
        assert_eq!(provider.call_count(), 1);
    }
}

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
))]
fn resolve_model(
    model: &str,
    system_prompt: String,
    tools: ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
) -> Result<BuiltAgent, AgentBuildError> {
    let operator: Box<dyn Operator> = if model.starts_with("claude-")
        || model.starts_with("anthropic:")
    {
        #[cfg(feature = "provider-anthropic")]
        {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| AgentBuildError::MissingApiKey {
                    env_var: "ANTHROPIC_API_KEY",
                })?;
            let provider = skg_provider_anthropic::AnthropicProvider::new(api_key);
            build_cognitive_operator(provider, system_prompt, tools, max_turns, max_tokens)
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
            build_cognitive_operator(provider, system_prompt, tools, max_turns, max_tokens)
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
            build_cognitive_operator(provider, system_prompt, tools, max_turns, max_tokens)
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

    Ok(BuiltAgent { operator })
}
