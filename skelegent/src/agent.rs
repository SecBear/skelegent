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
use layer0::context::{Message, Role};
use layer0::error::OperatorError;
use layer0::id::OperatorId;
use layer0::operator::OperatorOutput;
use skg_context_engine::{
    Context, EngineError, InferBoundary, ReactLoopConfig, Rule, react_loop,
    rules::{BudgetGuard, BudgetGuardConfig},
};
use skg_tool::{ToolCallContext, ToolRegistry};
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
    model: String,
    system_prompt: String,
    tools: ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
    provider: ProviderKind,
}

impl BuiltAgent {
    /// Run the agent with a user message.
    pub async fn run(&self, message: &str) -> Result<OperatorOutput, OperatorError> {
        self.provider
            .run(
                &self.model,
                &self.system_prompt,
                &self.tools,
                self.max_turns,
                self.max_tokens,
                message,
            )
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

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
))]
impl ProviderKind {
    async fn run(
        &self,
        model: &str,
        system_prompt: &str,
        tools: &ToolRegistry,
        max_turns: u32,
        max_tokens: u32,
        message: &str,
    ) -> Result<OperatorOutput, OperatorError> {
        match self {
            #[cfg(feature = "provider-anthropic")]
            Self::Anthropic(provider) => {
                run_with_provider(
                    model,
                    system_prompt,
                    tools,
                    max_turns,
                    max_tokens,
                    provider,
                    message,
                )
                .await
            }
            #[cfg(feature = "provider-openai")]
            Self::OpenAI(provider) => {
                run_with_provider(
                    model,
                    system_prompt,
                    tools,
                    max_turns,
                    max_tokens,
                    provider,
                    message,
                )
                .await
            }
            #[cfg(feature = "provider-ollama")]
            Self::Ollama(provider) => {
                run_with_provider(
                    model,
                    system_prompt,
                    tools,
                    max_turns,
                    max_tokens,
                    provider,
                    message,
                )
                .await
            }
        }
    }
}

#[cfg(not(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
)))]
impl ProviderKind {
    async fn run(
        &self,
        _model: &str,
        _system_prompt: &str,
        _tools: &ToolRegistry,
        _max_turns: u32,
        _max_tokens: u32,
        _message: &str,
    ) -> Result<OperatorOutput, OperatorError> {
        match *self {}
    }
}

async fn run_with_provider<P: Provider>(
    model: &str,
    system_prompt: &str,
    tools: &ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
    provider: &P,
    message: &str,
) -> Result<OperatorOutput, OperatorError> {
    let guard = BudgetGuard::with_config(BudgetGuardConfig {
        max_cost: None,
        max_turns: Some(max_turns),
        max_duration: None,
        max_tool_calls: None,
    });
    let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
        "budget_guard",
        100,
        guard,
    )]);

    ctx.inject_system(system_prompt)
        .await
        .map_err(|e| OperatorError::ContextAssembly(e.to_string()))?;
    let user_msg = Message::new(Role::User, Content::text(message));
    ctx.inject_message(user_msg)
        .await
        .map_err(|e| OperatorError::ContextAssembly(e.to_string()))?;

    let config = ReactLoopConfig {
        system_prompt: system_prompt.to_string(),
        model: Some(model.to_string()),
        max_tokens: Some(max_tokens),
        temperature: None,
        tool_filter: None,
    };
    let tool_ctx = ToolCallContext::new(OperatorId::from("agent"));

    react_loop(&mut ctx, provider, tools, &tool_ctx, &config)
        .await
        .map_err(map_engine_error)
}

fn map_engine_error(err: EngineError) -> OperatorError {
    match err {
        EngineError::Provider(err) => {
            if err.is_retryable() {
                OperatorError::Retryable(err.to_string())
            } else {
                OperatorError::Model(err.to_string())
            }
        }
        EngineError::Operator(err) => err,
        EngineError::Tool(err) => OperatorError::SubDispatch {
            operator: "tool".into(),
            message: err.to_string(),
        },
        EngineError::Halted { reason } => OperatorError::NonRetryable(reason),
        EngineError::Exit { detail, .. } => OperatorError::NonRetryable(detail),
        EngineError::Custom(err) => OperatorError::Other(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skg_turn::infer::{InferRequest, InferResponse};
    use skg_turn::provider::ProviderError;
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
        ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
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
        let tools = ToolRegistry::new();

        let output = run_with_provider("test-model", "system", &tools, 0, 128, &provider, "hi")
            .await
            .expect("budget exits should return operator output");

        assert_eq!(output.exit_reason, layer0::operator::ExitReason::MaxTurns);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
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

#[cfg(not(any(
    feature = "provider-anthropic",
    feature = "provider-openai",
    feature = "provider-ollama"
)))]
fn resolve_model(
    model: &str,
    _system_prompt: String,
    _tools: ToolRegistry,
    _max_turns: u32,
    _max_tokens: u32,
 ) -> Result<BuiltAgent, AgentBuildError> {
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
