# skg-provider-ollama

> Ollama local model provider for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-provider-ollama.svg)](https://crates.io/crates/skg-provider-ollama)
[![docs.rs](https://docs.rs/skg-provider-ollama/badge.svg)](https://docs.rs/skg-provider-ollama)
[![license](https://img.shields.io/crates/l/skg-provider-ollama.svg)](LICENSE-MIT)

## Overview

`skg-provider-ollama` implements the `Provider` trait from
[`skg-turn`](../../turn/skg-turn) for [Ollama](https://ollama.com), a local LLM runtime.
It speaks Ollama's native chat API (not the OpenAI-compat shim), which enables access to
Ollama-specific features like model keep-alive control and native tool call format.

Supports: any model loaded into your local Ollama instance (`llama3.2`, `qwen2.5`, `mistral`,
`phi4`, etc.).

## Usage

```toml
[dependencies]
skg-provider-ollama = "0.4"
skg-turn = "0.4"
```

### Setup

Start Ollama (`ollama serve`) and pull a model (`ollama pull llama3.2`).

```rust
use skg_provider_ollama::OllamaProvider;

let provider = OllamaProvider::default(); // connects to http://localhost:11434
// Use provider with react_loop() from skg-context-engine
```

### Custom host

```rust
let provider = OllamaProvider::builder()
    .base_url("http://my-gpu-box:11434")
    .model("qwen2.5:14b")
    .build()?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
