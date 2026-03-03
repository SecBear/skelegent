# neuron-provider-anthropic

> Anthropic Claude API provider for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-provider-anthropic.svg)](https://crates.io/crates/neuron-provider-anthropic)
[![docs.rs](https://docs.rs/neuron-provider-anthropic/badge.svg)](https://docs.rs/neuron-provider-anthropic)
[![license](https://img.shields.io/crates/l/neuron-provider-anthropic.svg)](LICENSE-MIT)

## Overview

`neuron-provider-anthropic` implements the `Provider` trait from
[`neuron-turn`](../neuron-turn) for the
[Anthropic Messages API](https://docs.anthropic.com/en/api/messages). It handles request
serialization, response parsing, tool call routing, and cost accounting for Claude models.

Supports: `claude-opus-4`, `claude-sonnet-4`, `claude-haiku-3-5`, and any future model
accepted by the Messages API.

## Usage

```toml
[dependencies]
neuron-provider-anthropic = "0.4"
neuron-turn = "0.4"
```

### Setup

Set `ANTHROPIC_API_KEY` in your environment (or inject via `neuron-env-local`).

```rust
use neuron_provider_anthropic::AnthropicProvider;
use neuron_turn::Provider;

let provider = AnthropicProvider::from_env()?;
// Use provider with ReactOperator or SingleShotOperator
```

### Custom base URL (proxy / testing)

```rust
let provider = AnthropicProvider::builder()
    .api_key("sk-ant-...")
    .base_url("https://my-proxy/anthropic")
    .model("claude-sonnet-4-5")
    .build()?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
