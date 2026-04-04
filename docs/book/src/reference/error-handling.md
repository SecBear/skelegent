# Error Handling

> **Note:** This page reflects the v2 error model. `OperatorError` and `OrchError` from v1 are deprecated; `ProtocolError` is the canonical error type at invocation boundaries.

## Design pattern

skelegent uses `thiserror` for all error types. Each protocol has its own error enum in `layer0::error`. Error types are `#[non_exhaustive]` so new variants can be added without breaking downstream code.

Every error enum includes an `Other` variant with `#[from] Box<dyn std::error::Error + Send + Sync>` for wrapping arbitrary errors. This provides an escape hatch for implementation-specific errors that do not fit the named variants.

## Error types by protocol

### ProtocolError

The canonical error type at all invocation boundaries (`layer0::error::ProtocolError`):

```rust
pub enum ProtocolError {
    NotFound { operator: OperatorId },   // Operator not registered
    PolicyDenied { reason: String },     // Middleware/policy short-circuit
    Transient { message: String },       // Retryable failure
    Permanent { message: String },       // Non-retryable failure
    Internal { message: String },        // Unexpected runtime error
    Other(Box<dyn Error>),               // Catch-all
}
```

`ProtocolError::is_retryable()` returns `true` for `Transient`, and `false` for `Permanent`, `PolicyDenied`, and `NotFound`. The `RetryMiddleware` uses this to decide whether to retry a failed dispatch.

### StateError

Errors from state operations (`layer0::error::StateError`):

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

Errors from environment operations (`layer0::error::EnvError`):

```rust
pub enum EnvError {
    ProvisionFailed(String),    // Failed to set up the environment
    IsolationViolation(String), // Isolation boundary violated
    CredentialFailed(String),   // Credential injection failed
    ResourceExceeded(String),   // Resource limit exceeded
    Other(Box<dyn Error>),      // Catch-all
}
```

### ProviderError

Errors from LLM providers (Layer 1, `skg_turn::provider::ProviderError`):

```rust
pub enum ProviderError {
    TransientError { message: String, status: Option<u16> }, // 5xx / network failure
    RateLimited { retry_after: Option<Duration> },           // 429 response
    InvalidRequest { message: String, status: Option<u16> }, // 4xx client error (not retryable)
    ContentBlocked { message: String }, // Content blocked by provider
    AuthFailed(String),       // 401/403 response
    InvalidResponse(String),  // Response parse failure
    Other(Box<dyn Error>),    // Catch-all
}
```

`ProviderError::is_retryable()` returns `true` for `RateLimited` and `TransientError`. `InvalidRequest` is not retryable — 4xx client errors indicate malformed requests that will never succeed on retry.

### ToolError

Errors from tool operations (Layer 1, `skg_tool::ToolError`):

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
  ProtocolError
        ↓ (same type at all invocation boundaries)
  Dispatcher / Environment callers
```

Provider and tool errors are mapped to `ProtocolError` by the operator implementation (e.g., the `react_loop`-based operator maps `ProviderError::RateLimited` to `ProtocolError::Transient { .. }`). The `RetryMiddleware` checks `ProtocolError::is_retryable()` to determine whether to retry.

This unified error type ensures callers at every level see the same abstraction. There is no impedance mismatch between operator errors and orchestration errors.
