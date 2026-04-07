# Architecture

Skelegent is a composable agentic AI runtime. This document explains what we chose
and why. It governs all architectural decisions in this codebase.

This is a living document. When the problem space evolves, update positions here
first, then propagate to code. The architecture leads; the code follows.

## How to Read

**Implementing**: Find the relevant section before writing code. If a position
says the protocol boundary preserves user choice, your code must preserve that
choice — not hardcode an answer.

**Reviewing**: A change that violates a position here is a bug, regardless of
whether it compiles.

**Disagreeing**: Update this document first, with rationale. Get agreement.
Then change the code. Do not let code drift from the architecture.

**Authority**: Architecture > Specs (`specs/`) > Rules (`rules/`) > Agent
judgment. Higher authority wins. A spec may refine but not contradict a position.

**Decision surface map**: Each golden decision lives at one level: Layer 0 noun, turn-local knob, orchestration knob, or backend implementation point. Do not expand Layer 0 speculatively.

---

## Core Values

Ordered by priority. When values conflict, the higher-ranked value prevails.

### 1. Composability Over Convenience

Every architectural concern should be answerable independently. Durability
should not force a model selection. Isolation should not dictate communication
patterns. Memory strategy should not constrain orchestration topology.

Bundling answers to unrelated concerns into a monolith trades short-term
convenience for long-term rigidity. The composable alternative: protocol
boundaries between layers so implementations can be swapped. The web succeeded
because HTTP is a protocol, not a framework. We follow the same principle.

When you find yourself coupling two concerns that belong in separate layers,
stop. Introduce an interface.

### 2. Declaration Separated From Execution

Operators reason and declare intent. Orchestrators and environments execute.
No component should both decide what to do and carry it out. This is the intent
boundary.

An operator that directly writes to a state store has coupled reasoning to
infrastructure. An operator that emits a `WriteMemory` intent has declared
intent while preserving the freedom to execute that write against git,
PostgreSQL, or an in-memory store. If operators execute their own intents,
swapping a backend requires rewriting every operator. If they declare intents,
it's a configuration change.

When you see an operator importing a concrete state store, database client, or
filesystem API — that's a violation.

Two narrow exceptions to the intent boundary are permitted by design.

**Scoped state**: Operators may read and write their own state partition
via `FlushToStore`/`InjectFromStore` ops or by holding an `Arc<dyn StateStore>`
directly. This is internal state — not a cross-boundary side-effect.
Cross-scope writes remain as `Intent::WriteMemory`.

**Composition dispatch**: Operators may directly dispatch to other operators via
an injected `Arc<dyn Dispatcher>` capability. The dispatched operator's I/O
effects are handled by the dispatcher that executes it — the boundary is
preserved transitively.

### 3. Slim Defaults, Opt-In Complexity

The simplest useful configuration must work without understanding the full
system. Sequential tool execution. No steering. No streaming. Local best-effort
effects. One model. No observer.

Advanced behavior is opt-in via small, composable traits. Each capability is
independently adoptable. No boolean soup. Every new capability must work as an
additive layer — if adopting it requires changing code that doesn't use it, the
abstraction leaks.

### 4. Protocol Stability as the Foundation

Layer 0 — the protocol traits and wire types — is the stability contract. It
changes slowly, additively, and with version discipline. Everything above can
change freely.

Adding a method to a Layer 0 trait is a breaking change that affects every
implementor. Adding a field to a message type requires serde compatibility.
These need version planning. Changes above Layer 0 are routine.

### 5. Explicit Over Implicit

Exit conditions are enumerated, not emergent. Execution strategies are declared,
not inferred. Steering boundaries are defined, not discovered. Lifecycle
coordination flows through observable events, not hidden state.

Every exit reason has a name. Every middleware boundary is documented. Every effect
variant is in the enum. If behavior exists, it's in a type that can be
inspected, logged, and tested.

---

## The Turn

The atomic unit of agency: receive, reason, act. Every agent processes turns.

### Context Assembly

An agent's behavior is shaped entirely by its assembled context. What you
include creates capability; what you exclude creates safety.

**Triggers**: Orchestration routes; the turn receives uniformly via
`OperatorInput`. Operators must not special-case trigger types.

**Identity**: Turn-owned. From rich prompt injection to structural constraint —
but always explicitly configured, never implicitly assumed.

**History**: The turn reads from state; it writes only through intents. The
state backend is swappable without turn changes. Conversation persistence is a
policy above Layer 0, not a built-in context-engine op surface.

