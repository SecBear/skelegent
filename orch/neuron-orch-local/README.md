# neuron-orch-local

> In-process orchestrator for neuron — no network, no external services

[![crates.io](https://img.shields.io/crates/v/neuron-orch-local.svg)](https://crates.io/crates/neuron-orch-local)
[![docs.rs](https://docs.rs/neuron-orch-local/badge.svg)](https://docs.rs/neuron-orch-local)
[![license](https://img.shields.io/crates/l/neuron-orch-local.svg)](LICENSE-MIT)

## Overview

`neuron-orch-local` is a fully in-process implementation of `layer0`'s `Orchestrator` trait.
Operators are registered by `AgentId` and dispatched directly via `tokio::spawn`. No durability —
failed operators are not retried. Signals are tracked in an in-memory per-workflow journal.

Use it for:
- Single-machine agentic pipelines
- Testing multi-operator workflows without infrastructure
- Development and CI environments

## Exports

- **`LocalOrch`** — `new()`, `register(AgentId, Arc<dyn Operator>)`, `signal_count(&WorkflowId)`

Implements `Orchestrator` (from `layer0`): `dispatch`, `dispatch_many`, `signal`, `query`.

## Usage

```toml
[dependencies]
neuron-orch-local = "0.4"
layer0 = "0.4"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use neuron_orch_local::LocalOrch;
use layer0::{AgentId, Content, Operator, OperatorInput, OperatorOutput, Orchestrator};
use layer0::operator::TriggerType;
use std::sync::Arc;

// Given an `op: Arc<dyn Operator>`, register and dispatch:
let mut orch = LocalOrch::new();
orch.register(AgentId::new("worker"), op);

let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
// dispatch is async:
// let output = orch.dispatch(&AgentId::new("worker"), input).await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
