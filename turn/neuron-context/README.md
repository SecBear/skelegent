# neuron-context

> Context compaction strategies and assembly for neuron operators

[![crates.io](https://img.shields.io/crates/v/neuron-context.svg)](https://crates.io/crates/neuron-context)
[![docs.rs](https://docs.rs/neuron-context/badge.svg)](https://docs.rs/neuron-context)
[![license](https://img.shields.io/crates/l/neuron-context.svg)](LICENSE-MIT)

## Overview

`neuron-context` provides compaction functions and context assembly utilities for managing
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
neuron-context = "0.4"
```

```rust
use neuron_context::{sliding_window_compactor, tiered_compactor, TieredConfig};

// Create a sliding-window compactor closure
let mut compactor = sliding_window_compactor();

// Or a tiered compactor with custom active zone size
let mut tiered = tiered_compactor(TieredConfig { active_zone_size: 20 });

// Pass to Context::compact_with() during operator execution
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
