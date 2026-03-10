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
use layer0::context::Role;
use layer0::error::OperatorError;
use skg_context_engine::{
    AssemblyExt, Context, ReactLoopConfig, react_loop, rule::Trigger, rules::BudgetGuard,
};
use skg_tool::ToolRegistry;

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
    model: String,
    system_prompt: String,
    tools: ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
    provider: ProviderKind,
}

impl BuiltAgent {
    /// Run the agent with a user message.
    pub async fn run(
        &self,
        message: &str,
    ) -> Result<layer0::operator::OperatorOutput, OperatorError> {
        // Create context with budget guard rule
        let guard = BudgetGuard::with_config(skg_context_engine::rules::BudgetGuardConfig {
            max_cost: None,
            max_turns: Some(self.max_turns),
            max_duration: None,
            max_tool_calls: None,
        });
        let rule =
            skg_context_engine::rule::Rule::new("budget_guard", Trigger::BeforeAny, 100, guard);
        let mut ctx = Context::with_rules(vec![rule]);

        // Inject system prompt and user message
        ctx.inject_system(&self.system_prompt)
            .await
            .map_err(|_| OperatorError::InferenceError("Failed to inject system prompt".into()))?;
        let user_msg = layer0::context::Message::new(Role::User, Content::text(message));
        ctx.inject_message(user_msg)
            .await
            .map_err(|_| OperatorError::InferenceError("Failed to inject user message".into()))?;

        // Run the react loop
        let config = ReactLoopConfig {
            system_prompt: self.system_prompt.clone(),
            model: Some(self.model.clone()),
            max_tokens: Some(self.max_tokens),
            temperature: None,
            tool_filter: None,
        };

        self.provider
            .react_loop(&mut ctx, &self.tools, &config)
            .await
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

/// Enum to hold different provider types (since Provider is RPITIT and not object-safe).
enum ProviderKind {
    #[cfg(feature = "provider-anthropic")]
    Anthropic(skg_provider_anthropic::AnthropicProvider),
    #[cfg(feature = "provider-openai")]
    OpenAI(skg_provider_openai::OpenAIProvider),
    #[cfg(feature = "provider-ollama")]
    Ollama(skg_provider_ollama::OllamaProvider),
}

impl ProviderKind {
    /// Call react_loop with the appropriate provider.
    async fn react_loop(
        &self,
        ctx: &mut Context,
        tools: &ToolRegistry,
        config: &ReactLoopConfig,
    ) -> Result<layer0::operator::OperatorOutput, OperatorError> {
        match self {
            #[cfg(feature = "provider-anthropic")]
            Self::Anthropic(provider) => {
                skg_context_engine::react_loop(ctx, provider, tools, config)
                    .await
                    .map_err(|e| {
                        OperatorError::InferenceError(format!("Context engine error: {e}"))
                    })
            }
            #[cfg(feature = "provider-openai")]
            Self::OpenAI(provider) => {
                skg_context_engine::react_loop(ctx, provider, tools, config)
                    .await
                    .map_err(|e| {
                        OperatorError::InferenceError(format!("Context engine error: {e}"))
                    })
            }
            #[cfg(feature = "provider-ollama")]
            Self::Ollama(provider) => {
                skg_context_engine::react_loop(ctx, provider, tools, config)
                    .await
                    .map_err(|e| {
                        OperatorError::InferenceError(format!("Context engine error: {e}"))
                    })
            }
        }
    }
}

fn resolve_model(
    model: &str,
    system_prompt: String,
    tools: ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
) -> Result<BuiltAgent, AgentBuildError> {
    let provider = if model.starts_with("claude-") || model.starts_with("anthropic:") {
        #[cfg(feature = "provider-anthropic")]
        {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| AgentBuildError::MissingApiKey {
                    env_var: "ANTHROPIC_API_KEY",
                })?;
            let provider = skg_provider_anthropic::AnthropicProvider::new(api_key);
            ProviderKind::Anthropic(provider)
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
            ProviderKind::OpenAI(provider)
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
            ProviderKind::Ollama(provider)
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

    Ok(BuiltAgent {
        model: model.to_string(),
        system_prompt,
        tools,
        max_turns,
        max_tokens,
        provider,
    })
}
