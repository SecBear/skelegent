# neuron

> Composable async agentic AI framework for Rust

[![crates.io](https://img.shields.io/crates/v/neuron.svg)](https://crates.io/crates/neuron)
[![docs.rs](https://docs.rs/neuron/badge.svg)](https://docs.rs/neuron)
[![license](https://img.shields.io/crates/l/neuron.svg)](LICENSE-MIT)
[![book](https://img.shields.io/badge/book-secbear.github.io%2Fneuron-blue)](https://secbear.github.io/neuron)

## Overview

`neuron` is the umbrella re-export crate for the neuron workspace. It provides a single
dependency entry point for the most common combination of crates, controlled by feature flags.

The framework is built on a **6-layer protocol model** where each layer is a Rust trait:

```
layer0: Operator | StateStore | Environment | Orchestrator | Observable
         ↓           ↓             ↓               ↓          ↓
      operators   state        credentials    workflows   events
```

Every layer is independently replaceable. You can use the Anthropic provider with the OpenAI
operator interface, swap in-memory state for filesystem state, plug in Vault secrets without
changing your operator code, and so on.

## Quick start

```toml
[dependencies]
neuron = { version = "0.4", features = ["op-react", "provider-anthropic", "state-memory", "env-local"] }
tokio = { version = "1", features = ["full"] }
```

```rust
use neuron::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    let operator = ReactOperator::new(
        Arc::new(provider),
        Arc::new(ToolRegistry::new()),
    );
    let env = LocalEnv::new(Arc::new(EnvResolver));

    let input = OperatorInput::new("What is 2 + 2?");
    let output = operator.invoke(input, &env).await?;
    println!("{}", output.content.as_text().unwrap_or_default());
    Ok(())
}
```

## Feature flags

| Flag | Includes | Description |
|------|----------|-------------|
| `core` (default) | `layer0`, `neuron-context`, `neuron-tool`, `neuron-turn` | Protocol + wiring |
| `op-react` | `core` + `neuron-op-react` | ReAct loop operator |
| `op-single-shot` | `core` + `neuron-op-single-shot` | Single-turn operator |
| `mcp` | `core` + `neuron-mcp` | MCP bridge |
| `orch-kit` | `core` + `neuron-orch-kit` | Orchestration wiring |
| `orch-local` | `orch-kit` + `neuron-orch-local` | In-process orchestrator |
| `env-local` | `core` + `neuron-env-local` | Local environment |
| `state-memory` | `core` + `neuron-state-memory` | In-memory state store |
| `state-fs` | `core` + `neuron-state-fs` | Filesystem state store |
| `provider-anthropic` | `core` + `neuron-provider-anthropic` | Anthropic Claude |
| `provider-openai` | `core` + `neuron-provider-openai` | OpenAI GPT |
| `provider-ollama` | `core` + `neuron-provider-ollama` | Ollama local models |
| `providers-all` | all three providers | All built-in providers |

## Workspace crates

The workspace is split into focused crates you can depend on individually:

### Core protocol
- [`layer0`](https://crates.io/crates/layer0) — foundational protocol traits (no implementations)
- [`neuron-turn`](https://crates.io/crates/neuron-turn) — Provider trait + LLM types
- [`neuron-tool`](https://crates.io/crates/neuron-tool) — Tool + ToolRegistry
- [`neuron-context`](https://crates.io/crates/neuron-context) — context window strategies

### Operators
- [`neuron-op-react`](https://crates.io/crates/neuron-op-react) — ReAct reasoning loop
- [`neuron-op-single-shot`](https://crates.io/crates/neuron-op-single-shot) — single model call

### Providers
- [`neuron-provider-anthropic`](https://crates.io/crates/neuron-provider-anthropic) — Claude
- [`neuron-provider-openai`](https://crates.io/crates/neuron-provider-openai) — GPT
- [`neuron-provider-ollama`](https://crates.io/crates/neuron-provider-ollama) — Ollama

### Orchestration
- [`neuron-orch-kit`](https://crates.io/crates/neuron-orch-kit) — wiring kit
- [`neuron-orch-local`](https://crates.io/crates/neuron-orch-local) — in-process orchestrator

### Middleware & security
- [`neuron-hook-security`](https://crates.io/crates/neuron-hook-security) — security middleware (redaction + DLP)

### State
- [`neuron-state-memory`](https://crates.io/crates/neuron-state-memory) — in-memory store
- [`neuron-state-fs`](https://crates.io/crates/neuron-state-fs) — filesystem store

### Environment & credentials
- [`neuron-env-local`](https://crates.io/crates/neuron-env-local) — local environment
- [`neuron-secret`](https://crates.io/crates/neuron-secret) — secret resolution traits
- [`neuron-auth`](https://crates.io/crates/neuron-auth) — authentication traits
- [`neuron-crypto`](https://crates.io/crates/neuron-crypto) — cryptographic traits

### MCP
- [`neuron-mcp`](https://crates.io/crates/neuron-mcp) — Model Context Protocol bridge

## License

`neuron` is dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

## Documentation

- [Book (architecture + guides)](https://secbear.github.io/neuron)
- [API docs (docs.rs)](https://docs.rs/neuron)
- [GitHub](https://github.com/secbear/neuron)
