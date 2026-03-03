# neuron-provider-openai

> OpenAI API provider for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-provider-openai.svg)](https://crates.io/crates/neuron-provider-openai)
[![docs.rs](https://docs.rs/neuron-provider-openai/badge.svg)](https://docs.rs/neuron-provider-openai)
[![license](https://img.shields.io/crates/l/neuron-provider-openai.svg)](LICENSE-MIT)

## Overview

`neuron-provider-openai` implements the `Provider` trait from
[`neuron-turn`](../neuron-turn) for the
[OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat). It handles
request serialization, streaming-free response parsing, tool call routing, and cost accounting
for GPT models.

Supports: `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1`, `o3-mini`, and any model that
speaks the Chat Completions protocol (including many open-source proxies).

## Usage

```toml
[dependencies]
neuron-provider-openai = "0.4"
neuron-turn = "0.4"
```

### Setup

Set `OPENAI_API_KEY` in your environment (or inject via `neuron-env-local`).

```rust
use neuron_provider_openai::OpenAIProvider;

let provider = OpenAIProvider::from_env()?;
// Use provider with ReactOperator or SingleShotOperator
```

### OpenAI-compatible endpoints (Ollama, vLLM, LM Studio, etc.)

```rust
let provider = OpenAIProvider::builder()
    .api_key("not-needed")
    .base_url("http://localhost:11434/v1")
    .model("llama3.2")
    .build()?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
