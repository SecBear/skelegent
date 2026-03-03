# neuron-orch-kit

> Unopinionated wiring kit for composing neuron systems

[![crates.io](https://img.shields.io/crates/v/neuron-orch-kit.svg)](https://crates.io/crates/neuron-orch-kit)
[![docs.rs](https://docs.rs/neuron-orch-kit/badge.svg)](https://docs.rs/neuron-orch-kit)
[![license](https://img.shields.io/crates/l/neuron-orch-kit.svg)](LICENSE-MIT)

## Overview

`neuron-orch-kit` provides the plumbing layer for composing multiple operators into a system:
effect routing, state fan-out, delegation resolution, and handoff handling. It is
**orchestration-strategy-agnostic** — it handles the mechanics of wiring without dictating the
execution model.

Pair it with [`neuron-orch-local`](../neuron-orch-local) for a turnkey in-process implementation,
or implement your own `Orchestrator` using this kit as the foundation.

Key components:

- **`EffectRouter`** — routes `Effect` variants (`WriteMemory`, `DeleteMemory`, `Delegate`,
  `Handoff`, `Signal`) to the appropriate handler
- **`SystemBuilder`** — declarative builder for registering operators, state stores, and
  environments before constructing a runnable system

## Usage

```toml
[dependencies]
neuron-orch-kit = "0.4"
layer0 = "0.4"
```

```rust
use neuron_orch_kit::SystemBuilder;

let system = SystemBuilder::new()
    .operator("summarizer", Arc::new(summarize_op))
    .operator("router", Arc::new(router_op))
    .state(Arc::new(memory_store))
    .env(Arc::new(local_env))
    .build()?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
