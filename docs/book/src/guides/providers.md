# Providers

Providers are the LLM backend abstraction in neuron. Each provider implements the `Provider` trait (defined in `neuron-turn`), which sends a completion request to an LLM API and returns the response.

## The Provider trait

```rust
pub trait Provider: Send + Sync {
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl Future<Output = Result<ProviderResponse, ProviderError>> + Send;
}
```

This trait uses RPITIT (return-position `impl Trait` in traits) and is intentionally **not** object-safe. The object-safe boundary is `layer0::Operator`, not `Provider`. See [Design Decisions](../architecture/design-decisions.md) for why.

## Available providers

### Anthropic (`neuron-provider-anthropic`)

Connects to the Anthropic Messages API for Claude models.

```rust,no_run
use neuron_provider_anthropic::AnthropicProvider;

let provider = AnthropicProvider::new("sk-ant-...");
```

Configuration:
- **API key:** Passed to `new()`. Read it from `ANTHROPIC_API_KEY` in production.
- **Default model:** `claude-haiku-4-5-20251001`. Override per-request via `ProviderRequest.model`.
- **Default max tokens:** 4096. Override per-request via `ProviderRequest.max_tokens`.
- **API URL:** Override with `.with_url()` for proxies or testing.

```rust,no_run
use neuron_provider_anthropic::AnthropicProvider;

let provider = AnthropicProvider::new("sk-ant-...")
    .with_url("https://proxy.example.com/v1/messages");
```

Cost is calculated per-response based on input and output token counts using the Haiku pricing model.

### OpenAI (`neuron-provider-openai`)

Connects to the OpenAI Chat Completions API.

```rust,no_run
use neuron_provider_openai::OpenAiProvider;

let provider = OpenAiProvider::new("sk-...");
```

Configuration:
- **API key:** Passed to `new()`. Read it from `OPENAI_API_KEY` in production.
- **API URL:** Override with `.with_url()` for Azure OpenAI or proxies.

### Ollama (`neuron-provider-ollama`)

Connects to a local Ollama instance for running open-weight models.

```rust,no_run
use neuron_provider_ollama::OllamaProvider;

let provider = OllamaProvider::new(); // defaults to http://localhost:11434
```

Configuration:
- **URL:** Defaults to `http://localhost:11434`. Override with `.with_url()`.
- No API key required (Ollama runs locally).

## ProviderRequest and ProviderResponse

The `ProviderRequest` struct is the common input to all providers:

```rust
pub struct ProviderRequest {
    pub model: Option<String>,           // Model identifier
    pub messages: Vec<ProviderMessage>,  // Conversation history
    pub tools: Vec<ToolDefinition>,      // Available tools (JSON Schema)
    pub max_tokens: Option<u32>,         // Max output tokens
    pub temperature: Option<f32>,        // Sampling temperature
    pub system: Option<String>,          // System prompt
    pub extra: serde_json::Value,        // Provider-specific extensions
}
```

The `extra` field allows provider-specific features (Anthropic's prompt caching, thinking blocks, etc.) without polluting the common interface.

The `ProviderResponse` contains the model's output:

```rust
pub struct ProviderResponse {
    pub content: Vec<ContentPart>,  // Text, tool use, images
    pub stop_reason: StopReason,    // EndTurn, ToolUse, MaxTokens
    pub usage: TokenUsage,          // Input/output/cache tokens
    pub model: String,              // Model that responded
    pub cost: Option<Decimal>,      // Calculated cost in USD
    pub truncated: Option<bool>,    // Whether input was truncated
}
```

## Using providers with operators

Providers are not used directly in most application code. Instead, you pass a provider to an operator:

```rust,no_run
use neuron_op_react::{ReactConfig, ReactOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_tool::ToolRegistry;

let provider = AnthropicProvider::new("sk-ant-...");
let config = ReactConfig {
    system_prompt: "You are a helpful assistant.".into(),
    default_model: "claude-haiku-4-5-20251001".into(),
    default_max_tokens: 4096,
    default_max_turns: 10,
};

let operator = ReactOperator::new(
    provider,
    ToolRegistry::new(),
    Box::new(neuron_turn_kit::FullContext),
    Arc::new(neuron_state_memory::MemoryStore::new()),
    config,
);
// `operator` implements `layer0::Operator`
```

The operator handles the ReAct loop internally, calling `provider.complete()` as many times as needed. The `Provider` type parameter is erased at the `Operator` trait boundary -- callers interact with `&dyn Operator` or `Box<dyn Operator>`.

## Error handling

Provider errors are represented by `ProviderError`:

```rust
pub enum ProviderError {
    TransientError { message: String, status: Option<u16> },
    RateLimited,
    ContentBlocked { message: String },
    AuthFailed(String),
    InvalidResponse(String),
    Other(Box<dyn Error + Send + Sync>),
}
```

`ProviderError::is_retryable()` returns `true` for `RateLimited` and `TransientError` (transient network errors), and `false` for `AuthFailed`, `ContentBlocked`, and `InvalidResponse` (permanent errors). Operator implementations use this to decide whether to retry.
