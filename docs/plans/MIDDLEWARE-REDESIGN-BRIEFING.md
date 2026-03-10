# Middleware Redesign — Sub-Agent Briefing

> Compact handoff document. Read this FIRST, then your assigned task
> from `2026-03-07-middleware-redesign-impl.md`.

## What We're Building

Replace neuron's Hook/HookPoint system with **per-boundary continuation-based
middleware** and unify context types into a single concrete `Context`.

## The Four Changes

### 1. Three middleware traits in layer0

```rust
// Each boundary gets a typed middleware trait + Next trait.
// Middleware wraps the boundary via continuation passing:
//   code before next = pre-processing
//   code after next  = post-processing
//   not calling next = short-circuit (guardrail halt)

#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    async fn dispatch(&self, operator: &OperatorId, input: OperatorInput,
                      next: &dyn DispatchNext) -> Result<OperatorOutput, OrchError>;
}

#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    async fn write(&self, scope: &Scope, key: &str, value: serde_json::Value,
                   next: &dyn StoreWriteNext) -> Result<(), StateError>;
    async fn read(&self, scope: &Scope, key: &str,
                  next: &dyn StoreReadNext) -> Result<Option<serde_json::Value>, StateError>;
}

#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    async fn run(&self, input: OperatorInput, spec: &EnvironmentSpec,
                 next: &dyn ExecNext) -> Result<OperatorOutput, EnvError>;
}
```

Provider middleware is **NOT in layer0** — it stays in neuron-turn (Layer 1),
uses generic wrapping (`RetryProvider<P: Provider>`), not `Box<dyn>`.

### 2. Middleware stack builder with semantic ordering

```rust
let stack = DispatchStack::builder()
    .observe(telemetry)    // outermost — always runs, always calls next
    .transform(sanitizer)  // mutates input, always calls next
    .guard(budget_check)   // innermost — may short-circuit
    .build(orchestrator);  // wraps the concrete Orchestrator
```

Preserves HookRegistry's three-phase dispatch: Observers -> Transformers -> Guards.

### 3. Concrete Context type replacing three overlapping types

```rust
pub struct Message { pub role: Role, pub content: Content, pub meta: MessageMeta }
pub enum Role { System, User, Assistant, Tool { name: String, call_id: String } }

pub struct Context {
    operator_id: OperatorId,
    messages: Vec<Message>,
    watchers: Vec<Arc<dyn ContextWatcher>>,
}

impl Context {
    pub fn compact_truncate(&mut self, keep_last: usize) -> Vec<Message> { .. }
    pub fn compact_by_policy(&mut self) -> Vec<Message> { .. }
    pub fn compact_with(&mut self, f: impl FnMut(&[Message]) -> Vec<Message>) -> Vec<Message> { .. }
}
```

Replaces: `AgentContext<M>` (layer0), `AnnotatedMessage` (neuron-turn),
`ContextStrategy` trait (neuron-turn).

### 4. ReactOperator-internal hook points move out of layer0

`PreInference`, `PostInference`, `ExitCheck`, `SubDispatchUpdate`,
`PreSteeringInject`, `PostSteeringSkip` become a local `ReactInterceptor`
trait in `neuron-op-react`. These are operator implementation details,
not protocol boundaries.

## What Gets Deleted

- `Hook` trait, `HookPoint` enum (19 variants), `HookContext` (18 optional fields)
- `HookAction` enum, `HookPayload` enum
- `HookRegistry` (neuron-hooks crate)
- `ObservableEvent`, `EventSource` (lifecycle.rs)
- `AgentContext<M>`, `ContextMessage<M>` (layer0)
- `AnnotatedMessage`, `ContextStrategy` trait (neuron-turn)

## What Stays Unchanged

- `Operator` trait, `Orchestrator` trait, `StateStore` trait, `Environment` trait
- `Effect` enum, `OperatorInput`/`OperatorOutput`, `ExitReason`
- `CompactionPolicy`, `BudgetEvent`, `CompactionEvent`
- `ContextWatcher` (local to Context, not a protocol boundary)
- All ID types, `Scope`, `Content`

## Consumer Map — Exact Files Per Concern

### Hook consumers (neuron/ only — extras/ has ZERO hook usage)

