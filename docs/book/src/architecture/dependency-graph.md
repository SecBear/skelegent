# Dependency Graph

This page shows how neuron's crates depend on each other. The fundamental rule is that dependencies flow downward: higher layers depend on lower layers, never the reverse.

> **Note:** The ASCII diagram below reflects the core dependency relationships but is
> incomplete — `neuron-effects-core`, `neuron-effects-local`, `neuron-turn-kit`,
> `neuron-auth`, and `neuron-crypto` are not shown. See the crate list in
> [layers.md](layers.md) for the complete and authoritative crate inventory.

## ASCII dependency graph

```
                        neuron (umbrella)
                 feature-gated re-exports of all layers
                              │
         ┌────────────────────┼────────────────────────┐
         │                    │                        │
         ▼                    ▼                        ▼
  neuron-op-react    neuron-op-single-shot     neuron-orch-local
  (Layer 1)          (Layer 1)                 (Layer 2)
    │  │  │              │  │                    │  │
    │  │  │              │  │                    │  └──► neuron-orch-kit (L2)
    │  │  │              │  │                    │         │
    │  │  └──────────────┼──┼────────────────────┘        │
    │  │                 │  │                              │
    │  ▼                 │  ▼                              │
    │  neuron-hooks ◄────┘  neuron-turn ◄─────────────────┘
    │  (Layer 5)            (Layer 1)
    │    │                    ▲  ▲  ▲
    │    │        ┌───────────┘  │  └───────────┐
    │    │        │              │              │
    │    │  neuron-provider-  neuron-provider-  neuron-provider-
    │    │  anthropic         openai            ollama
    │    │  (Layer 1)         (Layer 1)         (Layer 1)
    │    │
    ▼    ▼
  neuron-tool              neuron-mcp
  (Layer 1)                (Layer 1)
    │                        │
    │                        │
    └────────┬───────────────┘
             │
             ▼
           layer0
         (Layer 0)
             ▲
             │
    ┌────────┼──────────┬──────────────┐
    │        │          │              │
neuron-   neuron-    neuron-       neuron-
state-    state-     env-local     secret-*
memory    fs         (Layer 4)     neuron-auth-*
(Layer 3) (Layer 3)                neuron-crypto-*
                                   (Layer 4)
```

## Key relationships

### Layer 0: The foundation

`layer0` has no workspace dependencies. It depends only on:
- `serde` (serialization for protocol messages)
- `async-trait` (object-safe async traits)
- `thiserror` (ergonomic error types)
- `rust_decimal` (precise cost tracking)
- `serde_json` (for `Value` in metadata and state)

Every other crate in the workspace depends on `layer0`, directly or transitively.

### Layer 1: Operator ecosystem

The operator ecosystem has several internal dependencies:

- **`neuron-turn`** provides the `Provider` trait and shared types. All three provider crates depend on it.
- **`neuron-tool`** provides `ToolDyn` and `ToolRegistry`. It depends only on `layer0`.
- **`neuron-mcp`** depends on `neuron-tool` (it creates tools from MCP servers).
- **`neuron-op-react`** depends on `neuron-turn` (for `Provider`), `neuron-tool` (for `ToolRegistry`), and `neuron-hooks` (for `HookRegistry`).
- **`neuron-op-single-shot`** depends on `neuron-turn` and `neuron-hooks`.

### Layer 2: Orchestration

- **`neuron-orch-local`** depends on `layer0` and `neuron-orch-kit`. It holds `Arc<dyn Operator>` references.
- **`neuron-orch-kit`** provides shared utilities for orchestrator implementations.

### Layer 3: State

- **`neuron-state-memory`** and **`neuron-state-fs`** depend only on `layer0` (and `tokio` for async I/O). They are completely independent of each other and of all other layers.

### Layer 4: Environment and credentials

- **`neuron-env-local`** depends on `layer0`. It holds an `Arc<dyn Operator>`.
- The secret backends (`neuron-secret-*`), auth backends (`neuron-auth-*`), and crypto backends (`neuron-crypto-*`) depend on `neuron-secret`/`neuron-auth`/`neuron-crypto` respectively, and transitively on `layer0`.

### Layer 5: Cross-cutting

- **`neuron-hooks`** depends on `layer0` (for the `Hook` trait).
- **`neuron-hook-security`** depends on `neuron-hooks` and `layer0`.

### The umbrella

- **`neuron`** depends on everything, all behind `optional = true` with feature flags. It re-exports but adds no logic.

## External dependencies by layer

| Layer | External deps |
|-------|--------------|
| 0 | `serde`, `async-trait`, `thiserror`, `rust_decimal`, `serde_json` |
| 1 | `reqwest`, `tokio`, `serde_json`, `schemars` (tools) |
| 2 | `tokio` |
| 3 | `tokio` |
| 4 | Provider-specific SDKs (`aws-sdk`, `gcp`, `reqwest`) |
| 5 | `layer0` only (hooks are pure logic) |

## Crates not shown in the ASCII diagram

The following crates were added after the diagram was drawn and are not yet reflected in
the ASCII art above:

| Crate | Layer | Depends on |
|---|---|---|
| `neuron-turn-kit` | 1 | `layer0`, `neuron-turn` |
| `neuron-effects-core` | 2 | `layer0` |
| `neuron-effects-local` | 2 | `layer0`, `neuron-effects-core` |
| `neuron-auth` | 4 | `layer0` |
| `neuron-crypto` | 4 | `layer0` |