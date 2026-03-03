# Agent Constitution

This document governs architectural decisions in this codebase. Any agent — human or artificial — working here must conform to the values, principles, and positions it defines.

This is a living document. When the problem space evolves, update positions here first, then propagate to code. The constitution leads; the code follows.

It derives from the [Agentic Decision Map](docs/architecture/agentic-decision-map-v3.md), which enumerates all 23 architectural decisions any agentic system must answer. Where the decision map is analytical (here is the design space), this document is normative (here is what we believe, and why).

## How to Read

**Implementing**: Find the relevant decision before writing code. If a position says the protocol boundary preserves user choice, your code must preserve that choice — not hardcode an answer.

**Reviewing**: A change that violates a position here is a bug, regardless of whether it compiles.

**Disagreeing**: Update this document first, with rationale. Get agreement. Then change the code. Do not let code drift from the constitution.

**Authority**: Constitution > Specs (`specs/`) > Rules (`rules/`) > Agent judgment. Higher authority wins. A spec may refine but not contradict a position. Agent judgment fills gaps but must not invent behavior that contradicts any of the above.

---

## Core Values

Ordered by priority. When values conflict, the higher-ranked value prevails.

### 1. Composability Over Convenience

Every decision should be answerable independently. Durability should not force a model selection. Isolation should not dictate communication patterns. Memory strategy should not constrain orchestration topology.

Bundling answers to unrelated decisions into a monolith trades short-term convenience for long-term rigidity. The composable alternative: protocol boundaries between layers so implementations can be swapped. The web succeeded because HTTP is a protocol, not a framework. We follow the same principle.

When you find yourself coupling two concerns that the 23 decisions list separately, stop. Introduce an interface.

### 2. Declaration Separated From Execution

Operators reason and declare intent. Orchestrators and environments execute. No component should both decide what to do and carry it out. This is the effects boundary.

An operator that directly writes to a state store has coupled reasoning to infrastructure. An operator that emits a `WriteMemory` effect has declared intent while preserving the freedom to execute that write against git, PostgreSQL, or an in-memory store. If operators execute their own effects, swapping a backend requires rewriting every operator. If they declare effects, it's a configuration change.

When you see an operator importing a concrete state store, database client, or filesystem API — that's a violation.

### 3. Slim Defaults, Opt-In Complexity

The simplest useful configuration must work without understanding the full system. Sequential tool execution. No steering. No streaming. Local best-effort effects. One model. No observer.

Advanced behavior is opt-in via small, composable traits. Each capability is independently adoptable. No boolean soup. Every new capability must work as an additive layer — if adopting it requires changing code that doesn't use it, the abstraction leaks.

### 4. Protocol Stability as the Foundation

Layer 0 — the protocol traits and wire types — is the stability contract. It changes slowly, additively, and with version discipline. Everything above can change freely.

Adding a method to a Layer 0 trait is a breaking change that affects every implementor. Adding a field to a message type requires serde compatibility. These need version planning. Changes above Layer 0 are routine.

### 5. Explicit Over Implicit

Exit conditions are enumerated, not emergent. Execution strategies are declared, not inferred. Steering boundaries are defined, not discovered. Lifecycle coordination flows through observable events, not hidden state.

Every exit reason has a name. Every hook point is documented. Every effect variant is in the enum. If behavior exists, it's in a type that can be inspected, logged, and tested.

---

## The Turn

The atomic unit of agency: receive, reason, act. Every agent processes turns. These decisions are universal.

### Context Assembly (D1, D2A–D2E)

An agent's behavior is shaped entirely by its assembled context. What you include creates capability; what you exclude creates safety.

**D1 — Triggers**: Orchestration routes; the turn receives uniformly via `OperatorInput`. Operators must not special-case trigger types.

**D2A — Identity**: Turn-owned. From rich prompt injection to structural constraint — but always explicitly configured, never implicitly assumed.

**D2B — History**: The turn reads from state; it writes only through effects. The state backend is swappable without turn changes.

**D2C — Memory**: Three tiers — hot (always loaded, taxes every turn), warm (on-demand within session), cold (cross-session search). Tier assignment is per-agent configuration.

**D2D — Tools**: Definitions are the source of truth for execution metadata, including concurrency hints. Tool execution is mediated by the environment protocol, not the turn.

**D2E — Context budget**: Turn-owned. The compaction reserve must never be zero — a system at 100% capacity before compacting has no room to run compaction.

### Inference (D3A–D3C)

Model selection, durability, and retry are separable decisions, but durability and retry are entangled with orchestration. This coupling is real and acknowledged.