**Memory**: Three tiers — hot (always loaded, taxes every turn), warm
(on-demand within session), cold (cross-session search). Tier assignment is
per-agent configuration.

**Tools**: Tools are described by `ToolMetadata`, but the current runtime still compiles schemas from `ToolRegistry` and dispatches either through a direct tool path or an explicitly injected `Arc<dyn Dispatcher>`. Do not pretend the dispatcher-only future state has already landed. Expose only the tools the agent actually needs; use lazy catalog or progressive disclosure for the rest.

**Context budget**: Turn-owned. The compaction reserve must never be zero — a
system at 100% capacity before compacting has no room to run compaction.

### Inference

Model selection, durability, and retry are separable concerns, but durability
and retry are entangled with orchestration. This coupling is real and
acknowledged.

**Model selection**: Turn-owned. Single through three-tier routing supported,
not mandated.

**Durability**: Orchestration-owned. The turn cooperates by respecting orchestration-controlled boundaries and explicit lifecycle outcomes. Local orchestration has no durability. Durable orchestration adds a run/control layer above Layer 0 for starting, inspecting, signalling, resuming, and cancelling long-lived runs. Recovery internals — replay, checkpoints, journals, leases, and storage schema — remain backend-specific. The same operator works in both deployments; durability is a deployment and infrastructure choice, not an operator-level change.

**Retry**: Orchestration-owned. Turn classifies errors as retryable or not
(budget exhaustion and safety refusals: never retry). A single retry
authority — SDK and orchestrator retry must not coexist.

### Sub-Operator Dispatch

Where reasoning meets the real world. Three independent boundaries govern sub-operator dispatch: trust, credentials, and result integration.

**Isolation**: Environment-owned. The full spectrum is supported. The turn
does not know its isolation level — moving from none to container is an
environment swap.

**Credentials**: Environment-owned. Boundary injection preferred — credentials
added at the edge, stripped from context. Tests must prove no secret leakage.

**Backfill**: Turn-owned. Dispatch outputs are the majority of what the model
reasons over. Format them with the same care as prompt design. Strip
security-sensitive content before backfill.

### Exit

Layer multiple independent stop conditions. "Model signals done" alone risks
infinite loops. "Max turns" alone cuts off hard tasks.

Exit can be triggered from multiple sources. See `specs/v2/02-invocation-outcomes-and-waits.md`
for the authoritative outcome table. In summary:
safety halt > turn/step limits > cost budget > completion. The `Outcome` type is
explicit — every exit path has a named variant.

---

## Composition

How turns from different agents relate. Not every system needs this, but every
system that does will face these decisions.

All patterns are built from seven primitives: **Chain**, **Fan-out**, **Fan-in**,
**Delegate**, **Handoff**, **Observe**, and **Intervene**. The first five pass
output as input. Observe watches concurrently. Intervene modifies a running
operator's context from outside. If a new pattern can't be expressed as a
combination of these seven, the framework may need a new primitive.
**Dispatch capability**: Operators receive dispatch capability via `Arc<dyn Dispatcher>`
injected at construction time. `Dispatcher` is the single invocation primitive —
one trait, one method (`dispatch`), used everywhere. There is no separate
"orchestrator dispatch" vs "operator dispatch."

**The Orchestrator is a pattern, not a trait**: The code that builds operators,
wires their dependencies (state stores, providers, middleware, dispatchers),
registers them with a Dispatcher implementation, and manages lifecycle — this
is application code. Different applications wire differently. The Dispatcher is
the swappable part.
Different applications wire differently. The Dispatcher is the swappable part.

**Context transfer**: Task-only injection is the default. Context boundaries
should be enforced by infrastructure (separate process), not by prompt
instruction (fragile). Summary injection preferred over full context inheritance
for multi-level delegation.

**Result routing**: Two-path preferred — full output to persistent storage for
audit, summary to parent context for token efficiency. Direct injection is
acceptable for simple cases but does not scale.

**Agent lifecycle**: Ephemeral is the default. Long-lived is opt-in.
Conversation-scoped handoff (child inherits the conversation, parent terminates)
is a distinct pattern from delegation.

**Communication**: Synchronous call/return is the default (`Dispatcher::dispatch`).
Signals for distributed orchestration (`Intent::Signal`). `ExecutionEvent` is the
semantic observation plane. Observation and intervention are buildable above the
kernel as middleware and orchestration adapters; they are not built into `Context`.

**Observation and intervention**: Middleware-first. A runtime can broadcast
semantic events from middleware, and orchestration can inject control decisions
through its own adapters, but the context engine itself does not own stream or
intervention channels anymore.

