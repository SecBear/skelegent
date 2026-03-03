# neuron-context

> Context window management strategies for neuron operators

[![crates.io](https://img.shields.io/crates/v/neuron-context.svg)](https://crates.io/crates/neuron-context)
[![docs.rs](https://docs.rs/neuron-context/badge.svg)](https://docs.rs/neuron-context)
[![license](https://img.shields.io/crates/l/neuron-context.svg)](LICENSE-MIT)

## Overview

`neuron-context` implements the `ContextStrategy` types used by providers to manage conversation
history before sending requests to an LLM. As conversations grow, context strategies control
what messages are kept, trimmed, or summarized to fit within a model's context window.

Built-in strategies:

| Strategy | Behaviour |
|----------|-----------|
| `Unlimited` | No windowing — all history is sent (default, risky for long conversations) |
| `LastN { n }` | Keep only the last *n* turns |
| `MaxTokens { limit }` | Trim oldest messages until estimated token count is within `limit` |

## Usage

```toml
[dependencies]
neuron-context = "0.4"
neuron-turn = "0.4"
```

```rust
use neuron_context::ContextStrategy;
use neuron_turn::TurnRequest;

let strategy = ContextStrategy::LastN { n: 10 };
// Pass to a provider or operator that accepts ContextStrategy
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