**D3A — Model selection**: Turn-owned. Single through three-tier routing supported, not mandated.

**D3B — Durability**: Orchestration-owned. The turn cooperates via heartbeat hooks. Local orchestration: no durability. Durable orchestration: checkpoint or replay. Same operator works in both — deployment choice, not code change.

**D3C — Retry**: Orchestration-owned. Turn classifies errors as retryable or not (budget exhaustion and safety refusals: never retry). A single retry authority — SDK and orchestrator retry must not coexist.

### Tool Execution (D4A–D4C)

Where reasoning meets the real world. Three independent boundaries: trust, credentials, and result integration.

**D4A — Isolation**: Environment-owned. The full spectrum is supported. The turn does not know its isolation level — moving from none to container is an environment swap.

**D4B — Credentials**: Environment-owned. Boundary injection preferred — credentials added at the edge, stripped from context. Tests must prove no secret leakage.

**D4C — Backfill**: Turn-owned. Tool outputs are the majority of what the model reasons over. Format them with the same care as prompt design. Strip security-sensitive content before backfill.

### Exit (D5)

Layer multiple independent stop conditions. "Model signals done" alone risks infinite loops. "Max turns" alone cuts off hard tasks.

Exit can be triggered from multiple sources. Priority resolves conflicts: **safety halt > budget > max turns > model done**. The `ExitReason` enum is explicit — every exit path has a named variant.

---

## Composition

How turns from different agents relate. Not every system needs this, but every system that does will face these decisions.

All patterns are built from six primitives: **Chain**, **Fan-out**, **Fan-in**, **Delegate**, **Handoff**, and **Observe**. The first five pass output as input. Observe watches concurrently and may intervene. If a new pattern can't be expressed as a combination of these six, the framework may need a new primitive — update this constitution.

**C1 — Context transfer**: Task-only injection is the default. Context boundaries should be enforced by infrastructure (separate process), not by prompt instruction (fragile). Summary injection preferred over full context inheritance for multi-level delegation.

**C2 — Result routing**: Two-path preferred — full output to persistent storage for audit, summary to parent context for token efficiency. Direct injection is acceptable for simple cases but does not scale.

**C3 — Agent lifecycle**: Ephemeral is the default. Long-lived is opt-in. Conversation-scoped handoff (child inherits the conversation, parent terminates) is a distinct pattern from delegation.

**C4 — Communication**: Synchronous call/return is the default. Signals for distributed orchestration. Shared state and event streams require explicit ordering and conflict resolution.

**C5 — Observation**: Mediated by hooks, attached by orchestration. Three forms: oracle (pull, advisory), guardrail (checkpoint, can halt), observer agent (continuous, full intervention). Hook handlers must not block indefinitely.

---

## Lifecycle

Concerns that span multiple turns and agents. They cut across all protocols.

**L1 — Memory persistence**: Pre-compaction flush is mandatory. Before destroying old turns, write important state to persistent storage. On termination, capture work product before the context window is destroyed. This is the single most critical lifecycle mechanism for long-running agents.

**L2 — Compaction**: Three-way coordination between turn (detects pressure, executes summarization), orchestration (may continue-as-new), and state (persists results). Summarization is the default. The compaction reserve must never be zero.

**L3 — Crash recovery**: Entangled with orchestration by design. Local: no recovery, acceptable for short tasks. Durable: replay recovery. The same operator works in both. This entanglement is architectural, not incidental — we accept it rather than fighting it with leaky abstractions.

**L4 — Budget governance**: Single authority. The turn emits cost events. Orchestration tracks aggregate cost. The lifecycle coordinator makes halt/continue/downgrade decisions. Planners observe remaining budget (read-only).

**L5 — Observability**: Cross-cutting. All layers emit through a common event interface with source, type, timestamp, and trace ID. Overhead must be proportional — structured tracing for production, full event logging for debugging.

---

## Adapting This Document

This constitution is designed to be forked. The structure is universal; the positions are project-specific.

**Keep when forking**: The five core values and their ordering. The three-layer organization. The 23 decision points. The authority hierarchy.

**Replace when forking**: Each decision's position. The authority hierarchy entries. References to specific protocols, traits, and types.

**Updating**: When a position changes, note what it was and why. Agents loading this in future sessions need to know if their cached understanding is current. If a new decision emerges that doesn't fit the 23, add it — and consider contributing it back to the [Agentic Decision Map](docs/architecture/agentic-decision-map-v3.md).

---

*Derived from the [Agentic Decision Map v3](docs/architecture/agentic-decision-map-v3.md). Last updated: 2026-03-03.*
