# Layer 0 Development Log

This document captures every decision, research finding, and implementation step that produced the current codebase. Written for a new context agent to understand exactly what exists and why.

---

## What This Crate Is

`layer0` is a Rust crate defining **protocol traits** for composable agentic AI systems. It defines the boundaries between components — not the components themselves. Think of it as Tower's `Service` trait, but for agentic AI: it defines WHAT operations exist, not HOW they're implemented.

**The 4 protocols + 2 interfaces:**

| # | Name | Trait/Types | What it defines |
|---|------|------------|-----------------|
| 1 | Turn | `Turn` | What one agent does per cycle |
| 2 | Orchestration | `Orchestrator` | How agents compose + durability |
| 3 | State | `StateStore`, `StateReader` | How data persists across turns |
| 4 | Environment | `Environment` | Isolation, credentials, resources |
| 5 | Hooks | `Hook` | Observation + intervention |
| 6 | Lifecycle | `BudgetEvent`, `CompactionEvent`, `ObservableEvent` | Cross-layer coordination |

**Core design principle:** "Operation-defined, not mechanism-defined." `Turn::execute` means "cause this agent to process one cycle" — not "make an API call." This is what makes implementations swappable.

---

## Source Documents

Three documents drove every design decision. They live in the repo root:

1. **`HANDOFF.md`** — The implementation spec. Every trait signature, type definition, module structure, and phased build plan. This is what was built from.

2. **`composable-agentic-architecture.md`** — The design rationale. Why 4 protocols + 2 interfaces (not 3 protocols). The gap analysis. The coverage map proving all 23 decisions are handled.

3. **`agentic-decision-map-v3.md`** — The full design space. All 23 architectural decisions every agentic system makes (D1-D5 Turn layer, C1-C5 Composition layer, L1-L5 Lifecycle layer). Read when you need to understand what a trait must accommodate.

**`CLAUDE.md`** contains build commands and rules for working on this crate.

---

## Implementation History

### Session 1: Phase 1 + Phase 2 Implementation

Built the entire crate from scratch following HANDOFF.md:

**Phase 1** — Protocol traits and message types:
- All 4 protocol traits (Turn, Orchestrator, StateStore/StateReader, Environment)
- All message types (TurnInput, TurnOutput, TurnConfig, TurnMetadata, Content, Effect, etc.)
- Hook interface (Hook trait, HookPoint, HookAction, HookContext)
- Lifecycle events (BudgetEvent, CompactionEvent, ObservableEvent)
- Error types (TurnError, OrchError, StateError, EnvError, HookError)
- Typed IDs (AgentId, SessionId, WorkflowId, ScopeId)
- 65 acceptance tests (serde roundtrips, object safety, error Display)

**Phase 2** — Test utility implementations (behind `test-utils` feature flag):
- `EchoTurn` — returns input as output
- `LocalOrchestrator` — in-process dispatch with HashMap of agents
- `InMemoryStore` — HashMap-backed state store
- `LocalEnvironment` — passthrough execution
- `LoggingHook` — records all hook events
- 25 integration tests proving all traits compose

### Session 2: Wire-Format Stability + Audit

**Wire-format fixes:**
- `std::time::Duration` → `DurationMs(u64)` newtype everywhere. Duration serializes as `{"secs":N,"nanos":N}` which is fragile. DurationMs serializes as a plain integer (milliseconds). Added `src/duration.rs`.
- `rust_decimal` with `serde-str` feature — costs serialize as `"0.005"` (string), not `0.005` (float). Prevents precision loss.
- `Content` uses `#[serde(untagged)]` — `Text` serializes as a bare string, `Blocks` as an array. Structurally distinct, no tag needed.
- `ImageSource` uses tagged `{type, data/url}` instead of spec's untagged format — more explicit deserialization.

**Full audit against source documents** confirmed 100% coverage of all 23 architectural decisions.

### Session 3: Design Review + Hardening Plan

The user did a design review and raised 7 issues. A 5-phase hardening plan was created:

**Phase A** (`f837369`): Added `#[non_exhaustive]` to all 22 public enums. Zero test impact — `#[non_exhaustive]` on enums only affects external crate pattern matching.

