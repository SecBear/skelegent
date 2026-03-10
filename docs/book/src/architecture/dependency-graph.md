# Dependency Graph

This page shows how skelegent's crates depend on each other. The fundamental rule is that dependencies flow downward: higher layers depend on lower layers, never the reverse.

> **Note:** The ASCII diagram below reflects the core dependency relationships but is
> incomplete — `skg-effects-core`, `skg-effects-local`, `skg-turn-kit`,
> `skg-auth`, and `skg-crypto` are not shown. See the crate list in
> [layers.md](layers.md) for the complete and authoritative crate inventory.

## ASCII dependency graph

```
                        skelegent (umbrella)
                 feature-gated re-exports of all layers
                              │
         ┌────────────────────┼────────────────────────┐
         │                    │                        │
         ▼                    ▼                        ▼
  skg-context-engine  skg-op-single-shot     skg-orch-local
  (Layer 1)          (Layer 1)                 (Layer 2)
    │  │                 │                       │  │
    │  │                 │                       │  └──► skg-orch-kit (L2)
    │  │                 │                       │         │
    │  └─────────────────┼───────────────────────┘        │
    │                    │                                │
    │                    ▼                                │
    │                 skg-turn ◄───────────────────────┘
    │                 (Layer 1)
    │                    ▲  ▲  ▲
    │        ┌───────────┘  │  └───────────┐
    │        │              │              │
    │  skg-provider-  skg-provider-  skg-provider-
    │  anthropic         openai            ollama
    │  (Layer 1)         (Layer 1)         (Layer 1)
    │
    ▼
  skg-tool              skg-mcp
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
skg-   skg-    skg-       skg-
state-    state-     env-local     secret-*
memory    fs         (Layer 4)     skg-auth-*
(Layer 3) (Layer 3)                skg-crypto-*
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

- **`skg-turn`** provides the `Provider` trait and shared types. All three provider crates depend on it.
- **`skg-tool`** provides `ToolDyn` and `ToolRegistry`. It depends only on `layer0`.
- **`skg-mcp`** depends on `skg-tool` (it creates tools from MCP servers).
- **`skg-context-engine`** depends on `skg-turn` (for `Provider`), `skg-tool` (for `ToolRegistry`), and `layer0` (for middleware traits).
- **`skg-op-single-shot`** depends on `skg-turn` and `layer0`.

### Layer 2: Orchestration

- **`skg-orch-local`** depends on `layer0` and `skg-orch-kit`. It holds `Arc<dyn Operator>` references.
- **`skg-orch-kit`** provides shared utilities for orchestrator implementations.

### Layer 3: State

- **`skg-state-memory`** and **`skg-state-fs`** depend only on `layer0` (and `tokio` for async I/O). They are completely independent of each other and of all other layers.

### Layer 4: Environment and credentials

- **`skg-env-local`** depends on `layer0`. It holds an `Arc<dyn Operator>`.
- The secret backends (`skg-secret-*`), auth backends (`skg-auth-*`), and crypto backends (`skg-crypto-*`) depend on `skg-secret`/`skg-auth`/`skg-crypto` respectively, and transitively on `layer0`.

### Layer 5: Cross-cutting

- **`skg-hook-security`** depends on `layer0` (for middleware traits). It provides `RedactionMiddleware` and `ExfilGuardMiddleware`.
### The umbrella

- **`skelegent`** depends on everything, all behind `optional = true` with feature flags. It re-exports but adds no logic.

## External dependencies by layer

| Layer | External deps |
|-------|--------------|
| 0 | `serde`, `async-trait`, `thiserror`, `rust_decimal`, `serde_json` |
| 1 | `reqwest`, `tokio`, `serde_json`, `schemars` (tools) |
| 2 | `tokio` |
| 3 | `tokio` |
| 4 | Provider-specific SDKs (`aws-sdk`, `gcp`, `reqwest`) |
| 5 | `layer0` only (middleware is pure logic) |

## Crates not shown in the ASCII diagram

The following crates were added after the diagram was drawn and are not yet reflected in
the ASCII art above:

| Crate | Layer | Depends on |
|---|---|---|
| `skg-turn-kit` | 1 | `layer0`, `skg-turn` |
| `skg-effects-core` | 2 | `layer0` |
| `skg-effects-local` | 2 | `layer0`, `skg-effects-core` |
| `skg-auth` | 4 | `layer0` |
| `skg-crypto` | 4 | `layer0` |