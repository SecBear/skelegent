# The 6-Layer Model

neuron organizes its crates into six layers plus an umbrella crate. Each layer has a clear responsibility. The fundamental rule: **higher layers depend on lower layers, never the reverse.**

```
 ┌──────────────────────────────────────────────────┐
 │            neuron (umbrella crate)                │
 │         Feature-gated re-exports of all layers    │
 ├──────────────────────────────────────────────────┤
 │  LAYER 5 — Cross-Cutting                         │
 │  Security middleware, lifecycle coordination       │
 ├──────────────────────────────────────────────────┤
 │  LAYER 4 — Environment                           │
 │  Isolation, credentials, secret backends,         │
 │  auth backends, crypto backends                   │
 ├──────────────────────────────────────────────────┤
 │  LAYER 3 — State                                 │
 │  Persistence backends (memory, filesystem)        │
 ├──────────────────────────────────────────────────┤
 │  LAYER 2 — Orchestration                         │
 │  Multi-agent composition (local, kit)             │
 ├──────────────────────────────────────────────────┤
 │  LAYER 1 — Operator Implementations              │
 │  Providers, tools, operators, context, MCP        │
 ├──────────────────────────────────────────────────┤
 │  LAYER 0 — Protocol Traits (layer0)              │
 │  4 protocols + 2 interfaces + message types       │
 │  The stability contract. Changes: almost never.   │
 └──────────────────────────────────────────────────┘
```

## Layer 0 -- Protocol Traits

**Crate:** `layer0`

Layer 0 is the stability contract. It defines the four protocol traits (`Operator`, `Orchestrator`, `StateStore`/`StateReader`, `Environment`), two cross-cutting interfaces (per-boundary middleware traits, lifecycle events), and all the message types that cross protocol boundaries (`OperatorInput`, `OperatorOutput`, `Content`, `Effect`, `Scope`, typed IDs).

**Dependencies:** `serde`, `async-trait`, `thiserror`, `rust_decimal`, `serde_json`. Nothing else. No runtime, no HTTP, no provider-specific types.

**Change frequency:** Almost never. Adding a method to a protocol trait is a breaking change that ripples through every implementation. The traits were designed with extension points (`#[non_exhaustive]` enums, `serde_json::Value` metadata fields) to avoid needing changes.

## Layer 1 -- Operator Implementations

**Crates:**
- `neuron-turn` -- Shared toolkit: `Provider` trait, `InferRequest`, `InferResponse`, `TokenUsage`, type conversions
- `neuron-turn-kit` -- Turn decomposition primitives and helpers
- `neuron-provider-anthropic` -- Anthropic Claude API provider
- `neuron-provider-openai` -- OpenAI API provider
- `neuron-provider-ollama` -- Ollama local model provider
- `neuron-tool` -- `ToolDyn` trait, `ToolRegistry`, `AliasedTool`
- `neuron-context` -- Conversation context management and compaction strategies
- `neuron-mcp` -- MCP (Model Context Protocol) client
- `neuron-context-engine` -- Composable three-phase context engine (assembly, inference, reaction) with tool execution
- `neuron-op-single-shot` -- Single-shot operator (one model call, no tools)

Layer 1 is where the core agentic loop lives. The `Provider` trait (defined in `neuron-turn`) is intentionally not object-safe -- it uses RPITIT for zero-cost abstraction. The object-safe boundary is `layer0::Operator`. The bridge is a context engine implementation (generic over the provider type) that implements the object-safe `Operator` trait.

## Layer 2 -- Orchestration

**Crates:**
- `neuron-orch-local` -- In-process orchestrator using tokio tasks
- `neuron-orch-kit` -- Shared orchestration utilities
- `neuron-effects-core` -- `EffectExecutor` trait and shared effect execution types
- `neuron-effects-local` -- Local effect interpreter (executes effects in-process)

Layer 2 implements `layer0::Orchestrator`. The `LocalOrch` dispatches operator invocations in-process using tokio. It maps `AgentId` to `Arc<dyn Operator>` and handles parallel dispatch via `tokio::spawn`. The effects crates execute `Effect` payloads declared by operators — they live at Layer 2 because effect execution is an orchestration concern, not a protocol concern.

Future implementations could include Temporal workflows (durable, replayable) or Restate (durable execution with virtual objects).

## Layer 3 -- State

**Crates:**
- `neuron-state-memory` -- In-memory `HashMap` store (ephemeral, good for tests)
- `neuron-state-fs` -- Filesystem-backed store (durable across restarts)

Layer 3 implements `layer0::StateStore`. Both backends provide scoped key-value storage with `serde_json::Value` values. The memory store is ideal for testing and short-lived processes. The filesystem store persists data as files, suitable for CLI tools and local development.

Future implementations could include SQLite (embedded), PostgreSQL (queryable, transactional), or Redis (networked, fast).

## Layer 4 -- Environment

**Crates:**
- `neuron-env-local` -- Local passthrough environment (no isolation)
- `neuron-secret` -- Secret resolution trait
- `neuron-secret-vault` -- HashiCorp Vault secrets
- `neuron-auth` -- Authentication and credential framework
- `neuron-crypto` -- Cryptographic primitives

Layer 4 implements `layer0::Environment` and provides the credential infrastructure that environments use. `LocalEnv` passes through with no isolation -- it holds an `Arc<dyn Operator>` and calls `execute()` directly. The secret, auth, and crypto backends provide credential resolution for the `EnvironmentSpec`'s `CredentialRef` system.

## Layer 5 -- Cross-Cutting

**Crates:**
- `neuron-hook-security` -- Security middleware (`RedactionMiddleware`, `ExfilGuardMiddleware`)

Layer 5 provides security middleware that wraps operator dispatch, store access, and execution boundaries. The per-boundary middleware traits (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`) are defined in Layer 0 and composed into stacks (`DispatchStack`, `StoreStack`, `ExecStack`). Layer 5 crates supply concrete middleware implementations — for example, `RedactionMiddleware` scrubs sensitive data from model outputs, and `ExfilGuardMiddleware` blocks unauthorized data exfiltration through tool calls.

## The umbrella crate

**Crate:** `neuron`

The umbrella crate re-exports all layers behind feature flags. It exists so users can write `neuron = { features = ["context-engine", "provider-anthropic"] }` instead of depending on 5+ individual crates. See [Installation](../getting-started/installation.md) for the full feature flag table.

## Dependency rules

1. A crate may depend on crates at the same layer or lower layers.
2. A crate may **never** depend on a crate at a higher layer.
3. All crates depend on `layer0` (directly or transitively).
4. `layer0` depends on nothing in the workspace.

These rules ensure that any layer can be replaced independently. You can swap your state backend without touching your operator code. You can swap your orchestrator without touching your tools. The protocol traits in Layer 0 are the only shared vocabulary.
