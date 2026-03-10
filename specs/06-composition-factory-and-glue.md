# Composition Factory and Glue

## The Question

Where does “glue” live?

- Inside orchestrator implementations?
- As a wrapper around Skelegent?

## Answer (Specification)

Composition glue that wires agents, policies, and topology belongs with orchestration implementations (Layer 2), not in `layer0`.

Reason:

- It is inherently an orchestration concern (it chooses routing/topology/policy).
- It must be shared by examples and tests to prevent drift.
- It should remain optional. `layer0` must not become a product DSL.

A separate wrapper product (outside Skelegent) can exist to provide:

- YAML workflow DSL
- Slack/email delivery
- long-running job scheduling UX

That wrapper depends on Skelegent and uses the composition factories.

## Composition Patterns

Two patterns for composing operators are idiomatic in Skelegent:

1. **Application-layer functions** (e.g., `async fn run_sweep(orch: Arc<dyn Orchestrator>, state: ScopedState)`) — deterministic, typed sequences of operator dispatches. The calling code controls the flow; this is plain Rust with no framework overhead.
2. **Orchestrating operators** — operators that receive dispatch capability via an injected `Arc<dyn Orchestrator>` and use LLM reasoning to decide what to dispatch next. The LLM drives the sequencing; the operator drives the dispatch.

Both are valid. The choice depends on whether the sequencing logic is deterministic (use functions) or requires LLM judgment (use an orchestrating operator).

**Anti-pattern: No Workflow trait.** There is no `Workflow` trait. Workflows are application-layer code, not a framework abstraction. Adding a `Workflow` trait would conflate sequencing (application concern) with execution (infrastructure concern), creating an abstraction that provides no isolation benefit but forces all callers through a fixed interface.

## Required APIs

Skelegent core should provide an *unopinionated wiring kit* plus (optionally) a small set of reference factories.

### `skg-orch-kit` (Recommended)

Define a crate named `skg-orch-kit` as the “boring glue” layer that a product like Sortie would build on.

`skg-orch-kit` MUST:

- remain a Rust API (no workflow DSL)
- allow registering arbitrary agents/operators (not just preset flows)
- support swapping implementations (mock vs real; local vs distributed) via explicit selectors
- expose a pluggable effect runner/interpreter policy for:
  - `WriteMemory` / `DeleteMemory`
  - `Delegate`
  - `Handoff`
  - `Signal`
- allow bypassing any defaults (zero lock-in)

`skg-orch-kit` MUST NOT:

- require a fixed topology enum as the only composition mechanism
- silently fall back when routing/policy inputs are unknown
- hardcode delivery integrations (Slack/webhooks/email)
- hardcode a particular durable engine (Temporal/Restate/etc.)

### Reference Factories (Optional)

Skelegent may also provide a small set of *reference* factory entrypoints that:

- accept a declarative spec (flow/topology + runtime profile)
- return a runnable orchestrator graph
- support mock and real profiles

These are allowed to be opinionated, but they must be clearly labeled as reference flows and must be bypassable.

## Sortie Integration Rule

If writing Sortie from scratch, Sortie SHOULD depend on `skg-orch-kit`.

If `skg-orch-kit` becomes constraining (e.g., it encodes product-level policy or freezes topology), Sortie SHOULD bypass it and wire directly against `layer0` instead. This “escape hatch” is not a failure; it is the signal that `skg-orch-kit` needs to become less opinionated.

## Context Transfer Rules

From `ARCHITECTURE.md §Composition`:

- **Task-only injection is the default.** When wiring a `Delegate` or `Handoff` effect,
  the `OperatorInput` passed to the sub-agent SHOULD contain only the task description
  and directly relevant context — not the full parent conversation history.
- **Enforce boundaries via infrastructure, not prompts.** Running sub-agents in a
  separate process (or separate context window) is more reliable than instructing an
  agent to "ignore" parent context. `skg-orch-kit` wiring SHOULD respect this.
- **Summary injection for multi-level delegation.** When a parent delegates to a child
  that further delegates, prefer passing a summary of the parent's work product rather
  than the full inherited context. Full context inheritance does not scale past 2-3 levels.
- **Result routing**: two-path preferred — full output to persistent storage (via
  `Effect::WriteMemory`) for audit, summary to parent context for token efficiency.

## Current Implementation Status

`skg-orch-kit` exists as the unopinionated wiring kit.

Still required for “core complete”:

- end-to-end examples/tests that exercise `skg-orch-kit` as the shared wiring layer (to prevent drift)
- a reference effect execution story that is explicitly documented and test-proven (delegate/handoff/signal/state)
