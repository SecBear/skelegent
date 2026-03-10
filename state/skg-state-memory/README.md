# skg-state-memory

> In-memory `StateStore` implementation for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-state-memory.svg)](https://crates.io/crates/skg-state-memory)
[![docs.rs](https://docs.rs/skg-state-memory/badge.svg)](https://docs.rs/skg-state-memory)
[![license](https://img.shields.io/crates/l/skg-state-memory.svg)](LICENSE-MIT)

## Overview

`skg-state-memory` provides a thread-safe, async-ready in-memory implementation of the
`StateStore` trait from [`layer0`](../../layer0). State is stored as a `HashMap` behind a `RwLock`,
scoped to the process lifetime.

Use it for:
- Tests and CI (no disk I/O, no cleanup required)
- Short-lived operator runs where durability is not needed
- Development and prototyping

For durable persistence, use [`skg-state-fs`](../skg-state-fs) instead.

## Usage

```toml
[dependencies]
skg-state-memory = "0.4"
```

```rust
use skg_state_memory::MemoryStateStore;
use layer0::StateStore;
use std::sync::Arc;

let store: Arc<dyn StateStore> = Arc::new(MemoryStateStore::new());
store.write("session:42:last_tool", b"calculator").await?;
let val = store.read("session:42:last_tool").await?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
