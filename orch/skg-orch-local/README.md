# skg-orch-local

> In-process orchestrator for skelegent — no network, no external services

[![crates.io](https://img.shields.io/crates/v/skg-orch-local.svg)](https://crates.io/crates/skg-orch-local)
[![docs.rs](https://docs.rs/skg-orch-local/badge.svg)](https://docs.rs/skg-orch-local)
[![license](https://img.shields.io/crates/l/skg-orch-local.svg)](LICENSE-MIT)

## Overview

`skg-orch-local` is a fully in-process implementation of `layer0`'s `Dispatcher`, `Signalable`, and `Queryable` traits.
Operators are registered by `OperatorId` and dispatched directly via `tokio::spawn`. No durability —
failed operators are not retried. Signals are tracked in an in-memory per-workflow journal.

Use it for:
- Single-machine agentic pipelines
- Testing multi-operator workflows without infrastructure
- Development and CI environments

## Exports

- **`LocalOrch`** — `new()`, `register(OperatorId, Arc<dyn Operator>)`, `signal_count(&WorkflowId)`

Implements `Dispatcher`, `Signalable`, and `Queryable` (from `layer0`): `dispatch`, `signal`, and `query`.

## Usage

```toml
[dependencies]
skg-orch-local = "0.4"
layer0 = "0.4"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use skg_orch_local::LocalOrch;
use layer0::{OperatorId, Content, Operator, OperatorInput, OperatorOutput, Dispatcher};
use layer0::operator::TriggerType;
use std::sync::Arc;

// Given an `op: Arc<dyn Operator>`, register and dispatch:
let mut orch = LocalOrch::new();
orch.register(OperatorId::new("worker"), op);

let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
// dispatch is async:
// let output = orch.dispatch(&OperatorId::new("worker"), input).await?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
