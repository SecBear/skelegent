# layer0

> Protocol layer for the Skelegent agentic AI architecture

[![crates.io](https://img.shields.io/crates/v/layer0.svg)](https://crates.io/crates/layer0)
[![docs.rs](https://docs.rs/layer0/badge.svg)](https://docs.rs/layer0)
[![license](https://img.shields.io/crates/l/layer0.svg)](LICENSE-MIT)

## Overview

`layer0` defines the foundational protocol traits for composable agentic AI systems. It contains
**no implementations** — only the contracts every agentic component must satisfy.

Four protocol traits + two cross-cutting interfaces:

| Protocol | Trait | Responsibility |
|----------|-------|----------------|
| ① Operator | `Operator` | One agent's work per cycle |
| ② Dispatch | `Dispatcher` | Multi-agent invocation primitive |
| ③ State | `StateStore` / `StateReader` | Persistent key-value memory |
| ④ Environment | `Environment` | Isolation, credentials, resource limits |
| ⑤ Middleware | `DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware` | Interception + policy at each boundary |
| ⑥ Lifecycle | `BudgetEvent`, `CompactionEvent` | Cross-layer coordination events |

## Exports

**Operator:** `Operator`, `OperatorInput`, `OperatorOutput`, `OperatorConfig`, `OperatorMetadata`,
`SubDispatchRecord`, `ToolMetadata`, `ExitReason`

**Dispatcher:** `Dispatcher`, `Dispatcher::dispatch`

**Signalable:** `Signalable`, `Signalable::signal`

**Queryable:** `Queryable`, `Queryable::query`

**State:** `StateStore`, `StateReader`, `SearchResult`

**Environment:** `Environment`, `EnvironmentSpec`

**Middleware:** `DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`

**Lifecycle:** `BudgetEvent`, `CompactionEvent`, `ObservableEvent`

**Effects:** `Effect`, `Scope`, `SignalPayload`

**Identity:** `OperatorId`, `WorkflowId`, `ScopeId`, `SessionId`

**Content:** `Content`, `ContentBlock`

**Errors:** `EnvError`, `OperatorError`, `OrchError`, `StateError`

**Misc:** `DurationMs`, `SecretAccessEvent`, `SecretAccessOutcome`, `SecretSource`

## Usage

```toml
[dependencies]
layer0 = "0.4"
```

### Test utilities

```toml
[dev-dependencies]
layer0 = { version = "0.4", features = ["test-utils"] }
```

The `test-utils` feature exports in-memory implementations useful for testing downstream crates:
`EchoOperator`, `InMemoryStore`, `LocalEnvironment`, `LocalOrchestrator`.

### Implementing the Operator trait

```rust,no_run
use async_trait::async_trait;
use layer0::{Content, ExitReason, Operator, OperatorInput, OperatorOutput};
use layer0::error::OperatorError;

pub struct MyOperator;

#[async_trait]
impl Operator for MyOperator {
    async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let msg = Content::text("done");
        Ok(OperatorOutput::new(msg, ExitReason::Complete))
    }
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
