# skelegent

> Composable async agentic AI framework for Rust

[![crates.io](https://img.shields.io/crates/v/skelegent.svg)](https://crates.io/crates/skelegent)
[![docs.rs](https://docs.rs/skelegent/badge.svg)](https://docs.rs/skelegent)
[![license](https://img.shields.io/crates/l/skelegent.svg)](LICENSE-MIT)
[![book](https://img.shields.io/badge/book-secbear.github.io%2Fskelegent-blue)](https://secbear.github.io/skelegent)

## Overview

`skelegent` is the umbrella re-export crate for the skelegent workspace. It provides a single
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
skelegent = { version = "0.4", features = ["agent", "provider-anthropic"] }
tokio = { version = "1", features = ["full"] }
```

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = skelegent::agent("claude-sonnet-4-20250514")
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
| `core` (default) | `layer0`, `skg-context`, `skg-tool`, `skg-turn` | Protocol + wiring |
| `context-engine` | `core` + `skg-context-engine` | Composable context engine |
| `op-single-shot` | `core` + `skg-op-single-shot` | Single-turn operator |
| `mcp` | `core` + `skg-mcp` | MCP bridge |
| `orch-kit` | `core` + `skg-orch-kit` | Orchestration wiring |
| `orch-local` | `orch-kit` + `skg-orch-local` | In-process orchestrator |
| `env-local` | `core` + `skg-env-local` | Local environment |
| `state-memory` | `core` + `skg-state-memory` | In-memory state store |
| `state-fs` | `core` + `skg-state-fs` | Filesystem state store |
| `provider-anthropic` | `core` + `skg-provider-anthropic` | Anthropic Claude |
| `provider-openai` | `core` + `skg-provider-openai` | OpenAI GPT |
| `provider-ollama` | `core` + `skg-provider-ollama` | Ollama local models |
| `providers-all` | all three providers | All built-in providers |
| `agent` | `context-engine` + `state-memory` | High-level agent API |
| `macros` | `skg-tool/macros` | Proc-macro support for deriving ToolDyn |

## Workspace crates

The workspace is split into focused crates you can depend on individually:

### Core protocol
- [`layer0`](https://crates.io/crates/layer0) — foundational protocol traits (no implementations)
- [`skg-turn`](https://crates.io/crates/skg-turn) — Provider trait + LLM types
- [`skg-tool`](https://crates.io/crates/skg-tool) — Tool + ToolRegistry
- [`skg-context`](https://crates.io/crates/skg-context) — context window strategies

### Operators
- [`skg-context-engine`](https://crates.io/crates/skg-context-engine) — Composable context engine
- [`skg-op-single-shot`](https://crates.io/crates/skg-op-single-shot) — single model call

### Providers
- [`skg-provider-anthropic`](https://crates.io/crates/skg-provider-anthropic) — Claude
- [`skg-provider-openai`](https://crates.io/crates/skg-provider-openai) — GPT
- [`skg-provider-ollama`](https://crates.io/crates/skg-provider-ollama) — Ollama

### Orchestration
- [`skg-orch-kit`](https://crates.io/crates/skg-orch-kit) — wiring kit
- [`skg-orch-local`](https://crates.io/crates/skg-orch-local) — in-process orchestrator

### Middleware & security
- [`skg-hook-security`](https://crates.io/crates/skg-hook-security) — security middleware (redaction + DLP)

### State
- [`skg-state-memory`](https://crates.io/crates/skg-state-memory) — in-memory store
- [`skg-state-fs`](https://crates.io/crates/skg-state-fs) — filesystem store

### Environment & credentials
- [`skg-env-local`](https://crates.io/crates/skg-env-local) — local environment
- [`skg-secret`](https://crates.io/crates/skg-secret) — secret resolution traits
- [`skg-auth`](https://crates.io/crates/skg-auth) — authentication traits
- [`skg-crypto`](https://crates.io/crates/skg-crypto) — cryptographic traits

### MCP
- [`skg-mcp`](https://crates.io/crates/skg-mcp) — Model Context Protocol bridge

## License

`skelegent` is dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

## Documentation

- [Book (architecture + guides)](https://secbear.github.io/skelegent)
- [API docs (docs.rs)](https://docs.rs/skelegent)
- [GitHub](https://github.com/secbear/skelegent)