| File | What it does with hooks |
|------|------------------------|
| `layer0/src/hook.rs` | Definition: HookPoint, HookAction, HookContext, Hook trait |
| `hooks/neuron-hooks/src/lib.rs` | HookRegistry: 3-phase dispatch (Observer/Transformer/Guardrail) |
| `hooks/neuron-hook-security/src/lib.rs` | RedactionHook (PostSubDispatch), ExfilGuardHook (PreSubDispatch) |
| `op/neuron-op-react/src/lib.rs` | Heaviest consumer: 9 hook points (PreInference:646, PostInference:702, PreSubDispatch:936, PostSubDispatch:1019, ExitCheck:1374, PreSteeringInject:542, PostSteeringSkip:835/888/1110/1171, PreCompaction:1554, PostCompaction:1598) |
| `orch/neuron-orch-local/src/lib.rs` | PreDispatch:83, PostDispatch:100, HookPayload::Dispatch |
| `orch/neuron-orch-kit/src/runner.rs` | PreMemoryWrite:154, HookAction matching:165 |
| `effects/neuron-effects-local/src/lib.rs` | PreMemoryWrite:87-89, dispatch:97 |
| `layer0/src/test_utils/logging_hook.rs` | LoggingHook test util |
| `effects/neuron-effects-local/tests/hooks.rs` | HaltHook, RecordingObserver, ModifyTransformer, LifetimeGuardrail |
| `hooks/neuron-hooks/tests/registry.rs` | NamedHook, HaltingHook, SkipDispatchHook, ModifyInputHook |
| `orch/neuron-orch-local/tests/orch.rs` | CountHook test |
| `orch/neuron-orch-kit/tests/hooks.rs` | HaltHook test |

### Context/AnnotatedMessage consumers

| File | What it uses |
|------|-------------|
| `layer0/src/context.rs` | AgentContext<M> definition (UNUSED — no consumers) |
| `turn/neuron-turn/src/context.rs` | AnnotatedMessage struct, ContextStrategy trait, NoCompaction |
| `turn/neuron-context/src/lib.rs` | SlidingWindow + TieredStrategy implementing ContextStrategy |
| `turn/neuron-context/src/context_assembly.rs` | ContextAssembler producing Vec<AnnotatedMessage> |
| `op/neuron-op-react/src/lib.rs` | context_strategy field:185, current_context as Vec<AnnotatedMessage>:198, AnnotatedMessage::from at 779/817/866/1087/1138/1158/1353/1363, compact:1572 |

### Protocol trait implementations (middleware targets)

| Trait | Implementations |
|-------|----------------|
| Orchestrator | LocalOrch (`orch/neuron-orch-local`), ToolRegistryOrch (`turn/neuron-tool`), TemporalOrch (`orch/neuron-orch-temporal`), test LocalOrch (`layer0/test_utils`) |
| StateStore | FsStore (`state/neuron-state-fs`), MemoryStore (`state/neuron-state-memory`), InMemoryStore (`layer0/test_utils`) |
| Environment | LocalEnv (`env/neuron-env-local`) |

### extras/ consumers (will need recompile but minimal changes)

| File | Concern |
|------|---------|
| `extras/orch/neuron-orch-kit/src/middleware.rs` | MiddlewareOrchestrator — already does middleware pattern! |
| `extras/orch/neuron-orch-kit/src/runner.rs` | LocalEffectInterpreter — fires PreMemoryWrite hook |
| `extras/orch/neuron-orch-kit/src/routing.rs` | RoutingOrchestrator — pure delegation |
| `extras/orch/neuron-orch-temporal/src/lib.rs` | TemporalOrch — pure delegation |

## Key Constraints

- **Provider is RPITIT, NOT object-safe** — generic `<P: Provider>` is the boundary
- **`OperatorInput`/`OperatorOutput` are `#[non_exhaustive]`** — use `::new()` constructors
- **No golden decision vocabulary** (D1-D5, C1-C5, L1-L5) in neuron/ code or docs
- **Authority**: ARCHITECTURE.md > specs/ > rules/ > judgment
- **Verification**: `cd neuron/ && nix develop --command cargo test --workspace --all-targets`

## File Layout After Redesign

```
layer0/src/
  context.rs      — Context, Message, Role, MessageMeta, ContextWatcher (rewritten)
  middleware.rs    — DispatchMiddleware/Next, StoreMiddleware/Next, ExecMiddleware/Next
  middleware/
    dispatch_stack.rs  — DispatchStack builder
    store_stack.rs     — StoreStack builder
    exec_stack.rs      — ExecStack builder
  hook.rs         — DELETED
  lifecycle.rs    — BudgetEvent, CompactionEvent (ObservableEvent/EventSource DELETED)
  (all other files unchanged)
```

## Execution Order

| Phase | Tasks | Scope | Parallelizable? |
|-------|-------|-------|----------------|
| 1 | 1.1–1.5 | Add new types (additive, nothing deleted) | 1.1+1.2 parallel, 1.3+1.4 parallel |
| R | R.1–R.6 | Reshape reusable code onto new types | R.1+R.2+R.3+R.4 parallel, R.5 then R.6 serial |
| 2 | 2.1–2.5 | Replace old context types with Context | Mostly serial (cascading) |
| 3 | 3.1–3.5 | Migrate hook consumers to middleware | 3.1+3.2+3.3 parallel, 3.4 solo |
| 4 | 4.1–4.3 | Delete old hook system | Serial (verify between) |
| 5 | 5.1–5.3 | Docs + final verification | 5.1+5.2 parallel |

Phase R tasks produce NEW code alongside old code. Old code stays alive
until Phases 2–4 delete it. This avoids compile breakage during reshaping.

## Full task specs

See `2026-03-07-middleware-redesign-impl.md` for exact code-level specifications
per task, including current source, target code, test rewrites, and dependencies.
