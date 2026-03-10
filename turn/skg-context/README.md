# skg-context

> Context compaction strategies and assembly for skelegent operators

[![crates.io](https://img.shields.io/crates/v/skg-context.svg)](https://crates.io/crates/skg-context)
[![docs.rs](https://docs.rs/skg-context/badge.svg)](https://docs.rs/skg-context)
[![license](https://img.shields.io/crates/l/skg-context.svg)](LICENSE-MIT)

## Overview

`skg-context` provides compaction functions and context assembly utilities for managing
conversation history before sending requests to an LLM. As conversations grow, compactors
control which messages are kept, trimmed, or discarded to fit within a model's context window.

Built-in compactors:

| Compactor | Behaviour |
|-----------|-----------|
| `sliding_window_compactor()` | Preserves the first message and fills remaining budget from the most recent messages backward; pinned messages always survive |
| `tiered_compactor(TieredConfig)` | Zone-partitioned: keeps pinned and the most-recent *N* normal messages (active zone), discards older normal and noise messages |
| `salience_packing_compactor(SaliencePackingConfig)` | MMR-based selection maximising salience diversity within a token budget; optional "lost in the middle" reordering |

Also provides `ContextAssembler` for assembling sweep context packages from state store data.

## Usage

```toml
[dependencies]
skg-context = "0.4"
```

```rust
use skg_context::{sliding_window_compactor, tiered_compactor, TieredConfig};

// Create a sliding-window compactor closure
let mut compactor = sliding_window_compactor();

// Or a tiered compactor with custom active zone size
let mut tiered = tiered_compactor(TieredConfig { active_zone_size: 20 });

// Pass to Context::compact_with() during operator execution
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
