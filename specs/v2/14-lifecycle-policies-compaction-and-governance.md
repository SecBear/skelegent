# Lifecycle Policies, Compaction, and Governance

## Purpose

Define the v2 lifecycle layer above the kernel: memory persistence policy,
compaction policy, crash-recovery posture, budget governance, and lifecycle
queries.

Lifecycle concerns span turns and agents. They are not reducible to one turn or
one composition pattern.

## Lifecycle Boundary

Layer 0 contributes stable nouns and hints used by lifecycle systems:

- message metadata
- scopes
- intents
- semantic events
- outcomes and waits

Lifecycle policy itself lives above Layer 0.

## Memory Persistence Policy

Lifecycle policy decides when important state is persisted.

V2 preserves the current strong position:

- pre-compaction flush is mandatory when destructive compaction would otherwise lose important state
- termination and handoff may require explicit work-product persistence
- continue-as-new boundaries must explicitly carry forward needed state

The kernel does not decide what is important. Lifecycle policy does.

## Compaction Policy

Compaction is a runtime/orchestration concern, not a base state trait concern.

V2 requires support for the following concepts:

- compaction reserve must never be zero
- message-level hints may influence compaction treatment
- destructive compaction must be preceded by any required flush
- fresh summary replacement is preferred over recursive summary-of-summary degradation

Compaction strategies remain replaceable policies.

## Continue-as-New and Fresh Starts

V2 distinguishes:

- fresh session/start over
- continue-as-new with explicit carried state
- local compaction within the current execution

These are different lifecycle operations and must not be collapsed into one
"reset" concept.

## Crash-Recovery Posture

Lifecycle policy determines the recovery posture of a system:

- no recovery
- checkpoint-based recovery
- event/history replay
- durable execution replay

The shared invocation and wait vocabulary from v2/09 must remain valid across
all of these, while backend internals remain free.

## Budget Governance

V2 preserves the single-authority principle:

- each failure/budget class has one retry or halt owner
- local runtime enforcement may stop an invocation on local limits
- broader governance policy such as downgrade, reschedule, or workflow halt belongs above the runtime

Budget governance includes:

- token/cost limits
- turn and tool-call limits
- wall-clock limits
- aggregate workflow/session budgets

## Lifecycle Queries

Lifecycle systems require read-only projections for:

- compaction state and decisions
- current and aggregate budgets
- carried-forward state boundaries
- recovery posture and current wait state

These queries are above Layer 0 and must remain read-only.

## Compatibility Rules

- Compaction policy must remain separable from state backend implementation.
- Budget governance must remain separable from model selection policy.
- Crash recovery internals must remain backend-specific.
- Lifecycle queries must not mutate runtime or durable state.

## Relationship to Current Specs

This spec refines the lifecycle sections of `ARCHITECTURE.md`,
`specs/04-operator-turn-runtime.md`, `specs/09-hooks-lifecycle-and-governance.md`,
`specs/11-testing-examples-and-backpressure.md`, and `specs/14-durable-orchestration-core.md`
for the v2 track.

## Minimum Proving Tests

- A destructive compaction path refuses to proceed after a failed required flush.
- Fresh summary replacement avoids recursive summary chaining across repeated compaction cycles.
- Local runtime budget stops and orchestration-level governance decisions use one shared outcome vocabulary without sharing one implementation substrate.
- A lifecycle query surface can report compaction and budget state without mutating execution.
