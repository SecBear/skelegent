# Middleware & Interception

skelegent uses two complementary interception mechanisms:

- **Per-boundary middleware** (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`) — wraps protocol-level operations using the continuation pattern. Defined in `layer0::middleware`.
- **Operator-local interception** (Rule system) — typed per-trigger rules inside the context engine. Rules fire via Trigger enum: Before (pre-inference, pre-tool), After (post-inference, post-tool), or When (exit checks). Defined in `skg-context-engine::rules`.

Security middleware (`RedactionMiddleware`, `ExfilGuardMiddleware`) lives in the `skg-hook-security` crate.

## Per-boundary middleware

Three traits — one per Layer 0 protocol boundary — follow the continuation pattern: call `next` to forward, skip `next` to short-circuit.

### DispatchMiddleware (wraps Dispatcher::dispatch)

```rust
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, ProtocolError>;
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
use layer0::id::OperatorId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::error::ProtocolError;

struct LoggingMiddleware;

#[async_trait]
impl DispatchMiddleware for LoggingMiddleware {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, ProtocolError> {
        tracing::info!(%operator, "dispatch start");
        let result = next.dispatch(operator, input).await;
        tracing::info!(%operator, ok = result.is_ok(), "dispatch end");
        result
    }
}
```

### Example: dispatch guardrail (deny a tool by name)

```rust,no_run
use async_trait::async_trait;
use layer0::middleware::{DispatchMiddleware, DispatchNext};
use layer0::id::OperatorId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::error::ProtocolError;

struct DenyToolMiddleware {
    denied: String,
}

#[async_trait]
impl DispatchMiddleware for DenyToolMiddleware {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, ProtocolError> {
        if operator.as_str() == self.denied {
            return Err(ProtocolError::PolicyDenied {
                reason: format!("tool {} is denied by policy", self.denied),
            });
        }
        next.dispatch(operator, input).await
    }
}
```

## Rule System (operator-local interception)

For interception inside the context engine (before/after inference, before/after tool calls, exit checks), use the Rule system. Rules fire via Trigger enum with three phases: Before (pre-inference, pre-tool), After (post-inference, post-tool), or When (exit conditions). Each rule has a default no-op implementation — override only what you need.

```rust
#[async_trait]
pub trait Rule: Send + Sync {
    async fn before_inference(&self, state: &LoopState) -> RuleAction { ... }
    async fn after_inference(&self, state: &LoopState, response: &Content) -> RuleAction { ... }
    async fn before_tool_call(&self, state: &LoopState, tool: &str, input: &Value) -> RuleAction { ... }
    async fn after_tool_call(&self, state: &LoopState, tool: &str, result: &str) -> RuleAction { ... }
    async fn when_exit_check(&self, state: &LoopState) -> RuleAction { ... }
    async fn before_steering_inject(&self, state: &LoopState, messages: &[String]) -> RuleAction { ... }
    async fn when_steering_skip(&self, state: &LoopState, skipped: &[String]) { }
    async fn before_compaction(&self, state: &LoopState) -> RuleAction { ... }
    async fn after_compaction(&self, state: &LoopState) { }
}
```

### Return types

- **`RuleAction`** — `Continue` or `Halt { reason }`. Returned by `before_inference`, `after_inference`, `when_exit_check`, `before_steering_inject`, `before_compaction`.

### Attaching a rule

```rust,no_run
use skg_context_engine::{Context, react_loop, ReactLoopConfig};
use skg_context_engine::rule::Rule;

// Rules are attached to Context, then passed to react_loop
let mut ctx = Context::new();
ctx.add_rule(my_rule);

// react_loop fires rules automatically during execution
let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config).await?;
```

### Example: budget enforcement rule

```rust,no_run
use async_trait::async_trait;
use skg_context_engine::rules::{Rule, RuleAction, LoopState};
use rust_decimal_macros::dec;

struct BudgetRule;

#[async_trait]
impl Rule for BudgetRule {
    async fn after_inference(&self, state: &LoopState, _response: &layer0::content::Content) -> RuleAction {
        if state.cost > dec!(1.00) {
            RuleAction::Halt {
                reason: "budget exceeded $1.00".into(),
            }
        } else {
            RuleAction::Continue
        }
    }
}
```

### Example: tool input sanitizer rule

```rust,no_run
use async_trait::async_trait;
use skg_context_engine::rules::{Rule, RuleAction, LoopState};
use serde_json::Value;

struct StripSecretRule;

#[async_trait]
impl Rule for StripSecretRule {
    async fn before_tool_call(
        &self,
        _state: &LoopState,
        _tool_name: &str,
        input: &Value,
    ) -> RuleAction {
        if let Some(obj) = input.as_object() {
            if obj.contains_key("api_key") {
                let mut cleaned = obj.clone();
                cleaned.remove("api_key");
                return RuleAction::ModifyInput {
                    new_input: Value::Object(cleaned),
                };
            }
        }
        RuleAction::Continue
    }
}
```

## Security middleware

The `skg-hook-security` crate provides two production-ready middleware implementations:

- **`RedactionMiddleware`** — redacts sensitive data from dispatch output (implements `DispatchMiddleware`).
- **`ExfilGuardMiddleware`** — blocks exfiltration attempts in dispatch input (implements `DispatchMiddleware`).

## Use cases

| Use case | Mechanism | Why |
|----------|-----------|-----|
| Budget enforcement | `Rule::after_inference` | Needs access to loop state cost |
| Tool policy (deny/skip) | `Rule::before_tool_call` | Per-tool decision inside the loop |
| Secret redaction | `RedactionMiddleware` or `Rule::after_tool_call` | Boundary-level or loop-level |
| Telemetry / logging | `DispatchMiddleware` (observer) | Cross-cutting, protocol-level |
| Encryption at rest | `StoreMiddleware` | Wraps state store reads/writes |
| Steering audit | `Rule::before_steering_inject` | Observe/block steering injection |
| Exfiltration guard | `ExfilGuardMiddleware` | Policy enforcement at dispatch boundary |
