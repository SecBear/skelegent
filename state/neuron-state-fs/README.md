# neuron-state-fs

> Filesystem-backed `StateStore` for neuron — durable, zero-dependency persistence

[![crates.io](https://img.shields.io/crates/v/neuron-state-fs.svg)](https://crates.io/crates/neuron-state-fs)
[![docs.rs](https://docs.rs/neuron-state-fs/badge.svg)](https://docs.rs/neuron-state-fs)
[![license](https://img.shields.io/crates/l/neuron-state-fs.svg)](LICENSE-MIT)

## Overview

`neuron-state-fs` implements the `StateStore` trait from [`layer0`](../../layer0) using the local
filesystem. Each key maps to a file inside a configurable base directory, using a safe filename
encoding. Reads and writes are async (Tokio `fs`).

Use it for:
- Single-machine agents that need durable state across restarts
- Development when you want to inspect state without a database
- Sidecar deployments with a shared volume

For ephemeral / test use, prefer [`neuron-state-memory`](../neuron-state-memory).

## Usage

```toml
[dependencies]
neuron-state-fs = "0.4"
```

```rust
use neuron_state_fs::FsStateStore;
use layer0::StateStore;
use std::sync::Arc;

let store: Arc<dyn StateStore> = Arc::new(
    FsStateStore::new("/var/lib/my-agent/state")
);

store.write("session:42:plan", plan_bytes).await?;
let plan = store.read("session:42:plan").await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
