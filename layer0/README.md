# layer0

> Protocol layer for the Neuron agentic AI architecture

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
| ② Orchestration | `Orchestrator` | Multi-agent composition + workflow routing |
| ③ State | `StateStore` / `StateReader` | Persistent key-value memory |
| ④ Environment | `Environment` | Isolation, credentials, resource limits |
| ⑤ Hooks | `Hook`, `HookPoint`, `HookAction` | Observation + intervention in the turn loop |
| ⑥ Lifecycle | `BudgetEvent`, `CompactionEvent` | Cross-layer coordination events |

## Exports

**Operator:** `Operator`, `OperatorInput`, `OperatorOutput`, `OperatorConfig`, `OperatorMetadata`,
`ToolCallRecord`, `ExitReason`

**Orchestrator:** `Orchestrator`, `QueryPayload`

**State:** `StateStore`, `StateReader`, `SearchResult`

**Environment:** `Environment`, `EnvironmentSpec`

**Hooks:** `Hook`, `HookAction`, `HookContext`, `HookPoint`

**Lifecycle:** `BudgetEvent`, `CompactionEvent`, `ObservableEvent`

**Effects:** `Effect`, `Scope`, `SignalPayload`

**Identity:** `AgentId`, `WorkflowId`, `ScopeId`, `SessionId`

**Content:** `Content`, `ContentBlock`

**Errors:** `EnvError`, `HookError`, `OperatorError`, `OrchError`, `StateError`

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
`EchoOperator`, `InMemoryStore`, `LocalEnvironment`, `LocalOrchestrator`, `LoggingHook`.

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

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
