# skg-orch-kit

> Unopinionated wiring kit for composing skelegent systems

[![crates.io](https://img.shields.io/crates/v/skg-orch-kit.svg)](https://crates.io/crates/skg-orch-kit)
[![docs.rs](https://docs.rs/skg-orch-kit/badge.svg)](https://docs.rs/skg-orch-kit)
[![license](https://img.shields.io/crates/l/skg-orch-kit.svg)](LICENSE-MIT)

## Overview

`skg-orch-kit` provides the plumbing layer for composing multiple operators into a system:
effect routing, state fan-out, delegation resolution, and handoff handling. It is
**orchestration-strategy-agnostic** — it handles the mechanics of wiring without dictating the
execution model.

Pair it with [`skg-orch-local`](../skg-orch-local) for a turnkey in-process implementation,
or implement your own `Dispatcher`, `Signalable`, and `Queryable` traits using this kit as the foundation.

Key components:

- **`EffectRouter`** — routes `Effect` variants (`WriteMemory`, `DeleteMemory`, `Delegate`,
  `Handoff`, `Signal`) to the appropriate handler
- **`SystemBuilder`** — declarative builder for registering operators, state stores, and
  environments before constructing a runnable system

## Usage

```toml
[dependencies]
skg-orch-kit = "0.4"
layer0 = "0.4"
```

```rust
use skg_orch_kit::SystemBuilder;

let system = SystemBuilder::new()
    .operator("summarizer", Arc::new(summarize_op))
    .operator("router", Arc::new(router_op))
    .state(Arc::new(memory_store))
    .env(Arc::new(local_env))
    .build()?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
