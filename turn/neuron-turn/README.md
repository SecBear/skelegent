# neuron-turn

> Shared toolkit for building LLM providers and operators

[![crates.io](https://img.shields.io/crates/v/neuron-turn.svg)](https://crates.io/crates/neuron-turn)
[![docs.rs](https://docs.rs/neuron-turn/badge.svg)](https://docs.rs/neuron-turn)
[![license](https://img.shields.io/crates/l/neuron-turn.svg)](LICENSE-MIT)

## Overview

`neuron-turn` is the shared toolkit that concrete providers (Anthropic, OpenAI, Ollama) and
operators (ReAct, single-shot) build on top of. It provides:

- **`Provider` trait** — the async interface that every LLM integration implements
- **Request / response types** — `TurnRequest`, `TurnResponse`, `ContentPart`, `ToolCall`, etc.
- **Context strategy types** — `ContextStrategy` enum and its resolution logic (used by providers
  to window conversation history before sending to the model)
- **Shared cost / token accounting** — `TokenUsage`, `Cost`, `DurationMs`

## Usage

```toml
[dependencies]
neuron-turn = "0.4"
```

### Implementing a custom provider

```rust
use neuron_turn::{Provider, TurnRequest, TurnResponse};
use async_trait::async_trait;

pub struct MyProvider { /* ... */ }

#[async_trait]
impl Provider for MyProvider {
    async fn turn(&self, request: TurnRequest) -> Result<TurnResponse, neuron_turn::TurnError> {
        // call your LLM API here
        todo!()
    }
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
