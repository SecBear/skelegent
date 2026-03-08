# Middleware & Interception

neuron uses two complementary interception mechanisms:

- **Per-boundary middleware** (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`) — wraps protocol-level operations using the continuation pattern. Defined in `layer0::middleware`.
- **Operator-local interception** (`ReactInterceptor`) — typed per-hook-point callbacks inside the ReAct loop. Defined in `neuron-op-react::intercept`.

Security middleware (`RedactionMiddleware`, `ExfilGuardMiddleware`) lives in the `neuron-hook-security` crate.

## Per-boundary middleware

Three traits — one per Layer 0 protocol boundary — follow the continuation pattern: call `next` to forward, skip `next` to short-circuit.

### DispatchMiddleware (wraps Orchestrator::dispatch)

```rust
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError>;
}
```

Code before `next.dispatch()` = pre-processing (input mutation, logging).
Code after `next.dispatch()` = post-processing (output mutation, metrics).
Not calling `next.dispatch()` = short-circuit (guardrail halt, cached response).

### StoreMiddleware (wraps StateStore read/write)

```rust
#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError>;

    async fn read(
        &self,
        scope: &Scope,
        key: &str,
        next: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError> {
        next.read(scope, key).await
    }
}
```

Use for: encryption-at-rest, audit trails, caching, access control.

### ExecMiddleware (wraps Environment::run)

```rust
#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError>;
}
```

Use for: resource metering, credential injection, sandboxing.

## Middleware stacks

Middleware composes via stack builders. Each stack organizes layers into three phases:

1. **Observers** — outermost; always run, always call next.
2. **Transformers** — mutate input/output, always call next.
3. **Guards** — innermost; may short-circuit by not calling next.

```rust,no_run
use layer0::middleware::DispatchStack;
use std::sync::Arc;

let stack = DispatchStack::builder()
    .observe(Arc::new(logging_middleware))
    .transform(Arc::new(sanitizer_middleware))
    .guard(Arc::new(policy_middleware))
    .build();
```

Call order: observers → transformers → guards → terminal (the real orchestrator). The same builder pattern applies to `StoreStack` and `ExecStack`.

### Example: dispatch logging middleware

```rust,no_run
use async_trait::async_trait;
use layer0::middleware::{DispatchMiddleware, DispatchNext};
use layer0::id::AgentId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::error::OrchError;

struct LoggingMiddleware;

#[async_trait]
impl DispatchMiddleware for LoggingMiddleware {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        tracing::info!(%agent, "dispatch start");
        let result = next.dispatch(agent, input).await;
        tracing::info!(%agent, ok = result.is_ok(), "dispatch end");
        result
    }
}
```

### Example: dispatch guardrail (deny a tool by name)

```rust,no_run
use async_trait::async_trait;
use layer0::middleware::{DispatchMiddleware, DispatchNext};
use layer0::id::AgentId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::error::OrchError;

struct DenyToolMiddleware {
    denied: String,
}

#[async_trait]
impl DispatchMiddleware for DenyToolMiddleware {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        if agent.as_str() == self.denied {
            return Err(OrchError::PolicyDenied {
                reason: format!("tool {} is denied by policy", self.denied),
            });
        }
        next.dispatch(agent, input).await
    }
}
```

## ReactInterceptor (operator-local interception)

For interception inside the ReAct loop (before/after inference, before/after tool calls, exit checks), use the `ReactInterceptor` trait. Every method has a default no-op implementation — override only what you need.

```rust
#[async_trait]
pub trait ReactInterceptor: Send + Sync {
    async fn pre_inference(&self, state: &LoopState) -> ReactAction { ... }
    async fn post_inference(&self, state: &LoopState, response: &Content) -> ReactAction { ... }
    async fn pre_sub_dispatch(&self, state: &LoopState, tool: &str, input: &Value) -> SubDispatchAction { ... }
    async fn post_sub_dispatch(&self, state: &LoopState, tool: &str, result: &str) -> SubDispatchResult { ... }
    async fn exit_check(&self, state: &LoopState) -> ReactAction { ... }
    async fn pre_steering_inject(&self, state: &LoopState, messages: &[String]) -> ReactAction { ... }
    async fn post_steering_skip(&self, state: &LoopState, skipped: &[String]) { }
    async fn pre_compaction(&self, state: &LoopState) -> ReactAction { ... }
    async fn post_compaction(&self, state: &LoopState) { }
}
```

### Return types

- **`ReactAction`** — `Continue` or `Halt { reason }`. Returned by `pre_inference`, `post_inference`, `exit_check`, `pre_steering_inject`, `pre_compaction`.
- **`SubDispatchAction`** — `Continue`, `Halt { reason }`, `Skip { reason }`, or `ModifyInput { new_input }`. Returned by `pre_sub_dispatch`.
- **`SubDispatchResult`** — `Continue`, `Halt { reason }`, or `ModifyOutput { new_output }`. Returned by `post_sub_dispatch`.

### Attaching an interceptor

```rust,no_run
use std::sync::Arc;
use neuron_op_react::{ReactOperator, ReactConfig};
use neuron_op_react::intercept::ReactInterceptor;

let op = ReactOperator::new(provider, tools, context_strategy, state_reader, config)
    .with_interceptor(Arc::new(my_interceptor));
```

### Example: budget enforcement interceptor

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, ReactAction, LoopState};
use rust_decimal_macros::dec;

struct BudgetInterceptor;

#[async_trait]
impl ReactInterceptor for BudgetInterceptor {
    async fn post_inference(&self, state: &LoopState, _response: &layer0::content::Content) -> ReactAction {
        if state.cost > dec!(1.00) {
            ReactAction::Halt {
                reason: "budget exceeded $1.00".into(),
            }
        } else {
            ReactAction::Continue
        }
    }
}
```

### Example: tool input sanitizer interceptor

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, SubDispatchAction, LoopState};
use serde_json::Value;

struct StripSecretInterceptor;

#[async_trait]
impl ReactInterceptor for StripSecretInterceptor {
    async fn pre_sub_dispatch(
        &self,
        _state: &LoopState,
        _tool_name: &str,
        input: &Value,
    ) -> SubDispatchAction {
        if let Some(obj) = input.as_object() {
            if obj.contains_key("api_key") {
                let mut cleaned = obj.clone();
                cleaned.remove("api_key");
                return SubDispatchAction::ModifyInput {
                    new_input: Value::Object(cleaned),
                };
            }
        }
        SubDispatchAction::Continue
    }
}
```

## Security middleware

The `neuron-hook-security` crate provides two production-ready middleware implementations:

- **`RedactionMiddleware`** — redacts sensitive data from dispatch output (implements `DispatchMiddleware`).
- **`ExfilGuardMiddleware`** — blocks exfiltration attempts in dispatch input (implements `DispatchMiddleware`).

## Use cases

| Use case | Mechanism | Why |
|----------|-----------|-----|
| Budget enforcement | `ReactInterceptor::post_inference` | Needs access to loop state cost |
| Tool policy (deny/skip) | `ReactInterceptor::pre_sub_dispatch` | Per-tool decision inside the loop |
| Secret redaction | `RedactionMiddleware` or `ReactInterceptor::post_sub_dispatch` | Boundary-level or loop-level |
| Telemetry / logging | `DispatchMiddleware` (observer) | Cross-cutting, protocol-level |
| Encryption at rest | `StoreMiddleware` | Wraps state store reads/writes |
| Steering audit | `ReactInterceptor::pre_steering_inject` | Observe/block steering injection |
| Exfiltration guard | `ExfilGuardMiddleware` | Policy enforcement at dispatch boundary |
