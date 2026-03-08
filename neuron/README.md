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
neuron = { version = "0.4", features = ["agent", "provider-anthropic"] }
tokio = { version = "1", features = ["full"] }
```

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = neuron::agent("claude-sonnet-4-20250514")
        .system("You are a helpful assistant.")
        .build()?
        .run("What is the capital of France?")
        .await?;

    if let Some(text) = output.message.as_text() {
        println!("{text}");
    }
    Ok(())
}
```

## Feature flags

| Flag | Includes | Description |
|------|----------|-------------|
| `core` (default) | `layer0`, `neuron-context`, `neuron-tool`, `neuron-turn` | Protocol + wiring |
| `context-engine` | `core` + `neuron-context-engine` | Composable context engine |
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
- [`neuron-context-engine`](https://crates.io/crates/neuron-context-engine) — Composable context engine
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
