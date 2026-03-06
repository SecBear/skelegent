# Architecture

Neuron is a composable agentic AI runtime. This document explains what we chose
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
No component should both decide what to do and carry it out. This is the effects
boundary.

An operator that directly writes to a state store has coupled reasoning to
infrastructure. An operator that emits a `WriteMemory` effect has declared
intent while preserving the freedom to execute that write against git,
PostgreSQL, or an in-memory store. If operators execute their own effects,
swapping a backend requires rewriting every operator. If they declare effects,
it's a configuration change.

When you see an operator importing a concrete state store, database client, or
filesystem API — that's a violation.

Two narrow exceptions to the effects boundary are permitted by design.

**Scoped state**: Operators may read and write their own state partition directly
via injected `ScopedState`. This is internal state — not a cross-boundary
side-effect. Cross-scope writes remain as `Effect::WriteMemory`.

**Composition dispatch**: Operators may directly dispatch to other operators via
an injected `Arc<dyn Orchestrator>` capability. The dispatched operator's I/O
effects are handled by the orchestrator that executes it — the boundary is
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

Every exit reason has a name. Every hook point is documented. Every effect
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

**History**: The turn reads from state; it writes only through effects. The
state backend is swappable without turn changes. Serialized snapshots
(save/load context) are implemented via `ContextCommand::SaveSnapshot` /
`LoadSnapshot` — a user-triggered portable checkpoint pattern for long sessions.

**Memory**: Three tiers — hot (always loaded, taxes every turn), warm
(on-demand within session), cold (cross-session search). Tier assignment is
per-agent configuration.

**Tools**: Tools are operators registered with `ToolMetadata` — name, description, input schema, and concurrency hints (`parallel_safe`) are carried in the metadata, not a separate tool registry. Sub-operator dispatch (formerly 'tool execution') is mediated by an injected `Arc<dyn Orchestrator>` capability, not the environment protocol directly. Antipattern: naive API-to-MCP conversion — exposing every REST endpoint as an MCP tool without filtering causes context pollution and token waste. Expose only the tools the agent actually needs; use lazy catalog or progressive disclosure for the rest.

**Context budget**: Turn-owned. The compaction reserve must never be zero — a
system at 100% capacity before compacting has no room to run compaction.

### Inference

Model selection, durability, and retry are separable concerns, but durability
and retry are entangled with orchestration. This coupling is real and
acknowledged.

**Model selection**: Turn-owned. Single through three-tier routing supported,
not mandated.

**Durability**: Orchestration-owned. The turn cooperates via heartbeat hooks.
Local orchestration: no durability. Durable orchestration: checkpoint or replay.
Same operator works in both — deployment choice, not code change.

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

Exit can be triggered from multiple sources. See `specs/04-operator-turn-runtime.md
§Exit Priority Ordering` for the authoritative priority table. In summary:
safety halt > turn/step limits > cost budget > completion. The `ExitReason` enum is
explicit — every exit path has a named variant.

---

## Composition

How turns from different agents relate. Not every system needs this, but every
system that does will face these decisions.

All patterns are built from six primitives: **Chain**, **Fan-out**, **Fan-in**,
**Delegate**, **Handoff**, and **Observe**. The first five pass output as input.
Observe watches concurrently and may intervene. If a new pattern can't be
expressed as a combination of these six, the framework may need a new primitive.

**Dispatch capability**: Operators receive dispatch capability via `Arc<dyn Orchestrator>`
injected at construction time.

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

**Communication**: Synchronous call/return is the default. Signals for
distributed orchestration. Shared state and event streams require explicit
ordering and conflict resolution.

**Observation**: Mediated by hooks, attached by orchestration. Three forms:
oracle (pull, advisory), guardrail (checkpoint, can halt), observer agent
(continuous, full intervention). Hook handlers must not block indefinitely.

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

**Compaction**: Three-way coordination between turn (detects pressure, executes
summarization), orchestration (may continue-as-new), and state (persists
results). Summarization is the default. The compaction reserve must never be
zero. Selective/tiered compaction (`TieredStrategy`) is implemented — context
partitioned into zones with different policies (pin, compress, discard).
Recursive summarization degradation is a documented failure mode: summarizing
summaries loses critical detail after 2-3 cycles; fresh summary replacement is
the mitigation. Message-level metadata (`AnnotatedMessage`, `CompactionPolicy`)
enables per-message pin/compress/discard policies.

**Crash recovery**: Entangled with orchestration by design. Local: no recovery,
acceptable for short tasks. Durable: replay recovery. The same operator works
in both. This entanglement is architectural, not incidental — we accept it
rather than fighting it with leaky abstractions.

**Budget governance**: Single authority. The turn emits cost events.
Orchestration tracks aggregate cost. The lifecycle coordinator makes
halt/continue/downgrade decisions. Planners observe remaining budget (read-only).

**Observability**: Cross-cutting. All layers emit through a common event
interface with source, type, timestamp, and trace ID. Overhead must be
proportional — structured tracing for production, full event logging for
debugging.

---

## Three-Primitive Operator Composition

Operators compose three independent primitives: **hooks** (observation +
intervention, always wired — constructor parameter, may be an empty registry),
**steering** (opt-in via builder, external control flow), and **planner**
(opt-in via builder, execution strategy). These are structurally different
and must not be unified:

- Hooks are event-driven, return actions, compose by kind
  (guardrail/transformer/observer).
- Steering is poll-driven, returns messages, composes by concatenation.
- Planner is declarative, returns batch plans, composes by delegation.

Hooks observe steering (via PreSteeringInject/PostSteeringSkip) without
replacing it. This provides security visibility into steering without
conflating two architecturally distinct primitives.

Hook composition varies by `HookKind`: guardrails short-circuit on Halt;
transformers chain modifications; observers run unconditionally. Dispatch
order: observers, then transformers, then guardrails. For exit priority ordering,
see `specs/04-operator-turn-runtime.md §Exit Priority Ordering`.

The planner primitive is `DispatchPlanner` (renamed from `ToolExecutionPlanner`).
It plans sub-operator dispatches — not just tool calls — and is the canonical
extension point for custom execution strategies.