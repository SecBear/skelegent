# skg-provider-openai

> OpenAI API provider for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-provider-openai.svg)](https://crates.io/crates/skg-provider-openai)
[![docs.rs](https://docs.rs/skg-provider-openai/badge.svg)](https://docs.rs/skg-provider-openai)
[![license](https://img.shields.io/crates/l/skg-provider-openai.svg)](LICENSE-MIT)

## Overview

`skg-provider-openai` implements the `Provider` trait from
[`skg-turn`](../../turn/skg-turn) for the
[OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat). It handles
request serialization, streaming-free response parsing, tool call routing, and cost accounting
for GPT models.

Supports: `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1`, `o3-mini`, and any model that
speaks the Chat Completions protocol (including many open-source proxies).

## Usage

```toml
[dependencies]
skg-provider-openai = "0.4"
skg-turn = "0.4"
```

### Setup

Set `OPENAI_API_KEY` in your environment (or inject via `skg-env-local`).

```rust
use skg_provider_openai::OpenAIProvider;

let provider = OpenAIProvider::from_env()?;
// Use provider with react_loop() from skg-context-engine
```

### OpenAI-compatible endpoints (Ollama, vLLM, LM Studio, etc.)

```rust
let provider = OpenAIProvider::builder()
    .api_key("not-needed")
    .base_url("http://localhost:11434/v1")
    .model("llama3.2")
    .build()?;
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
