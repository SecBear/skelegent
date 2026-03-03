# neuron-state-memory

> In-memory `StateStore` implementation for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-state-memory.svg)](https://crates.io/crates/neuron-state-memory)
[![docs.rs](https://docs.rs/neuron-state-memory/badge.svg)](https://docs.rs/neuron-state-memory)
[![license](https://img.shields.io/crates/l/neuron-state-memory.svg)](LICENSE-MIT)

## Overview

`neuron-state-memory` provides a thread-safe, async-ready in-memory implementation of the
`StateStore` trait from [`layer0`](../layer0). State is stored as a `HashMap` behind a `RwLock`,
scoped to the process lifetime.

Use it for:
- Tests and CI (no disk I/O, no cleanup required)
- Short-lived operator runs where durability is not needed
- Development and prototyping

For durable persistence, use [`neuron-state-fs`](../neuron-state-fs) instead.

## Usage

```toml
[dependencies]
neuron-state-memory = "0.4"
```

```rust
use neuron_state_memory::MemoryStateStore;
use layer0::StateStore;
use std::sync::Arc;

let store: Arc<dyn StateStore> = Arc::new(MemoryStateStore::new());
store.write("session:42:last_tool", b"calculator").await?;
let val = store.read("session:42:last_tool").await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
