# Error Handling

> **Note:** This page covers the error type design. Usage examples and error recovery patterns are planned for a future update.

## Design pattern

neuron uses `thiserror` for all error types. Each protocol has its own error enum in `layer0::error`. Error types are `#[non_exhaustive]` so new variants can be added without breaking downstream code.

Every error enum includes an `Other` variant with `#[from] Box<dyn std::error::Error + Send + Sync>` for wrapping arbitrary errors. This provides an escape hatch for implementation-specific errors that do not fit the named variants.

## Error types by protocol

### OperatorError

Errors from operator execution (Layer 0, `layer0::error::OperatorError`):

```rust
pub enum OperatorError {
    Model(String),           // LLM provider error
    Tool { tool, message },  // Tool execution error
    ContextAssembly(String), // Context assembly failed
    Retryable(String),       // Transient, may succeed on retry
    NonRetryable(String),    // Permanent failure (budget, safety, invalid input)
    Other(Box<dyn Error>),   // Catch-all
}
```

The `Retryable` / `NonRetryable` distinction lets orchestrators make retry decisions without inspecting error details.

### OrchError

Errors from orchestration (Layer 0, `layer0::error::OrchError`):

```rust
pub enum OrchError {
    AgentNotFound(String),    // Agent ID not registered
    WorkflowNotFound(String), // Workflow ID not found
    DispatchFailed(String),   // Dispatch failed
    SignalFailed(String),     // Signal delivery failed
    OperatorError(OperatorError), // Propagated from operator
    Other(Box<dyn Error>),    // Catch-all
}
```

`OperatorError` propagates into `OrchError` via the `From` trait. If an operator fails during dispatch, the error is wrapped automatically.

### StateError

Errors from state operations (Layer 0, `layer0::error::StateError`):

```rust
pub enum StateError {
    NotFound { scope, key },  // Key expected to exist but doesn't
    WriteFailed(String),      // Write operation failed
    Serialization(String),    // Serde error
    Other(Box<dyn Error>),    // Catch-all
}
```

Note: `StateStore::read` returns `Ok(None)` for missing keys. `NotFound` is for higher-level APIs that expect a key to exist.

### EnvError

Errors from environment operations (Layer 0, `layer0::error::EnvError`):

```rust
pub enum EnvError {
    ProvisionFailed(String),    // Failed to set up the environment
    IsolationViolation(String), // Isolation boundary violated
    CredentialFailed(String),   // Credential injection failed
    ResourceExceeded(String),   // Resource limit exceeded
    OperatorError(OperatorError), // Propagated from operator
    Other(Box<dyn Error>),      // Catch-all
}
```

Like `OrchError`, `OperatorError` propagates into `EnvError` via `From`.

### ProviderError

Errors from LLM providers (Layer 1, `neuron_turn::provider::ProviderError`):

```rust
pub enum ProviderError {
    TransientError { message: String, status: Option<u16> }, // HTTP/network failure
    RateLimited,              // 429 response
    ContentBlocked { message: String }, // Content blocked by provider
    AuthFailed(String),       // 401/403 response
    InvalidResponse(String),  // Response parse failure
    Other(Box<dyn Error>),    // Catch-all
}
```

`ProviderError::is_retryable()` returns `true` for `RateLimited` and `TransientError`.

### ToolError

Errors from tool operations (Layer 1, `neuron_tool::ToolError`):

```rust
pub enum ToolError {
    NotFound(String),         // Tool not in registry
    ExecutionFailed(String),  // Tool execution failed
    InvalidInput(String),     // Input didn't match schema
    Other(Box<dyn Error>),    // Catch-all
}
```

## Error propagation

Errors propagate upward through the layer stack:

```
ProviderError / ToolError
        ↓ (mapped by operator implementation)
  OperatorError
        ↓ (From impl)
  OrchError / EnvError
```

Provider and tool errors are mapped to `OperatorError` by the operator implementation (e.g., `ReactOperator` maps `ProviderError::RateLimited` to `OperatorError::Retryable`). Operator errors propagate into orchestration and environment errors automatically via `From` impls.

This layered propagation ensures that callers at each level see errors appropriate to their abstraction. An orchestrator sees `OrchError`, never `ProviderError`.
