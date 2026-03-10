# skg-state-fs

> Filesystem-backed `StateStore` for skelegent — durable, zero-dependency persistence

[![crates.io](https://img.shields.io/crates/v/skg-state-fs.svg)](https://crates.io/crates/skg-state-fs)
[![docs.rs](https://docs.rs/skg-state-fs/badge.svg)](https://docs.rs/skg-state-fs)
[![license](https://img.shields.io/crates/l/skg-state-fs.svg)](LICENSE-MIT)

## Overview

`skg-state-fs` implements the `StateStore` trait from [`layer0`](../../layer0) using the local
filesystem. Each key maps to a file inside a configurable base directory, using a safe filename
encoding. Reads and writes are async (Tokio `fs`).

Use it for:
- Single-machine agents that need durable state across restarts
- Development when you want to inspect state without a database
- Sidecar deployments with a shared volume

For ephemeral / test use, prefer [`skg-state-memory`](../skg-state-memory).

## Usage

```toml
[dependencies]
skg-state-fs = "0.4"
```

```rust
use skg_state_fs::FsStateStore;
use layer0::StateStore;
use std::sync::Arc;

let store: Arc<dyn StateStore> = Arc::new(
    FsStateStore::new("/var/lib/my-agent/state")
);

store.write("session:42:plan", plan_bytes).await?;
let plan = store.read("session:42:plan").await?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
