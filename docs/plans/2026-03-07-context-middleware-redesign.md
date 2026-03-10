# Context & Middleware Redesign

**Status:** Approved design. Pending implementation plan.

**Problem:** The current hook system is architecturally flawed.
`HookPoint` is a 19-variant flat enum that leaks operator internals
(`PreInference`, `PostInference`, `ExitCheck`, `SubDispatchUpdate`,
`PreSteeringInject`, `PostSteeringSkip`) into layer0. `AgentContext<M>`
duplicates `AnnotatedMessage` + `ContextStrategy` across two crates.
The Hook trait is event-based — the weakest intervention pattern. Every
mature framework (tower, axum, gRPC, ASP.NET, Django, Vercel AI SDK)
converged on continuation-based middleware instead.

**Core invariant preserved:** Swappability of state backends, execution
environments (local/docker/k8s/wasm), providers (anthropic/openai/ollama),
and orchestration (none/temporal/restate).

---

## Design

### 1. Three Protocol-Boundary Middleware Traits (Layer 0)

Layer0 has three protocol boundaries: Orchestrator, StateStore,
Environment. Each gets a typed middleware trait using the continuation
pattern:

```rust
/// Middleware wrapping Orchestrator::dispatch.
///
/// Budget enforcement, tracing, guardrails, audit logging — anything
/// that needs to observe or intervene in agent dispatch.
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError>;
}

/// Middleware wrapping StateStore read/write.
///
/// Encryption-at-rest, audit trails, caching, access control.
#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError>;

    async fn read(
        &self,
        scope: &Scope,
        key: &str,
        next: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError>;
}

/// Middleware wrapping Environment::run.
///
/// Credential injection, isolation upgrade, resource limit enforcement.
#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError>;
}
```

The continuation pattern provides all three intervention modes
naturally:

- **Observer**: call `next`, log both input and output.
- **Transformer**: mutate input before calling `next`, or mutate
  output after.
- **Guardrail**: conditionally don't call `next` — return an error
  or synthetic response.

### 2. Provider Middleware (Layer 1 — NOT in layer0)

Provider is RPITIT, not object-safe, and lives in the turn layer
(`skg-turn`). Provider middleware uses generic wrapping, not
`Box<dyn>`:

```rust
// In skg-turn, NOT layer0
struct RetryProvider<P: Provider> {
    inner: P,
    max_retries: u32,
}

impl<P: Provider> Provider for RetryProvider<P> {
    fn complete(&self, req: ProviderRequest)
        -> impl Future<Output = Result<ProviderResponse, ProviderError>> + Send
    {
        // retry logic wrapping self.inner.complete(req)
    }
}
```

This keeps layer0 clean — it has no knowledge of Provider. The three
layer0 middleware traits (Dispatch, Store, Exec) compose via
`Box<dyn>`. Provider middleware composes via generic wrapping. This
distinction is architecturally correct and must remain explicit.

### 3. Middleware Stack Builder

The current HookRegistry dispatches in three phases: Observers,
then Transformers, then Guardrails. The middleware builder preserves
these ordering semantics:

```rust
let dispatch_stack = DispatchStack::builder()
    .observe(telemetry_middleware)     // always runs, always calls next
    .observe(audit_logger)
    .transform(input_sanitizer)       // mutates input, always calls next
    .guard(budget_check)              // may short-circuit
    .guard(content_policy)
    .build(orchestrator);             // wraps the concrete Orchestrator
```

The builder enforces the stacking order:
1. **Observers** (outermost) — always call `next`, see everything.
2. **Transformers** — mutate input/output, always call `next`.
3. **Guardrails** (innermost, closest to the real call) — may
   short-circuit by not calling `next`.

This means observers always run (even if a guardrail halts), and
guardrails see the already-transformed input. Same semantics as
the current HookRegistry, expressed as middleware stacking.

### 4. Unified Context Type (Layer 0)

Replace `AgentContext<M>` (generic), `AnnotatedMessage` (turn layer),
and `ContextStrategy` (trait) with a single concrete type:

```rust
/// A message in an agent's context window.
pub struct Message {
    pub role: Role,
    pub content: Content,
    pub meta: MessageMeta,
}

pub enum Role {
    System,
    User,
    Assistant,
    Tool { name: String, call_id: String },
}

/// The agent's view of the world. Owns the message window.
///
/// Compaction is methods on Context, not a trait hierarchy.
pub struct Context {
    operator_id: OperatorId,
    messages: Vec<Message>,
    watchers: Vec<Arc<dyn ContextWatcher>>,
}

impl Context {
    // --- Mutation (watcher-guarded) ---
    pub fn push(&mut self, msg: Message) -> Result<(), ContextError> { .. }
    pub fn remove_where(&mut self, pred: impl Fn(&Message) -> bool) -> Result<Vec<Message>, ContextError> { .. }
    pub fn transform(&mut self, f: impl FnMut(&mut Message)) { .. }

    // --- Compaction (built-in strategies) ---
    pub fn compact_truncate(&mut self, keep_last: usize) -> Vec<Message> { .. }
    pub fn compact_by_policy(&mut self) -> Vec<Message> { .. }
    pub fn compact_with(&mut self, f: impl FnMut(&[Message]) -> Vec<Message>) -> Vec<Message> { .. }

    // --- Introspection ---
    pub fn snapshot(&self) -> ContextSnapshot { .. }
    pub fn messages(&self) -> &[Message] { .. }
    pub fn estimated_tokens(&self) -> usize { .. }
}
```

