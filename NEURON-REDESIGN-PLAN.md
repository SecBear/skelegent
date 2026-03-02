# Neuron Workspace Redesign Plan

> **⚠️ Historical document.** This was the original redesign plan. The redesign is now implemented.
> Crate names referenced here (`neuron-types`, `neuron-loop`, `neuron-runtime`, `neuron-otel`) no longer
> exist in the workspace. For current architecture, see [`SPECS.md`](SPECS.md) and the
> [mdbook](docs/book/).

> NOTE: This file is preserved for historical context. It is not the active source of truth. For current architecture and decisions, see `SPECS.md`, the mdbook, and rules.
---

## What We're Doing

Redesigning the `neuron` Rust workspace (https://github.com/secbear/neuron, crates.io: `neuron` v0.3.0) to implement the full 6-layer composable agentic architecture defined in our source documents.

**The existing `layer0` crate** (currently in `/Users/bear/dev/layer0/`) becomes the foundation of the neuron workspace. Everything else builds on top.

**The existing neuron crates** (v0.3.0) are redesigned to implement layer0's protocol traits. The current neuron-types is replaced by layer0. The current neuron-loop, neuron-provider-*, etc. are rewritten against the new trait definitions.

**Target branch:** `redesign/v2` on the neuron repo.

---

## Why

The current neuron (v0.3.0) has excellent Layer 1 implementations (providers, loop, tools, MCP) but lacks:
- A stability contract (no trait crate, types are implementation-coupled)
- Orchestration (no multi-agent composition, no durability)
- State protocol (sessions exist but no abstract StateStore/StateReader)
- Environment protocol (sandbox trait exists but minimal)
- Lifecycle coordination (no budget events, no compaction coordination)

Layer 0 provides the missing foundation. The redesign wires the existing neuron implementations to the new protocol traits, then fills the missing layers.

---

## Source Documents

These documents define WHAT to build and WHY:

| Document | What it is | Location |
|---|---|---|
| `DEVELOPMENT-LOG.md` | Complete history of all decisions, research, and rationale | `/Users/bear/dev/layer0/` |
| `HANDOFF.md` | Implementation spec — trait signatures, types, module structure, layer definitions | `/Users/bear/dev/layer0/` |
| `composable-agentic-architecture.md` | Design rationale — 4 protocols + 2 interfaces, gap analysis, coverage map | `/Users/bear/dev/layer0/` |
| `agentic-decision-map-v3.md` | Full design space — all 23 architectural decisions | `/Users/bear/dev/layer0/` |
| `validation-and-coordination.md` | Coordination patterns between protocols | `/Users/bear/dev/layer0/` |

---

## The 6-Layer Architecture

```
LAYER 0 — layer0 (trait crate)
  Protocol traits + message types. Changes: almost never.
  Crate: layer0

LAYER 1 — Turn Implementations
  The ReAct loop, providers, tools, context, MCP.
  Crates: neuron-loop, neuron-provider-*, neuron-tool, neuron-context, neuron-mcp

LAYER 2 — Orchestration Implementations
  Agent composition, durability, topology.
  Crates: neuron-orch-local, neuron-orch-temporal (future), neuron-orch-restate (future)

LAYER 3 — State Implementations
  Persistence backends.
  Crates: neuron-state-memory, neuron-state-fs, neuron-state-sqlite (future), neuron-state-postgres (future)

LAYER 4 — Environment Implementations
  Isolation, credentials, resource constraints.
  Crates: neuron-env-local, neuron-env-docker (future), neuron-env-k8s (future)

LAYER 5 — Cross-Cutting
  Hook registry, lifecycle coordination, observability.
  Crates: neuron-hooks, neuron-otel

UMBRELLA — neuron
  Re-exports everything with feature flags.
```

---

## Workspace Structure (Target)

```
neuron/                              # workspace root
  Cargo.toml                         # workspace manifest
  CLAUDE.md                          # development rules
  DEVELOPMENT-LOG.md                 # full history (copied from layer0)
  NEURON-REDESIGN-PLAN.md            # this document

  # Source documents (from layer0, moved here)
  docs/
    architecture/
      HANDOFF.md
      composable-agentic-architecture.md
      agentic-decision-map-v3.md
      validation-and-coordination.md

  # Layer 0 — Protocol traits
  layer0/
    Cargo.toml                       # name = "layer0"
    src/                             # EXACTLY as it exists today
      lib.rs, turn.rs, orchestrator.rs, state.rs, environment.rs,
      hook.rs, lifecycle.rs, effect.rs, content.rs, error.rs,
      id.rs, duration.rs, test_utils/
    tests/
      phase1.rs, phase2.rs

  # Layer 1 — Turn implementations
  neuron-loop/                       # the agentic while-loop
    Cargo.toml                       # depends on: layer0
    src/

  neuron-provider-anthropic/         # Anthropic Claude provider
    Cargo.toml                       # depends on: layer0 (for types), internal Provider trait
    src/

  neuron-provider-openai/            # OpenAI provider
    Cargo.toml
    src/

  neuron-provider-ollama/            # Ollama local provider
    Cargo.toml
    src/

  neuron-tool/                       # tool registry + middleware
    Cargo.toml                       # depends on: layer0
    src/

  neuron-tool-macros/                # proc macro for #[neuron_tool]
    Cargo.toml
    src/

  neuron-context/                    # context assembly + compaction strategies
    Cargo.toml                       # depends on: layer0
    src/

  neuron-mcp/                        # MCP client/server/bridge
    Cargo.toml                       # depends on: layer0, neuron-tool
    src/

  # Layer 2 — Orchestration implementations
  neuron-orch-local/                 # in-process orchestration (no durability)
    Cargo.toml                       # depends on: layer0
    src/

  # Layer 3 — State implementations
  neuron-state-memory/               # in-memory HashMap store
    Cargo.toml                       # depends on: layer0
    src/

  neuron-state-fs/                   # filesystem-backed store
    Cargo.toml                       # depends on: layer0
    src/

  # Layer 4 — Environment implementations
  neuron-env-local/                  # passthrough, no isolation
    Cargo.toml                       # depends on: layer0
    src/

  # Layer 5 — Cross-cutting
  neuron-hooks/                      # hook registry + composition
    Cargo.toml                       # depends on: layer0
    src/

  neuron-otel/                       # OpenTelemetry instrumentation
    Cargo.toml                       # depends on: layer0, neuron-hooks
    src/

  # Umbrella crate
  neuron/                            # re-exports with feature flags
    Cargo.toml
    src/lib.rs
```

---

## Key Design Decisions for the Redesign

### D1: Object Safety Boundary

**Layer 0 traits (object-safe, async-trait):**
- `Turn`, `Orchestrator`, `StateStore`, `StateReader`, `Environment`, `Hook`
- These MUST work as `dyn Trait` for composition (`Box<dyn Turn>`, `Arc<dyn Turn>`)
- Uses `#[async_trait]` — confirmed correct via research (dyn-safe async fn in traits not stabilized as of Feb 2026)

**Layer 1+ internal traits (RPITIT, not object-safe):**
- `Provider` (LLM API client) — used internally by NeuronTurn, never needs `dyn Provider`
- `ContextStrategy` — used internally by context assembly
- These can use native async traits (RPITIT) for zero-cost abstraction

**The bridge:** `NeuronTurn` implements `layer0::Turn` (object-safe) while internally using `Provider` (RPITIT, not object-safe). The protocol boundary is object-safe. Implementation internals are whatever's fastest.

### D2: What Happens to neuron-types

`neuron-types` is **replaced** by `layer0`. The types that exist in both are reconciled:

| neuron-types concept | layer0 equivalent | Action |
|---|---|---|
| `Message { role, content }` | `Content`, `ContentBlock` | Use layer0's Content model. Adapt neuron's Message to wrap it. |
| `CompletionRequest/Response` | Not in layer0 (internal to Turn) | Keep in neuron-loop as provider-internal types. NOT part of the protocol. |
| `Provider` trait | `Turn` trait | Provider becomes Layer 1 internal. Turn is the protocol boundary. |
| `Tool` / `ToolDyn` | `ContentBlock::ToolUse/ToolResult` | layer0 defines tool USE (the protocol). neuron-tool defines tool EXECUTION (the implementation). |
| `ObservabilityHook` | `Hook` trait | Reconcile. layer0's Hook has typed HookPoints; neuron's has HookEvent. Merge to layer0's design. |
| `DurableContext` | Not in layer0 (orchestration internal) | Keep in neuron-orch-temporal as internal concern. |
| `PermissionPolicy` | `HookAction::SkipTool` | Subsume into Hook system. |
| `SessionStorage` | `StateStore` | Replace with layer0's StateStore. |
| `ProviderError` | `TurnError` | Map ProviderError variants to TurnError variants. |
| `StreamEvent/StreamHandle` | Not in layer0 (Layer 1) | Keep in neuron-loop. Streaming is delivery, not protocol. |
| `ToolRegistry` | Not in layer0 (Layer 1) | Keep in neuron-tool. |

### D3: What Stays, What Changes, What's New

**Stays (rewritten against layer0 traits):**
- neuron-loop → implements `layer0::Turn`
- neuron-provider-anthropic/openai/ollama → internal to neuron-loop
- neuron-tool + neuron-tool-macros → tool registry, middleware pipeline
- neuron-context → context assembly strategies
- neuron-mcp → MCP client/server/bridge
- neuron-otel → implements `layer0::Hook`

**Changes fundamentally:**
- neuron-types → deleted, replaced by layer0
- neuron-runtime → split into:
  - Sessions → neuron-state-memory + neuron-state-fs (Layer 3)
  - Guardrails → neuron-hooks (Layer 5)
  - Durability → neuron-orch-local (Layer 2)
  - Sandbox → neuron-env-local (Layer 4)

**New:**
- layer0 (moved from separate repo)
- neuron-orch-local (Layer 2 — in-process orchestration)
- neuron-state-memory (Layer 3 — in-memory store)
- neuron-state-fs (Layer 3 — filesystem store)
- neuron-env-local (Layer 4 — passthrough environment)
- neuron-hooks (Layer 5 — hook registry and composition)

### D4: Dependency Graph

```
layer0                              (serde, async-trait, thiserror, rust_decimal)
    ↑
    ├── neuron-provider-anthropic    (reqwest, tokio, serde_json)
    ├── neuron-provider-openai       (reqwest, tokio, serde_json)
    ├── neuron-provider-ollama       (reqwest, tokio, serde_json)
    ├── neuron-context               (layer0 only)
    ├── neuron-tool                  (layer0, schemars)
    │   ├── neuron-tool-macros       (proc-macro, syn, quote)
    │   └── neuron-mcp              (layer0, neuron-tool, rmcp)
    ├── neuron-state-memory          (layer0, tokio)
    ├── neuron-state-fs              (layer0, tokio)
    ├── neuron-env-local             (layer0)
    ├── neuron-hooks                 (layer0)
    │   └── neuron-otel             (layer0, neuron-hooks, opentelemetry, tracing)
    └── neuron-orch-local            (layer0, tokio)
        ↑
    neuron-loop                      (layer0, neuron-tool, neuron-context, provider crates)
        ↑
    neuron                           (umbrella, feature-gated re-exports)
```

### D5: neuron-loop Implements Turn

The critical bridge. `neuron-loop`'s `AgentLoop` becomes a `Turn` implementation:

```rust
// neuron-loop/src/lib.rs
use layer0::turn::{Turn, TurnInput, TurnOutput};
use layer0::error::TurnError;

pub struct NeuronTurn<P: Provider> {
    provider: P,
    tools: ToolRegistry,
    context_strategy: Box<dyn ContextStrategy>,
    hooks: Vec<BoxedHook>,
    config: LoopConfig,
}

#[async_trait]
impl<P: Provider + Send + Sync + 'static> Turn for NeuronTurn<P> {
    async fn execute(&self, input: TurnInput) -> Result<TurnOutput, TurnError> {
        // The ReAct loop:
        // 1. Convert TurnInput to provider's CompletionRequest
        // 2. Assemble context (identity, history, tools)
        // 3. Loop: call provider, check for tool_use, execute tools, backfill
        // 4. Build TurnOutput with effects, metadata, exit reason
    }
}
```

`Provider` stays as a non-object-safe internal trait. It's never exposed at protocol boundaries.

### D6: Distributed Scenario Support

The architecture supports single laptop to global distributed with the same traits:

**Single PC:** `NeuronTurn` + `LocalOrchestrator` + `InMemoryStore` + `LocalEnvironment` + `LoggingHook`

**Global distributed:** Same `NeuronTurn` + `TemporalOrchestrator` + `PostgresStore` + `K8sEnvironment` + `OtelHook` + `BudgetCoordinator`

**Sub-agents in different environments:** Orchestrator maps AgentId → (Turn + EnvironmentSpec). Each agent can have different isolation.

**Agents across the internet:** All layer0 types are `Serialize + Deserialize`. An A2A-based orchestrator serializes TurnInput/TurnOutput over HTTP.

**Shared state across environments:** Network-accessible StateStore (Postgres/Redis) or pre-assembled context in TurnInput (Codex pattern).

---

## Phased Implementation Plan

### Phase 1: Foundation (Move Layer 0 into Neuron Workspace)

1. On the `redesign/v2` branch, clear existing content
2. Set up workspace Cargo.toml with layer0 as first member
3. Copy layer0 source verbatim into `layer0/` directory
4. Move source documents into `docs/architecture/`
5. Copy DEVELOPMENT-LOG.md and this plan to workspace root
6. Write new CLAUDE.md for the workspace
7. Verify: `cargo build && cargo test --features test-utils && cargo clippy -- -D warnings`
8. Commit

### Phase 2: State Implementations (Layer 3)

Extract from neuron-runtime's session storage into proper StateStore implementations.

1. Create `neuron-state-memory` — implements `layer0::StateStore` with `HashMap<(Scope, String), Value>`
2. Create `neuron-state-fs` — implements `layer0::StateStore` with filesystem backend
3. Tests: serde roundtrip, scope isolation, concurrent access
4. Commit

### Phase 3: Environment Implementations (Layer 4)

1. Create `neuron-env-local` — passthrough, implements `layer0::Environment`
   - Owns an `Arc<dyn Turn>`, calls `turn.execute(input)` directly
   - Same as current layer0 test-utils `LocalEnvironment` but as a real crate
2. Tests: passthrough execution, error propagation
3. Commit

### Phase 4: Orchestration Implementations (Layer 2)

1. Create `neuron-orch-local` — in-process orchestration
   - Implements `layer0::Orchestrator`
   - HashMap of `AgentId → (Arc<dyn Turn>, EnvironmentSpec)`
   - `dispatch`: look up agent, get environment, call `env.run(input, &spec)`
   - `dispatch_many`: tokio::spawn concurrent dispatch
   - `signal`: channel-based signaling
   - `query`: callback-based queries
2. Tests: single dispatch, multi-dispatch, agent not found, signal/query
3. Commit

### Phase 5: Cross-Cutting (Layer 5)

1. Create `neuron-hooks` — hook registry and composition
   - `HookRegistry` — registers hooks, dispatches events at hook points
   - Middleware-like composition (ordered pipeline)
   - Built-in hooks: LoggingHook, BudgetHook
2. Adapt `neuron-otel` — implement `layer0::Hook` trait
3. Tests: hook ordering, halt propagation, budget tracking
4. Commit

### Phase 6: Turn Implementation Bridge (Layer 1)

This is the largest phase — adapting neuron-loop to implement `layer0::Turn`.

1. Adapt `neuron-tool` — tool registry works with layer0's `ContentBlock::ToolUse/ToolResult`
2. Adapt `neuron-context` — context strategies work with layer0's `TurnInput/TurnOutput`
3. Adapt `neuron-provider-*` — providers stay internal, map to/from layer0 types
4. Rewrite `neuron-loop` — `NeuronTurn` implements `layer0::Turn`
5. Adapt `neuron-mcp` — MCP bridge creates tools compatible with new tool system
6. Tests: full agentic loop through layer0 trait, provider integration
7. Commit

### Phase 7: Umbrella Crate

1. Rewrite `neuron/src/lib.rs` — re-export all layers with feature flags
2. Write new README.md explaining the 6-layer architecture
3. Update CLAUDE.md with new workspace conventions
4. Full integration tests: compose all layers, run end-to-end
5. Commit

### Phase 8: Documentation and Polish

1. Update DEVELOPMENT-LOG.md with redesign completion
2. Each crate gets a README.md with key types and usage examples
3. `cargo doc --no-deps` generates clean documentation
4. Remove any remaining references to old neuron-types
5. Final verification: build, test, clippy, doc all pass
6. Tag as v0.4.0-alpha or v1.0.0-alpha (depending on stability commitment)

---

## What NOT To Do

- Do NOT add providers beyond Anthropic/OpenAI/Ollama in the initial redesign
- Do NOT add Temporal/Restate orchestration yet — just local orchestration first
- Do NOT add Docker/K8s/Wasm environments yet — just local environment first
- Do NOT add database-backed state stores yet — just memory and filesystem first
- Do NOT build a CLI, TUI, or web interface
- Do NOT build a workflow/DAG engine
- Do NOT change layer0's trait signatures — they are the stability contract
- Do NOT make layer0 traits non-object-safe
- Do NOT add dependencies to layer0 beyond what's already there

---

## Verification Checklist (Every Phase)

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo doc --no-deps
```

All four must pass before any commit.

---

## Risk: Context Window Continuity

This redesign will span multiple sessions. To maintain continuity:

1. **This plan is the source of truth.** Every session starts by reading this document.
2. **DEVELOPMENT-LOG.md tracks what's been done.** Update it after each phase.
3. **Commits are frequent and descriptive.** Each phase ends with a commit.
4. **No undocumented decisions.** If you deviate from this plan, update the plan first.
5. **The phased approach is sequential.** Don't skip ahead. Each phase builds on the previous.
