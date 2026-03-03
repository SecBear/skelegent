# neuron-provider-ollama

> Ollama local model provider for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-provider-ollama.svg)](https://crates.io/crates/neuron-provider-ollama)
[![docs.rs](https://docs.rs/neuron-provider-ollama/badge.svg)](https://docs.rs/neuron-provider-ollama)
[![license](https://img.shields.io/crates/l/neuron-provider-ollama.svg)](LICENSE-MIT)

## Overview

`neuron-provider-ollama` implements the `Provider` trait from
[`neuron-turn`](../neuron-turn) for [Ollama](https://ollama.com), a local LLM runtime.
It speaks Ollama's native chat API (not the OpenAI-compat shim), which enables access to
Ollama-specific features like model keep-alive control and native tool call format.

Supports: any model loaded into your local Ollama instance (`llama3.2`, `qwen2.5`, `mistral`,
`phi4`, etc.).

## Usage

```toml
[dependencies]
neuron-provider-ollama = "0.4"
neuron-turn = "0.4"
```

### Setup

Start Ollama (`ollama serve`) and pull a model (`ollama pull llama3.2`).

```rust
use neuron_provider_ollama::OllamaProvider;

let provider = OllamaProvider::default(); // connects to http://localhost:11434
// Use provider with ReactOperator or SingleShotOperator
```

### Custom host

```rust
let provider = OllamaProvider::builder()
    .base_url("http://my-gpu-box:11434")
    .model("qwen2.5:14b")
    .build()?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