**Antipattern — no Workflow trait**: There is no Workflow trait. Workflows are
application-layer code — typed functions or LLM-backed orchestrating operators —
not a framework abstraction.
---

## Lifecycle

Concerns that span multiple turns and agents. They cut across all protocols.

**Memory persistence**: Pre-compaction flush is mandatory. Before destroying old
turns, write important state to persistent storage. On termination, capture work
product before the context window is destroyed. This is the single most critical
lifecycle mechanism for long-running agents.

**Compaction**: Coordination lives above Layer 0. The turn/runtime detects pressure and
runs middleware; orchestration may continue-as-new, and state persists results.
Layer 0 carries only message-level hints (`Message`, `CompactionPolicy`).
Compaction strategies are middleware that mutate `Context` (typically via
`ctx.set_messages(...)`). They are not rules, not a separate crate, and not a
kernel-level policy.

**Crash recovery**: Entangled with orchestration by design, but not with a single universal substrate. Local orchestration provides no recovery — acceptable for short tasks. Durable orchestration adds a run/control layer above Layer 0 and may recover through backend-specific replay, checkpoints, journals, leases, or platform-native history. Public contracts should describe run lifecycle and control semantics; backend recovery internals stay below that boundary. The same operator works in both deployments.

**Budget governance**: Single authority, but current enforcement is runtime-local.
The runtime currently enforces local cost, turn, duration, and tool-call limits
via `BudgetGuard` middleware in the pipeline. Budget-triggered stops surface as
`Outcome::Limited`. Broader halt/continue/downgrade policy belongs above Layer 0.
Planners observe remaining budget (read-only).

**Observability**: Cross-cutting. At protocol boundaries, middleware stacks
(`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`) provide interception
and logging. Runtime-level observation is built above the engine rather than
embedded inside `Context`. Overhead must be proportional to what is enabled.

---

## Three-Primitive Operator Composition

Operators compose three independent primitives: **middleware** (observation +
intervention via per-boundary stacks — `DispatchMiddleware`, `StoreMiddleware`,
`ExecMiddleware` — composed into `DispatchStack`, `StoreStack`, `ExecStack`),
**steering** (opt-in via builder, external control flow), and **planner**
(opt-in via builder, execution strategy). These are structurally different
and must not be unified:

- Middleware is per-boundary, composes via stacks, returns transformed
  requests/responses or short-circuits with errors.
- Steering is poll-driven, returns messages, composes by concatenation.
- Planner is declarative, returns batch plans, composes by delegation.

Security middleware (`RedactionMiddleware`, `ExfilGuardMiddleware` from
`skg-hook-security`) provides visibility into steering and dispatch
without conflating architecturally distinct primitives.

Middleware composition varies by boundary: dispatch middleware wraps sub-operator
dispatch, store middleware wraps state access, exec middleware wraps operator
execution. For invocation outcome ordering,
see `specs/v2/02-invocation-outcomes-and-waits.md`.

The planner primitive is `DispatchPlanner` (renamed from `ToolExecutionPlanner`).
It plans sub-operator dispatches — not just tool calls — and is the canonical
extension point for custom execution strategies.

---

## Behavior Lives in Code, Configuration Lives in Messages

A recurring question in agent frameworks: how does a parent inject behavior
into a child agent at dispatch time? Rule injection, middleware descriptors,
policy schemas — every framework eventually faces this.

Skelegent's answer: **don't serialize behavior. Share it through construction.**

This is not novel. It is the pattern behind every composition model that
scaled:

- Express/Koa: `app.use(cors())` at startup. The middleware is a function, not
  a descriptor.
- Tower: `ServiceBuilder::new().layer(TimeoutLayer)`. Layers compose at
  construction. Nothing is serialized.
- Linux: file operations are a vtable set at module load, not an enum of known
  operations.
- React: context providers wrap children at render time, not via schema.

The principle: **the composition point is the constructor, not the message.**

### What this means for skelegent

Rules, middleware, and operators are live objects — closures, trait objects,
function pointers. They compose at construction time via `Context::add_rule()`,
middleware stacks, and operator factories. They do not serialize.

`OperatorConfig` carries **data**: cost limits, turn limits, model overrides,
duration caps. The child operator reads this data and decides what behavior to
construct. The parent controls the parameters. The child owns the behavior.

