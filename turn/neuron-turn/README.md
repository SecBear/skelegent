# neuron-turn

> Shared toolkit for building LLM providers and operators

[![crates.io](https://img.shields.io/crates/v/neuron-turn.svg)](https://crates.io/crates/neuron-turn)
[![docs.rs](https://docs.rs/neuron-turn/badge.svg)](https://docs.rs/neuron-turn)
[![license](https://img.shields.io/crates/l/neuron-turn.svg)](LICENSE-MIT)

## Overview

`neuron-turn` is the shared toolkit that concrete providers (Anthropic, OpenAI, Ollama) and
operators (ReAct, single-shot) build on top of. It provides:

- **`Provider` trait** — the async interface that every LLM integration implements (`Provider::infer()`)
- **Request / response types** — `InferRequest`, `InferResponse`, `ToolCall`, `ToolSchema`, `StopReason`
- **Token accounting** — `TokenUsage` with input/output/cache token counts

## Usage

```toml
[dependencies]
neuron-turn = "0.4"
```

### Implementing a custom provider

```rust
use std::future::Future;
use neuron_turn::{Provider, InferRequest, InferResponse, ProviderError};

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

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
