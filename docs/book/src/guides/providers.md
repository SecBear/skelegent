# Providers

Providers are the LLM backend abstraction in skelegent. Each provider implements the `Provider` trait (defined in `skg-turn`), which sends a completion request to an LLM API and returns the response.

## The Provider trait

```rust
pub trait Provider: Send + Sync {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send;
}
```

This trait uses RPITIT (return-position `impl Trait` in traits) and is intentionally **not** object-safe. The object-safe boundary is `layer0::Operator`, not `Provider`. See [Design Decisions](../architecture/design-decisions.md) for why.

## Available providers

### Anthropic (`skg-provider-anthropic`)

Connects to the Anthropic Messages API for Claude models.

```rust,no_run
use skg_provider_anthropic::AnthropicProvider;

let provider = AnthropicProvider::new("sk-ant-...");
```

Configuration:
- **API key:** Passed to `new()`. Read it from `ANTHROPIC_API_KEY` in production.
- **Default model:** `claude-haiku-4-5-20251001`. Override per-request via `InferRequest.model`.
- **Default max tokens:** 4096. Override per-request via `InferRequest.max_tokens`.
- **API URL:** Override with `.with_url()` for proxies or testing.

```rust,no_run
use skg_provider_anthropic::AnthropicProvider;

let provider = AnthropicProvider::new("sk-ant-...")
    .with_url("https://proxy.example.com/v1/messages");
```

Cost is calculated per-response based on input and output token counts using the Haiku pricing model.

### OpenAI (`skg-provider-openai`)

Connects to the OpenAI Chat Completions API.

```rust,no_run
use skg_provider_openai::OpenAiProvider;

let provider = OpenAiProvider::new("sk-...");
```

Configuration:
- **API key:** Passed to `new()`. Read it from `OPENAI_API_KEY` in production.
- **API URL:** Override with `.with_url()` for Azure OpenAI or proxies.

### Ollama (`skg-provider-ollama`)

Connects to a local Ollama instance for running open-weight models.

```rust,no_run
use skg_provider_ollama::OllamaProvider;

let provider = OllamaProvider::new(); // defaults to http://localhost:11434
```

Configuration:
- **URL:** Defaults to `http://localhost:11434`. Override with `.with_url()`.
- No API key required (Ollama runs locally).

## InferRequest and InferResponse

The `InferRequest` struct is the common input to all providers:

```rust
pub struct InferRequest {
    pub model: Option<String>,           // Model identifier
    pub messages: Vec<Message>,           // Conversation history (layer0 types)
    pub tools: Vec<ToolSchema>,           // Available tools (JSON Schema)
    pub max_tokens: Option<u32>,          // Max output tokens
    pub temperature: Option<f64>,         // Sampling temperature
    pub system: Option<String>,           // System prompt
    pub extra: serde_json::Value,         // Provider-specific extensions
}
```

The `extra` field allows provider-specific features (Anthropic's prompt caching, thinking blocks, etc.) without polluting the common interface.

The `InferResponse` contains the model's output:

```rust
pub struct InferResponse {
    pub content: Content,             // Response content
    pub tool_calls: Vec<ToolCall>,    // Tool calls requested by the model
    pub stop_reason: StopReason,      // EndTurn, ToolUse, MaxTokens
    pub usage: TokenUsage,            // Input/output/cache tokens
    pub model: String,                // Model that responded
    pub cost: Option<Decimal>,        // Calculated cost in USD
}
```

## Using providers with operators

Providers are not used directly in most application code. Instead, you pass a provider to an operator:

```rust,no_run
use skg_context_engine::{Context, react_loop, ReactLoopConfig};
use skg_provider_anthropic::AnthropicProvider;
use layer0::DispatchContext;
use layer0::id::{DispatchId, OperatorId};
use skg_tool::ToolRegistry;

let provider = AnthropicProvider::new("sk-ant-...");
let config = ReactLoopConfig {
    system_prompt: "You are a helpful assistant.".into(),
    model: Some("claude-haiku-4-5-20251001".into()),
    max_tokens: Some(4096),
    temperature: None,
};

let tools = ToolRegistry::new();
let tool_context = DispatchContext::new(DispatchId::new("assistant"), OperatorId::new("assistant"));
let mut context = Context::new();

// react_loop drives the ReAct loop, calling provider.infer() as needed
let response = react_loop(&provider, &tools, &tool_context, &mut context, &config).await;
```

To make this usable behind `layer0::Operator` (which is object-safe), wrap the provider, tools, and config in a struct that implements `Operator`. The `Provider` type parameter is erased at the `Operator` trait boundary -- callers interact with `&dyn Operator` or `Box<dyn Operator>`.

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