**Phase B** (`a2beabe`): Added `CompactionEvent::ProviderManaged` variant for server-side context compaction (e.g., Anthropic's compaction). TDD — wrote failing test first.

**Phase C** (`4111727`): Added `#[non_exhaustive]` to all 15 public structs + constructor methods for each. This was the largest change — external tests can't use struct literal syntax on `#[non_exhaustive]` structs, so every struct literal in `tests/phase1.rs` and `tests/phase2.rs` was migrated to use constructors (`TurnInput::new(...)`, `TurnOutput::new(...)`, etc.).

**Phase D** (`d1927f7`): Changed `Environment::run` from `&dyn Turn` to `Arc<dyn Turn>` for concurrent dispatch. Changed `LocalOrchestrator` from `Box<dyn Turn>` to `Arc<dyn Turn>` with true concurrent dispatch via `tokio::spawn` in `dispatch_many`.

**Phase E** (`c78ab90`): Documentation — `serde_json::Value` coupling rationale, `async-trait` migration plan, sub-turn event streaming future note.

### Session 4: Deep Research Audit + Environment Fix

#### The Comprehensive Audit

Three parallel exploration agents analyzed:
1. **Design doc alignment** — Cross-referenced every claim in the 3 source docs against the implementation. Found 95%+ alignment with 6 intentional changes from spec (all justified).
2. **Test coverage gaps** — Found significant holes: HookAction serde untested, BudgetEvent variants untested, error Display only 5 of 24 variants, Arc safety only for Turn, Orchestrator::signal/query never tested.
3. **Ecosystem comparison** — Compared against Rig, AutoAgents, swarms-rs, langchain-rust, MCP SDKs, Tower.

#### Deep Research on Missing Primitives

Four research agents investigated whether 5 "gaps" belong in Layer 0 or elsewhere:

**1. Streaming Interface → NOT Layer 0**

Researched: Tower, Temporal, Restate, MCP, A2A, Crux/Elm, OpenAI Agents SDK, Google ADK, Anthropic API.

Conclusion: Streaming is a delivery mechanism (HOW), not an operation (WHAT). `Turn::execute` returns a complete `TurnOutput` with declared effects, metadata, and exit reason — all properties of the complete turn. Streaming tokens mid-turn would break the declared-effects pattern (effects must be atomic), conflict with orchestrator decision-making (needs complete output), and be incompatible with durable execution (Temporal can't journal a token stream).

The Hook interface already provides the right Layer 0 extension point. Streaming belongs in Layer 1 turn implementations, delivered through side-channels (callbacks, channels, SSE) — same pattern as Temporal activities streaming to UI while the workflow receives complete results.

**2. Cancellation Token → NOT Layer 0**

Researched: Rust async cancellation (drop-based, CancellationToken, stop-token), Tower, gRPC, Temporal, Restate, MCP, A2A, OpenAI Agents SDK, Google ADK.

Conclusion: No major framework puts cancellation in the operation signature. Tower's Service has no cancel method. gRPC uses context metadata. Temporal uses a server-side cancel API. MCP uses a separate `notifications/cancelled` notification.

Layer 0 already has the right mechanism: `Orchestrator::signal`. Its doc comment even says "Used for: inter-agent messaging, user feedback injection, budget adjustments, **cancellation**." Drop-based cancellation handles the in-process case. `#[non_exhaustive]` lets us add `ExitReason::Cancelled` or `TurnError::Cancelled` later if needed.

**3. Version Negotiation → NOT Layer 0**

Researched: Tower (`tower-service`), `http` crate, `tonic`, MCP, A2A, gRPC, OpenAPI, Cargo semver, serde format evolution.

Conclusion: No Rust trait crate uses runtime version negotiation. Tower, http, tonic all rely entirely on Cargo semver. Version negotiation exists when two independently-deployed binaries must agree at runtime — but Layer 0 is compiled into a single binary via Cargo. The type system provides compile-time guarantees strictly stronger than runtime negotiation. Serde format evolution is already handled by `#[non_exhaustive]`, `Option<T>`, `#[serde(default)]`, and no `deny_unknown_fields`.

**4. `Environment::run` Signature → YES, Layer 0 fix needed**

Researched: Tower Service, Temporal activities, Restate handlers, Kubernetes Jobs, E2B, Modal, Fly Machines.

Conclusion: The `Arc<dyn Turn>` parameter was a mechanism leak violating "operation-defined, not mechanism-defined." Every comparable system sends **data** across boundaries, not function references. Tower's Service takes only a Request. Temporal sends (activity_type + serialized_input). Layer 0's own Orchestrator takes `AgentId` + `TurnInput` — no `Arc<dyn Turn>`.

Remote environments (Docker, K8s) can't send a trait object across a container boundary. They would serialize `TurnInput`, run a turn process inside the container, and deserialize `TurnOutput` — ignoring the `Arc<dyn Turn>` parameter entirely.

**5. Scope Placement → NOT worth changing**

`Scope` in `effect.rs` imported by `state.rs` is a minor organizational quirk, not an architectural issue.

#### Implementation (Phases F-G)

**Phase F** (`ecb2c99`): Removed `Arc<dyn Turn>` from `Environment::run`. The trait now takes only `(TurnInput, &EnvironmentSpec)`. `LocalEnvironment` changed from unit struct to struct with `Arc<dyn Turn>` field, constructed via `LocalEnvironment::new(turn)`. This makes Environment symmetric with Orchestrator — both take only serializable data.

**Phase G** (`81b9e63`): Test coverage hardening — 15 new tests:
- Arc<dyn T> Send+Sync for all 6 traits (was only Turn)
- Error Display for all 24 variants (was 5)
- HookAction serde roundtrip for all 4 variants
- BudgetEvent/BudgetDecision all variants
- ExitReason all 8 variants
- Orchestrator::signal() and ::query()

### Session 5: Ecosystem Layer Mapping Validation

Comprehensive cross-ecosystem research to validate every Layer 0 boundary decision. Four parallel research agents mapped the layered architecture of 15+ systems against our design.

**Systems researched:** Tower/Hyper/Axum/Tonic, Temporal, Restate, gRPC/Connect-RPC, MCP, A2A, E2B, Modal, Fly Machines, Kubernetes Jobs, OpenAI Agents SDK, Google ADK, Anthropic API, LangChain/LangGraph, CrewAI, AutoGen.

**Key finding:** Every system converges on the same layered pattern — a minimal set of operations at Layer 0 with everything else pushed higher. All 6 of our Layer 0 concepts are validated. No changes needed. Details in the "Ecosystem Layer Mapping Validation" section below.

### Session 6: Neuron Workspace Redesign Planning

Detailed the full Layer 0-5 architecture, validated it works at both extremes (single laptop to global distributed), explored the existing `neuron` crate workspace (https://github.com/secbear/neuron, crates.io: `neuron` v0.3.0), and planned the redesign.

**Key decisions:**
- `neuron` becomes the encompassing Cargo workspace holding all 6 layers (0-5)
- `layer0` becomes a crate within the neuron workspace (the foundation)
- The existing neuron crates (neuron-loop, neuron-provider-*, neuron-tool, neuron-context, neuron-mcp, neuron-runtime, neuron-otel) are redesigned to implement layer0 traits
- `neuron-types` is replaced by `layer0` (our protocol traits are the new foundation)
(legacy v0.3.0-era crate names; removed or renamed in the redesign)
- The `redesign/v2` branch on the neuron repo will be the target for this work
- Removed duplicate files: `agentic-decision-map-v3 2.md`, `validation-and-coordination 2.md`

**The architectural reconciliation:** Layer 0 traits are object-safe (via async-trait) for composition (`dyn Turn`, `dyn Orchestrator`). Internal implementation traits in Layer 1+ (Provider, ContextStrategy) can use native RPITIT for performance. The NeuronTurn implements `layer0::Turn` while internally using non-object-safe Provider traits. Protocol boundary = object-safe. Implementation internals = whatever's fastest.

**Full redesign plan saved to:** `NEURON-REDESIGN-PLAN.md`

---

## Current State

### Commit History

```
f837369 feat: add #[non_exhaustive] to all public enums for semver safety
a2beabe feat: add CompactionEvent::ProviderManaged for provider-side compaction
4111727 feat: add #[non_exhaustive] to all public structs with constructor methods
d1927f7 feat: change Environment::run to Arc<dyn Turn> for concurrent dispatch
c78ab90 docs: document serde_json coupling, async_trait migration, sub-turn events
ecb2c99 feat: remove Arc<dyn Turn> from Environment::run — environment owns its turn
81b9e63 test: comprehensive coverage for error Display, Arc safety, serde variants, signal/query
a840405 chore: track all project files in git
```

Note: `d1927f7` added `Arc<dyn Turn>` to Environment::run. `ecb2c99` then removed it. This was intentional — the first change fixed a `&dyn Turn` lifetime issue from the original spec, but deeper research revealed the parameter shouldn't exist at all.

### Test Counts

- **phase1.rs**: 84 tests (serde roundtrips, object safety, error Display, wire format)
- **phase2.rs**: 27 tests (integration tests with concrete implementations)
- **doc-tests**: 1 (DurationMs example)
- **Total**: 112 tests, zero clippy warnings, clean docs

### File Inventory

**Protocol modules** (`src/`):
| File | What it defines |
|------|----------------|
| `lib.rs` | Crate root, `#![deny(missing_docs)]`, module declarations, re-exports |
| `turn.rs` | Turn trait, TurnInput, TurnOutput, TurnConfig, TurnMetadata, ToolCallRecord, TriggerType, ExitReason |
| `orchestrator.rs` | Orchestrator trait, QueryPayload |
| `state.rs` | StateStore trait, StateReader trait, SearchResult, blanket impl |
| `environment.rs` | Environment trait, EnvironmentSpec, IsolationBoundary, CredentialRef, ResourceLimits, NetworkPolicy, NetworkRule |
| `hook.rs` | Hook trait, HookPoint, HookAction, HookContext |
| `lifecycle.rs` | BudgetEvent, BudgetDecision, CompactionEvent, ObservableEvent, EventSource |
| `effect.rs` | Effect enum, Scope, SignalPayload, LogLevel |
| `content.rs` | Content, ContentBlock, ImageSource |
| `error.rs` | TurnError, OrchError, StateError, EnvError, HookError |
| `id.rs` | AgentId, SessionId, WorkflowId, ScopeId (newtype wrappers via macro) |
| `duration.rs` | DurationMs newtype for wire-format stable durations |

**Test utilities** (`src/test_utils/`, behind `test-utils` feature):
| File | What it implements |
|------|-------------------|
| `echo_turn.rs` | EchoTurn — returns input message as output |
| `local_orchestrator.rs` | LocalOrchestrator — HashMap dispatch, tokio::spawn for dispatch_many |
| `in_memory_store.rs` | InMemoryStore — HashMap-backed, scope-isolated |
| `local_environment.rs` | LocalEnvironment — owns Arc<dyn Turn>, passthrough execution |
| `logging_hook.rs` | LoggingHook — records all hook events for assertions |

### Dependencies

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
thiserror = "2"
rust_decimal = { version = "1", features = ["serde-str"] }
tokio = { version = "1", features = ["rt"], optional = true }  # only for test-utils

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

No runtime async runtime dependency. `tokio` is optional, only used by `test-utils` for `tokio::spawn` in `LocalOrchestrator::dispatch_many`.

### Build Commands

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo build
cargo test --features test-utils
cargo clippy --features test-utils -- -D warnings
cargo doc --no-deps
```

All four must pass before any commit.

---

## Key Design Decisions (with rationale)

### D1: `serde_json::Value` for extension points

**Decision:** Use `serde_json::Value` for metadata, tool inputs, custom payloads — not generics.

**Why:** Protocol traits must be object-safe (`Box<dyn Turn>`, `Box<dyn Orchestrator>`). Generics on traits break object safety. `serde_json::Value` is the de facto standard — MCP SDKs, langchain-rust, and AutoAgents all make the same choice. JSON is the universal interchange format for agentic systems.

### D2: `async-trait` (not native async fn in traits)

**Decision:** Use `#[async_trait]` on all protocol traits.

**Why:** Native `async fn in trait` (stable since Rust 1.75) does NOT support `dyn Trait`. Layer 0 requires `Box<dyn Turn>`, `Box<dyn Orchestrator>`, etc. `async-trait` generates `Pin<Box<dyn Future + Send>>` which IS dyn-safe. Migration to native async will happen when Rust stabilizes dyn-safe async fn in traits (no timeline as of Feb 2026). 78% of teams still use `async-trait`.

### D3: `#[non_exhaustive]` everywhere

**Decision:** `#[non_exhaustive]` on all 22 public enums and all 15 public structs.

**Why:** Semver safety. External crates can't exhaustively match enums or use struct literal syntax. This means Layer 0 can add new enum variants and struct fields in minor versions without breaking downstream. Every struct has a constructor (`new()`) or `Default` impl so external crates can still construct them.

### D4: `DurationMs` newtype

**Decision:** Custom `DurationMs(u64)` instead of `std::time::Duration`.

**Why:** `std::time::Duration` serializes as `{"secs":N,"nanos":N}` — an implementation detail of Rust's serde impl, fragile for cross-language interop. `DurationMs` serializes as a plain integer (milliseconds), matching Temporal's wire format. Human-readable. Stable across Rust versions.

### D5: `rust_decimal` for costs

**Decision:** `rust_decimal::Decimal` with `serde-str` feature for all cost fields.

**Why:** LLM pricing involves sub-cent values ($0.000003/token) accumulated over thousands of calls. `f64` accumulation errors become material at scale. `rust_decimal` provides 128-bit fixed-precision arithmetic. `serde-str` serializes as `"0.005"` (string) not `0.005` (float), preventing deserialization precision loss.

### D6: Declared effects pattern

**Decision:** `TurnOutput.effects: Vec<Effect>` — turns declare side-effects, calling layer executes them.

**Why:** Proven pattern from Elm Architecture / Crux (Rust). The turn is pure from the outside — it returns data describing what it wants to happen. The orchestrator decides when/how to execute. This enables: testing (capture effects for assertions), durability (serialize effects into Temporal workflow history), policy (reject forbidden effects), and batching.

### D7: `Environment::run` takes only data

**Decision:** `async fn run(&self, input: TurnInput, spec: &EnvironmentSpec) -> Result<TurnOutput, EnvError>` — no `Arc<dyn Turn>` parameter.

**Why:** The Environment owns or resolves its Turn internally — same pattern as Orchestrator (which takes `AgentId`, not `Arc<dyn Turn>`). This is operation-defined: "execute a turn in this environment." How the environment invokes the turn is internal. Passing `Arc<dyn Turn>` was a mechanism leak — remote environments (Docker, K8s) can't send trait objects across container boundaries. Research across Tower, Temporal, Restate, Kubernetes, E2B, and Modal confirmed: every system sends data across boundaries, not function references.

### D8: Blanket `StateReader` from `StateStore`

**Decision:** `impl<T: StateStore> StateReader for T` — every StateStore is automatically a StateReader.

**Why:** Enforces read/write asymmetry at the type level. Turns receive `&dyn StateReader` (can read state during context assembly) but cannot write (writes go through Effects in TurnOutput). The blanket impl means any `StateStore` works anywhere a `StateReader` is needed.

### D9: Hook as observation + intervention (not a trait on Turn)

**Decision:** Hooks are a separate `Hook` trait with `points()` + `on_event()`, not methods on `Turn`.

**Why:** Hooks are cross-cutting — they observe Turn execution but are managed by the orchestrator. A hook can Continue, Halt, SkipTool, or ModifyToolInput. The HookContext is read-only — hooks observe and decide, they don't mutate directly.

### D10: Lifecycle events are NOT a trait

**Decision:** `BudgetEvent`, `CompactionEvent`, `ObservableEvent` are plain enums/structs, not a trait.

**Why:** Lifecycle coordination is the orchestrator's job. The orchestrator listens for events, applies policies, and takes action. There's no separate "lifecycle service" — it's a responsibility of the orchestration layer. Events are a shared vocabulary, not a protocol boundary.

---

## What Does NOT Belong in Layer 0

These were explicitly researched and rejected:

| Primitive | Where it belongs | Why not Layer 0 |
|-----------|-----------------|-----------------|
| **Streaming interface** | Layer 1 (turn implementations) | Streaming is HOW results are delivered, not WHAT the operation does. Breaks declared effects (must be atomic). Incompatible with durable execution. Hook interface provides the observation point. |
| **Cancellation token** | Implementation + existing `Orchestrator::signal` | No framework puts cancellation in the operation signature (not Tower, not gRPC, not Temporal). Signal method already supports cancellation. Drop-based cancellation handles in-process case. |
| **Version negotiation** | Cargo semver | Layer 0 is a trait crate, not a wire protocol. Cargo's type system enforces compatibility at compile time. No Rust trait crate uses runtime version negotiation. |
| **Tool registration** | MCP / Layer 1 | Layer 0 defines tool *use* (ContentBlock::ToolUse/ToolResult) and tool *restrictions* (TurnConfig::allowed_tools), but tool discovery/registration is a runtime concern handled by MCP or turn implementations. |
| **Agent lifecycle management** | Layer 2 (orchestration implementations) | Whether agents are long-lived actors or ephemeral functions is an orchestrator decision, not a protocol concern. |

---

## Ecosystem Layer Mapping Validation

Comprehensive research across 15+ systems to validate that each Layer 0 concept is at the right layer. Research conducted in Session 5.

### The Universal Layer Pattern

Every mature system converges on the same structure:

| Layer | What lives here | Examples |
|---|---|---|
| **Layer 0** | Minimal trait/protocol — operations + message types | `tower-service` (1 trait, 0 deps), gRPC `.proto` files, MCP JSON-RPC methods, our 4 traits + 2 interfaces |
| **Layer 1** | Protocol-agnostic middleware + implementations | `tower` (retry, timeout, rate limit), Temporal SDK Core, LangChain Runnables |
| **Layer 2** | Domain-specific middleware + composition | `tower-http`, Temporal interceptors, LangGraph StateGraph |
| **Layer 3** | Frameworks + applications | axum/tonic, OpenAI Agents Runner, CrewAI Crew |

### The Tower Ecosystem (Closest Analog)

Tower's layered architecture is the closest structural analog to our design:

**`tower-service`** (Layer 0): Exports exactly ONE trait (`Service<Request>`) with ZERO dependencies. ~390 lines total. This is the integration point the entire Rust async ecosystem depends on.

**`tower-layer`** (Layer 0): Exports ONE trait (`Layer<S>`) with ZERO dependencies. Defines middleware wrapping. Kept in a separate crate from `tower-service` for independent versioning.

**`tower`** (Layer 1): Re-exports both traits + adds `ServiceExt`, `ServiceBuilder`, and 17 feature-gated middleware modules (retry, timeout, rate limit, load balancing, concurrency limit, etc.).

**`tower-http`** (Layer 2): HTTP-specific middleware (CORS, compression, auth, tracing). Depends on `http` crate types.

**`axum`/`tonic`** (Layer 3): Full frameworks. `axum`'s Router IS a `tower::Service<http::Request>`.

**Mapping to our design:**

| Tower concept | Our equivalent | Notes |
|---|---|---|
| `Service::call(Request) -> Future<Response>` | `Turn::execute(TurnInput) -> Result<TurnOutput>` | Isomorphic — single unit of async work |
| `Layer` trait (middleware wrapping) | `Hook` trait (observation + intervention) | Both are cross-cutting, composable, separate from the core trait |
| `steer` + `balance` + `discover` | `Orchestrator` trait | Tower splits routing across modules; we unify coordination |
| No equivalent | `StateStore`/`StateReader` | Tower leaves state to the application |
| No equivalent | `Environment` | Tower has no execution environment concept |
| `poll_ready` (backpressure) | Not in Layer 0 | Agent systems don't need network-style backpressure |

**Critical insight — the Hyper 1.0 fork:** Hyper 1.0 defined its OWN `Service` trait rather than depend on `tower-service` because `tower-service` hasn't reached 1.0. The ecosystem now has two nearly-identical `Service` traits bridged by adapters. **Lesson for us:** If we don't commit to a 1.0 release, downstream crates that want stability may fork our traits.

### Temporal and Restate (Durable Execution)

**Temporal's layers:**

| Layer | What lives here |
|---|---|
| Layer 0 (wire protocol) | gRPC API protos — 8 core RPCs + 17 command types. The true stability contract. |
| Layer 0 (user-facing) | Workflow/activity function signatures — the developer interface |
| Layer 1 (SDK Core) | Rust state machines (~16), polling, history replay, non-determinism detection |
| Layer 1 (configuration) | Retry policies, task queue routing, timeouts |
| Layer 2 (SDK middleware) | Interceptors (5 categories: client, workflow in/out, activity in/out) |
| Layer 2 (infrastructure) | Visibility/search attributes, namespace management, codec/encryption |

Temporal's compaction story: NO log compaction. Hard limit of 50K events / 50MB per workflow. Developer must explicitly call `Continue-As-New`. Our `CompactionEvent` addresses this coordination gap.

**Restate's layers:**

| Layer | What lives here |
|---|---|
| Layer 0 | Handler function + Context interface (`ctx.run()`, `ctx.get()`/`ctx.set()`, `ctx.sleep()`, `ctx.serviceClient()`) |
| Layer 0 (core primitives) | Virtual Objects (key-addressed, exclusive access), Awakeables (durable external callbacks) |
| Layer 1 | Workflows (built on Virtual Objects), durable promises |
| Infrastructure | Server-side journal management, RocksDB state store, Bifrost replicated log |

Restate's state model validates our `StateStore`/`StateReader` split — Restate has `ContextReadState`/`ContextWriteState` (same read/write asymmetry). Restate eliminates the compaction problem via journal-per-invocation.

**Mapping to our design:**

| Our concept | Temporal | Restate |
|---|---|---|
| `Turn` | Workflow task (activation → commands) | Handler invocation |
| `Orchestrator` | Workflow definition + SDK Core state machines | Handler body + server journal management |
| `StateStore` | Event history (implicit, replayed) | `ctx.get()`/`ctx.set()` (direct K/V) |
| `StateReader` | Queries (`@QueryMethod`) | Shared handlers on Virtual Objects |
| `Environment` | Activities on workers (distributed) | `ctx.run()` (inline, journaled) |
| `Hook` | Interceptors (5 categories) | No equivalent (server handles observability) |
| `CompactionEvent` | `Continue-As-New` (manual) | Not needed (journal-per-invocation) |

### AI Agent Frameworks

Researched: OpenAI Agents SDK, Google ADK, Anthropic API + MCP, LangChain/LangGraph, CrewAI, AutoGen.

**Their Layer 0 primitives (what each framework considers irreducible):**

| Framework | Layer 0 interface | Equivalent to our... |
|---|---|---|
| OpenAI Agents | `Agent` dataclass + `Model.get_response()` + `FunctionTool` | Turn + TurnInput/TurnOutput |
| Google ADK | `BaseAgent._run_async_impl(ctx) -> AsyncGen[Event]` | Turn::execute |
| Anthropic | Messages API: `(messages, tools) -> (content_blocks, stop_reason)` | Turn (most minimal of all) |
| LangChain | `Runnable[Input, Output].invoke()` | Turn (most general) |
| AutoGen Core | Actor + `@message_handler` typed dispatch | Turn + Orchestrator |
| CrewAI | `Agent(role, goal, backstory)` + `Task(description, expected_output)` | Turn + TurnInput |

**Orchestration is ALWAYS Layer 1+ in every framework:**

| Framework | Orchestration mechanism | Layer |
|---|---|---|
| OpenAI | `Runner` agentic loop + handoffs | 1 |
| Google ADK | `Runner` event loop + `sub_agents` tree + workflow agents | 1 |
| Anthropic | Client-side `while stop_reason == "tool_use"` loop | 1 (you build it) |
| LangChain | `StateGraph` with conditional edges | 1 |
| CrewAI | `Crew.process` (sequential/hierarchical) | 1 |
| AutoGen | `Team` implementations (RoundRobin, Selector) | 1-2 |

This confirms our `Orchestrator` trait is correctly placed: the **interface** is Layer 0, but all **implementations** are Layer 1+.

**State management is the biggest divergence point across frameworks:**

| Framework | State approach |
|---|---|
| Google ADK | Richest: keyed `session.state` dict + `state_delta` on Events, atomic commits via Runner |
| AutoGen | `save_state()`/`load_state()` returning `Mapping` — interface only, no prescribed store |
| LangChain | Graph state (annotated reducers) + `BaseCheckpointSaver` persistence |
| OpenAI | `TContext` (DI grab bag) + `Session` (opaque) — no formal protocol |
| CrewAI | Implicit (task output as context) — no state protocol |
| Anthropic | None — the messages array is your state |

Our `StateStore`/`StateReader` traits fill the gap that most frameworks leave ad-hoc.

**Hook boundaries are universal (3 points, found across all frameworks with hooks):**
1. Before/after **agent execution** → our `PreTurn`/`PostTurn`
2. Before/after **model call** → our `PreInference`/`PostInference`
3. Before/after **tool execution** → our `PreToolUse`/`PostToolUse`

Google ADK has exactly these 6 callbacks. OpenAI Agents has `RunHooks` with the same events. LangGraph has `wrap_model_call` and `wrap_tool_call`. Our 5 HookPoints cover all three boundaries.

### Protocol Standards (MCP, A2A, gRPC)

**MCP's layers:**

| Layer | Content |
|---|---|
| Layer 0 (protocol) | JSON-RPC framing + `initialize`/`initialized` lifecycle + 6 capability-gated method families (`tools/*`, `resources/*`, `prompts/*`, `sampling/*`, `roots/*`, `elicitation`) |
| Layer 1 (transport) | stdio, Streamable HTTP — swappable |
| Layer 2 (SDK) | Serialization, transport management, timeout/cancellation |
| Layer 3 (implementations) | Actual tool implementations, resource providers |

MCP's `tools/call` is finer-grained than our `Turn::execute`. An MCP server would be consumed as a **tool inside a Turn**, not as a Turn itself. MCP has no state protocol, no orchestration protocol, no lifecycle events — our design fills gaps MCP doesn't address.

**A2A's layers:**

| Layer | Content |
|---|---|
| Layer 0 | `message/send` + `tasks/get` + Task state machine (working/completed/failed) |
| Layer 1 | Streaming (SSE), polling, push notifications |
| Layer 2 | Agent Card discovery, authentication |

A2A's explicit opacity principle ("the remote agent is a black box") maps directly to our trait-object design. `dyn Turn` is opaque. A2A validates our approach.

**gRPC's layers:**

| Layer | Content |
|---|---|
| Layer 0 | `.proto` file (service + message definitions) |
| Layer 1 | Generated stubs/skeletons |
| Layer 2 | Interceptors |
| Layer 3 | Transport (HTTP/2), connection management |
| Layer 4 | Load balancing, retry, service discovery |

Our Rust traits are structurally isomorphic to `.proto` files: `rpc Execute(TurnInput) returns (TurnOutput)` ≡ `async fn execute(&self, input: TurnInput) -> Result<TurnOutput, TurnError>`. Connect-RPC proves Layer 0 definitions can be completely independent of wire format — same `.proto` serves three protocols. Same principle: our `Turn::execute` doesn't know if it's called locally or via HTTP.

**Execution environments (E2B, Modal, Fly, K8s):**

All send **serializable data** across boundaries, confirming our `Environment::run()` design:

| System | Their "run" interface | Data sent |
|---|---|---|
| E2B | `sandbox.commands.run(command)` | String command |
| Modal | `Function.remote(args)` | Serialized arguments |
| Fly | Create + Start + Wait REST API | JSON config |
| K8s Jobs | Job spec (image + command) | YAML/JSON spec |

All confirm: no system sends function references across execution boundaries.

### Validation Summary

| Our Layer 0 concept | Belongs at Layer 0? | Confidence | Key evidence |
|---|---|---|---|
| `Turn` | **YES** | Very High | Universal across all 15+ systems examined |
| `Orchestrator` | **YES** (interface) | High | Temporal puts interface at L0, impl at L1; same pattern |
| `StateStore`/`StateReader` | **YES** | High | Restate, ADK, AutoGen all have state interfaces at L0-1 |
| `Environment` | **YES** | High | E2B/Modal/Fly/K8s all define execution at L0 |
| `Hook` | **YES** | Very High | Tower makes it a separate L0 crate; 3 universal hook boundaries |
| Lifecycle events | **YES** | Medium-High | Novel — fills a gap no other system addresses explicitly |

**Things explicitly NOT in Layer 0 (reconfirmed):**

| Primitive | Ecosystem evidence |
|---|---|
| Streaming | Temporal activities return complete results. Tower streams at Layer 2 (`tower-http`). Delivery, not operation. |
| Cancellation token | Tower has no cancel. Temporal uses server-side API. gRPC uses context metadata. MCP uses notification. |
| Version negotiation | `tower-service`, `http`, `tonic` all use Cargo semver. gRPC uses proto versioning. No runtime negotiation. |
| Runtime agent discovery | MCP has capability negotiation, A2A has Agent Cards — both needed for cross-network. Ours is compile-time. |
| Persistent sessions | E2B/Modal/Fly offer interactive sandboxes over time. Our `Environment::run()` is one-shot by design — implementations manage session lifecycle internally. |
| Backpressure | Tower's `poll_ready` is L0 for network services. Agent systems don't have the same backpressure needs. |

### Enforcement Strategy Comparison

Three strategies for enforcing protocol contracts, and where we fit:

| Strategy | Systems | Tradeoff |
|---|---|---|
| **Compile-time** (type system) | gRPC (codegen), Connect-RPC, **our design** (Rust traits) | Strongest guarantees, language-locked |
| **Runtime negotiation** | MCP (capability exchange), A2A (Agent Cards) | Cross-language/cross-network, runtime errors |
| **SDK-as-contract** | E2B, Modal (SDK API surface IS the contract) | Easiest to iterate, least portable |

Our choice of compile-time enforcement is correct for a stability contract. gRPC's success at massive scale proves this approach works. The `#[non_exhaustive]` annotations give us evolution room within the compile-time paradigm.

---

## Intentional Changes from HANDOFF.md Spec

| Change | Spec said | Code does | Why |
|--------|-----------|-----------|-----|
| Duration types | `std::time::Duration` | `DurationMs(u64)` | Wire-format stability |
| ImageSource | `#[serde(untagged)]` with `Base64(String)`, `Url(String)` | `#[serde(tag = "type")]` with `Base64 { data }`, `Url { url }` | Explicit deserialization |
| HookPoint count | 7 (incl. PreBackfill, ContextSnapshot) | 5 (removed 2) | Simplified; `#[non_exhaustive]` allows adding back |
| Environment::run | `&dyn Turn` parameter | No turn parameter | Operation-defined, not mechanism-defined |
| CompactionEvent | 3 variants | 4 variants (added ProviderManaged) | Server-side compaction pattern |
| New module | Not in spec | `src/duration.rs` | Wire-format wrapper type |

---

## Test Coverage Summary

| Category | Tested | Total | Coverage |
|----------|--------|-------|----------|
| Serde roundtrip (enum variants) | ~65 | ~75 | 87% |
| Error Display | 24 | 24 | 100% |
| Trait object safety (Box) | 6 | 6 | 100% |
| Trait object safety (Arc) | 6 | 6 | 100% |
| Behavioral/integration | Good | — | ~70% |
| Edge cases (empty strings, zero, Unicode) | Minimal | — | ~15% |

### Known untested areas (low priority)
- Empty string IDs, Unicode in IDs
- Zero/max values for token counts, costs, durations
- Content::Text with empty string
- Content::Blocks with empty vec
- ImageSource::Base64 (only Url tested)
- NetworkRule with port=None
- Concurrent state write conflicts
- SearchResult score edge cases (0.0, NaN)
