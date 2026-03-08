# Installation

## Requirements

- **Rust** edition 2024, MSRV 1.85
- **Cargo** (included with Rust)

## With Nix (recommended for contributors)

If you use Nix, the repository includes a development shell:

```bash
nix develop
```

This provides the correct Rust toolchain, `cargo`, `clippy`, `rustfmt`, and all system dependencies.

## Adding neuron to your project

The `neuron` crate is an umbrella that re-exports all layers behind feature flags. Add it to your `Cargo.toml`:

```toml
[dependencies]
neuron = { version = "0.4", features = ["op-react", "provider-anthropic", "state-memory"] }
```

### Feature flags

The umbrella crate uses feature flags to control which implementations are compiled:

| Feature | What it enables |
|---------|----------------|
| `core` | Layer 0 protocols + `neuron-turn` + `neuron-context` + `neuron-tool` (included in default) |
| `op-react` | ReAct operator (`neuron-op-react`) |
| `op-single-shot` | Single-shot operator (`neuron-op-single-shot`) |
| `provider-anthropic` | Anthropic Claude provider |
| `provider-openai` | OpenAI provider |
| `provider-ollama` | Ollama local model provider |
| `providers-all` | All three providers |
| `state-memory` | In-memory state store |
| `state-fs` | Filesystem-backed state store |
| `orch-local` | In-process orchestrator |
| `orch-kit` | Orchestration utilities |
| `env-local` | Local (passthrough) environment |
| `mcp` | MCP client integration |

### Using individual crates

You can also depend on individual crates directly if you want finer control over your dependency tree:

```toml
[dependencies]
layer0 = "0.4"
neuron-turn = "0.4"
neuron-tool = "0.4"
neuron-op-react = "0.4"
neuron-provider-anthropic = "0.4"
```

## Verifying your setup

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

All three should pass cleanly on a fresh checkout.