```rust
// Parent controls the budget (data):
let config = OperatorConfig { max_cost: Some(dec!(5.0)), ..Default::default() };
let mut input = OperatorInput::new(message, TriggerType::Task);
input.config = Some(config);
dispatcher.dispatch(&operator_id, input).await;

// Child constructs the pipeline (behavior):
let mut pipeline = Pipeline::new();
if let Some(max_cost) = input.config.as_ref().and_then(|c| c.max_cost) {
    pipeline.push_before(Box::new(BudgetGuard::with_config(BudgetGuardConfig {
        max_cost: Some(max_cost),
        ..Default::default()
    })));
}
```

### Why not a MiddlewareSpec enum?

A serializable enum of known middleware types (`BudgetGuard`, `Telemetry`,
`AutoCompact`, `Custom(Value)`) is a closed vocabulary disguised as
extensible. Every new middleware requires editing the enum or falling back
to untyped JSON. This is a central registry — it violates composability
(§1) and forces the framework to know about every middleware that will ever exist.

### The dispatch boundary

Same-process dispatch (`LocalDispatcher`): the agent registry — the thing
that maps `OperatorId` to a constructed `Operator` — is where behavior is wired.
Middleware is attached at operator construction, not per-dispatch. If a system
architect wants agent B to always run with a budget guard, they configure that
when registering agent B.

Cross-process dispatch (durable orchestrators): only data crosses the wire.
`OperatorConfig` serializes. The remote process constructs behavior from that
data using its own operator factories. Behavior stays local to the process
that executes it.

Per-dispatch variation (parent wants a tighter budget this time): use
`OperatorConfig` fields. The child reads the data and adjusts its pipeline.
The data travels. The behavior doesn't.

### Antipattern — behavior descriptors

Do not create serializable representations of behavior (rule specs, middleware
descriptors, policy schemas) and ship them across dispatch boundaries. This
pattern creates a shadow type system that duplicates the real one, requires
synchronized registries, and collapses when the descriptor language can't
express what the actual code can. If you need richer per-dispatch
customization than `OperatorConfig` provides, add a field to `OperatorConfig`
(it is `#[non_exhaustive]`), not a descriptor language.

---

## Middleware Blueprint

Six middleware stacks protect six protocol boundaries. Each stack follows the
same structural pattern. The traits are hand-written per boundary because each
boundary has a unique method signature — dispatch takes `(ctx, input) → Handle`,
infer takes `request → Response`, store has separate `read` and `write`, etc.
Rust's type system prevents a single generic middleware trait across these
signatures. This is intentional: **type safety at each boundary IS the value.**

### Where stacks live

| Crate | Stacks | Boundaries |
|---|---|---|
| `layer0` | `DispatchStack`, `StoreStack`, `ExecStack` | Protocol boundaries (dispatch, state, environment) |
| `skg-turn` | `InferStack`, `EmbedStack` | Provider boundaries (inference, embedding) |
| `skg-secret` | `SecretStack` | Secret boundary (secret resolution) |

### The pattern

Every stack is built from the same five components. When adding a new boundary,
replicate this template:

1. **`*Next` trait** — Continuation to the next layer. One async method matching
   the boundary's signature. Implementing types: the chain struct and the
   terminal (the real service).

2. **`*Middleware` trait** — Wraps the call with a `next: &dyn *Next` parameter
   for continuation-passing. Code before `next` = pre-processing. Code after
   `next` = post-processing. Not calling `next` = short-circuit.

3. **`*Stack` struct** — `{ layers: Vec<Arc<dyn *Middleware>> }`. Holds the
   composed chain. Provides `*_with()` to run a request through the chain.

4. **`*StackBuilder`** — `{ observers, transformers, guards }` with `.observe()`,
   `.transform()`, `.guard()`, `.build()`. Enforces ordering at construction.

5. **`*Chain` struct** — Internal (not `pub`). Links middleware to next via
   `index + terminal`. Implements `*Next` so each layer sees the remainder of
   the chain as its continuation.

The `*_with()` method on the stack runs the request through the chain. If the
stack is empty, it calls the terminal directly (zero overhead).

### Ordering contract

Stacking order is fixed at build time:

```
Observers (outermost) → Transformers → Guards (innermost) → Terminal
```

- **Observers** always run, always call `next`. They see every request and
  response, even when a guard short-circuits. Use for: logging, metrics,
  audit trails.
- **Transformers** may modify the request or response, always call `next`.
  Use for: input normalization, encryption, model overrides.
- **Guards** may short-circuit by returning an error without calling `next`.
  Use for: budget enforcement, content filtering, access control.

This ordering means observers see the original request, transformers shape it,
and guards make the final accept/reject decision on the transformed input.