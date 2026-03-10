# skg-turn

> Shared toolkit for building LLM providers and operators

[![crates.io](https://img.shields.io/crates/v/skg-turn.svg)](https://crates.io/crates/skg-turn)
[![docs.rs](https://docs.rs/skg-turn/badge.svg)](https://docs.rs/skg-turn)
[![license](https://img.shields.io/crates/l/skg-turn.svg)](LICENSE-MIT)

## Overview

`skg-turn` is the shared toolkit that concrete providers (Anthropic, OpenAI, Ollama) and
operators (ReAct, single-shot) build on top of. It provides:

- **`Provider` trait** — the async interface that every LLM integration implements (`Provider::infer()`)
- **Request / response types** — `InferRequest`, `InferResponse`, `ToolCall`, `ToolSchema`, `StopReason`
- **Token accounting** — `TokenUsage` with input/output/cache token counts

## Usage

```toml
[dependencies]
skg-turn = "0.4"
```

### Implementing a custom provider

```rust
use std::future::Future;
use skg_turn::{Provider, InferRequest, InferResponse, ProviderError};

pub struct MyProvider { /* ... */ }

impl Provider for MyProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
        async move {
            // call your LLM API here
            todo!()
        }
    }
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
