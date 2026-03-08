//! High-level agent builder — ergonomic entry point for neuron.
//!
//! ```rust,ignore
//! use neuron::prelude::*;
//!
//! let output = neuron::agent("claude-sonnet-4-20250514")
//!     .system("You are helpful.")
//!     .build()?
//!     .run("Hello!")
//!     .await?;
//! ```

use layer0::content::Content;
use layer0::error::OperatorError;
use layer0::operator::{Operator, OperatorInput, OperatorOutput, TriggerType};
#[allow(unused_imports)]
use neuron_op_react::{ReactConfig, ReactOperator};
use neuron_tool::ToolRegistry;
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
/// let output = neuron::agent("claude-sonnet-4-20250514")
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
        let config = ReactConfig {
            default_model: self.model.clone(),
            system_prompt: self.system.unwrap_or_default(),
            default_max_turns: self.max_turns.unwrap_or(10),
            default_max_tokens: self.max_tokens.unwrap_or(4096),
            ..ReactConfig::default()
        };

        let state: Arc<dyn layer0::StateReader> =
            Arc::new(neuron_state_memory::MemoryStore::new());

        let operator: Box<dyn Operator> = resolve_model(&self.model, self.tools, state, config)?;

        Ok(BuiltAgent { operator })
    }
}

/// A fully constructed agent ready to run.
pub struct BuiltAgent {
    operator: Box<dyn Operator>,
}

impl BuiltAgent {
    /// Run the agent with a user message.
    pub async fn run(&self, message: &str) -> Result<OperatorOutput, OperatorError> {
        let input = OperatorInput::new(Content::text(message), TriggerType::Task);
        self.operator.execute(input).await
    }

    /// Run the agent with a fully constructed [`OperatorInput`].
    pub async fn run_input(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        self.operator.execute(input).await
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
            Self::UnknownModel(m) => write!(f, "Unknown model: {m}. Expected claude-*, gpt-*, openai:*, anthropic:*, or ollama:*"),
            Self::MissingApiKey { env_var } => write!(f, "Missing API key: set {env_var}"),
            Self::FeatureNotEnabled { feature } => write!(f, "Feature not enabled: {feature}"),
        }
    }
}

impl std::error::Error for AgentBuildError {}

#[allow(unused_variables)]
fn resolve_model(
    model: &str,
    tools: ToolRegistry,
    state: Arc<dyn layer0::StateReader>,
    config: ReactConfig,
) -> Result<Box<dyn Operator>, AgentBuildError> {
    if model.starts_with("claude-") || model.starts_with("anthropic:") {
        #[cfg(feature = "provider-anthropic")]
        {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| AgentBuildError::MissingApiKey {
                    env_var: "ANTHROPIC_API_KEY",
                })?;
            let provider = neuron_provider_anthropic::AnthropicProvider::new(api_key);
            let op = ReactOperator::new(provider, tools, state, config);
            return Ok(Box::new(op));
        }
        #[cfg(not(feature = "provider-anthropic"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-anthropic",
            });
        }
    }

    if model.starts_with("gpt-") || model.starts_with("openai:") || model.starts_with("o1-") || model.starts_with("o3-") {
        #[cfg(feature = "provider-openai")]
        {
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                AgentBuildError::MissingApiKey {
                    env_var: "OPENAI_API_KEY",
                }
            })?;
            let provider = neuron_provider_openai::OpenAIProvider::new(api_key);
            let op = ReactOperator::new(provider, tools, state, config);
            return Ok(Box::new(op));
        }
        #[cfg(not(feature = "provider-openai"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-openai",
            });
        }
    }

    if model.starts_with("ollama:") {
        #[cfg(feature = "provider-ollama")]
        {
            let provider = neuron_provider_ollama::OllamaProvider::new();
            let op = ReactOperator::new(provider, tools, state, config);
            return Ok(Box::new(op));
        }
        #[cfg(not(feature = "provider-ollama"))]
        {
            return Err(AgentBuildError::FeatureNotEnabled {
                feature: "provider-ollama",
            });
        }
    }

    Err(AgentBuildError::UnknownModel(model.to_string()))
}