Key changes from current:
- **No generic parameter.** `Message` is concrete (role + content +
  meta). The old `ContextMessage<M>` required callers to pick a
  message type. Now there's one message type in layer0.
- **System prompt is a message.** `Role::System` instead of a
  separate `Option<String>` field. System messages can have
  metadata (compaction policy, salience) like any other message.
- **Compaction is methods, not traits.** `compact_truncate`,
  `compact_by_policy`, `compact_with` replace the `ContextStrategy`
  trait hierarchy. Custom strategies use `compact_with(closure)`.
- **ContextWatcher stays** for mutation gating (injection approval,
  compaction approval). It's local to Context, not a protocol
  boundary concern.

### 5. What Gets Deleted

| Current | Replacement |
|---------|-------------|
| `Hook` trait | Three middleware traits (Dispatch, Store, Exec) |
| `HookPoint` enum (19 variants) | Deleted. Each middleware type IS its boundary. |
| `HookContext` (18 optional fields) | Middleware receives typed args per boundary. |
| `HookAction` enum | Continuation pattern (call/don't call `next`). |
| `HookPayload` enum | Deleted. Middleware sees native types directly. |
| `AgentContext<M>` (generic) | `Context` (concrete) |
| `AnnotatedMessage` (turn layer) | `Message` (layer0) |
| `ContextStrategy` trait (turn layer) | `Context::compact_*` methods |
| `lifecycle::ObservableEvent` | `tracing` spans + middleware |
| `lifecycle::EventSource` | Deleted. Tracing carries source via span hierarchy. |

### 6. What Stays Unchanged

| Component | Why |
|-----------|-----|
| `Operator` trait | Correct abstraction. The composition unit. |
| `Orchestrator` trait | Protocol boundary. Middleware wraps it. |
| `StateStore` trait | Protocol boundary. Middleware wraps it. |
| `Environment` trait | Protocol boundary. Middleware wraps it. |
| `Effect` enum | Declaration separated from execution — core value. |
| `OperatorInput` / `OperatorOutput` | Wire types. Stable. |
| `ExitReason` enum | Explicit exit conditions — core value. |
| `Scope` / ID types | Stable addressing. |
| `CompactionPolicy` | Per-message policy. Moves to `MessageMeta`. |
| `BudgetEvent` / `CompactionEvent` | Cross-layer coordination vocabulary. |
| `ContextWatcher` | Local to Context, not a protocol boundary. |

### 7. Observation vs Intervention

Clean separation:

- **Observation** = `tracing`. `#[instrument]` on every async trait
  boundary. Zero-cost when no subscriber. Already implemented on all
  protocol boundaries in the current codebase.
- **Intervention** = middleware. Budget enforcement, content policy,
  input sanitization, audit logging. Only added when needed.

The current Hook trait conflates these. A `TelemetryHook` that just
logs should not share an API with a `BudgetHook` that halts execution.
Tracing handles the first. Middleware handles the second.

### 8. Operator-Internal Concerns

The following move OUT of layer0 into operator implementations
(e.g., `skg-op-react`):

- `PreInference` / `PostInference` — ReactOperator's inference loop
- `ExitCheck` — ReactOperator's exit evaluation
- `SubDispatchUpdate` — ReactOperator's streaming updates
- `PreSteeringInject` / `PostSteeringSkip` — steering is opt-in
- Steering primitive — operator-local, not a protocol boundary
- Planner primitive — operator-local, not a protocol boundary

These become ReactOperator-internal extension points. Other operator
implementations (static tools, human-in-loop) never had these
concepts and shouldn't be forced to know about them.

---

## Rejected Alternatives

### Generic Service + Layer (Tower-style)

Rejected. `Service<Req>` erases the semantic differences between
boundaries. An Orchestrator dispatch and a StateStore write are
fundamentally different operations — different failure modes,
different retry semantics, different observability needs. Tower
works for HTTP because every HTTP handler really is the same shape.
Skelegent's protocol boundaries are meaningfully distinct. Making them
all `Service<SomeRequest>` is technically true but architecturally
misleading. Additionally, Rust's async + generics + dyn dispatch
create type signature complexity that undermines ergonomics.

### Boundary x Phase Event Matrix

Rejected. Keeps the event-based pattern (weakest). Cannot wrap
behavior — no `next.run()` means no natural request/response
modification. The insight that there are a small number of real
boundaries was correct; the mistake was keeping it event-based
rather than going all the way to continuation passing.

---

## Migration Notes

- `AgentContext<M>` → `Context`: mechanical. Tests already cover
  the API surface; change the type, fix the compile errors.
- `Hook` impls → middleware: each Hook becomes a middleware on the
  boundary it was observing. HookPoints map to boundaries:
  `PreDispatch`/`PostDispatch` → `DispatchMiddleware`,
  `PreStateWrite`/`PostStateRead` → `StoreMiddleware`,
  `PreProviderCall`/`PostProviderCall` → `ProviderMiddleware` (L1).
  ReactOperator-specific points move to `skg-op-react`.
- `HookRegistry` → `DispatchStack::builder()` (+ Store, Exec).
  Observer/Transformer/Guardrail ordering preserved by builder API.
