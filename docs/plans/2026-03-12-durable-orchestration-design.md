# Durable Orchestration Design

**Date:** 2026-03-12

## Goal
Define a durable orchestration foundation for Skelegent that is maximally composable, backend-pluggable, and consistent with the existing operator/dispatcher/effects/state/environment/context-engine split.

## Non-goals
- Do not expand Layer 0 speculatively.
- Do not force all durable backends into one replay/checkpoint implementation.
- Do not introduce a workflow DSL.
- Do not bundle governance, dashboards, org policy, or deployment UX into the durable primitive layer.

## Current architectural reading
Skelegent already has the right high-level separation:
- `Operator` is the reusable agentic unit. It may contain a turn/runtime loop, but it is not itself a single turn.
- `Dispatcher` is the immediate invocation primitive.
- `Effect` is the declaration boundary for externalized side effects.
- `StateStore` is the memory/data plane.
- `Environment` is the isolation/credential execution plane.
- `Context` is the live turn substrate.
- Orchestration already owns routing, topology, signals/queries, and the durability boundary.

The missing piece is a durable run/control substrate above Layer 0.

## Design principles
1. **Durable orchestration is orchestration-local, not Layer 0, unless later proven otherwise.**
2. **Public durable abstractions should describe run lifecycle and control, not backend replay internals.**
3. **DIY backends should compose from smaller pieces.**
4. **Sophisticated backends (Temporal-class) must be allowed to implement the top-level durable contract directly without pretending to be a SQL checkpoint engine.**
5. **Same physical backend is allowed; same logical abstraction is not required.** Memory state and durable run state may share SQLite/Postgres/etc. while remaining separate contracts.
6. **Behavior stays local to the executing process.** Durable boundaries move data, not serialized rule/middleware descriptors.

## Rejected approaches
### 1. Monolithic per-backend durable orchestrators only
Example: only `skg-orch-sqlite`, `skg-orch-temporal`, each inventing their own run model.

Rejected because it weakens mix-and-match composition and duplicates semantics.

### 2. StateStore-centered durable execution
Example: treat durable runs as just another use of `StateStore`.

Rejected because `StateStore` is clearly a memory/data-plane contract: CRUD, search, hints, links, transient scratchpad. Durable runs need lifecycle transitions, waiting, resume payloads, timers, cancellation, and possibly claiming/ownership.

## Recommended approach
Adopt a **two-level durable orchestration architecture**:

### Level A: public durable run/control contract
Create a new above-Layer-0 core crate, tentatively `orch/skg-run-core`, that defines the portable durable orchestration nouns and control-plane traits.

This layer standardizes:
- run identity
- lifecycle state
- waiting / interruption state
- resume input
- cancellation
- signal/query-style control-plane interaction
- terminal outcome view

This layer intentionally does **not** standardize:
- full event history schema
- checkpoint blob shape
- replay substrate
- worker ownership model
- exact-once/at-least-once implementation details
- backend storage schema

### Level B: optional lower pluggables for DIY backends
Provide narrower internal or semi-public components for backends that want to compose a durable orchestrator from parts:
- `RunStore`
- `WaitStore` / `InboxStore`
- `TimerStore`
- `LeaseStore` (optional)
- `ContinuationStore` / `CheckpointStore` (optional, opaque payloads)
- `RunKernel`
- `RunDriver`

These components are composition seams for SQLite/in-memory/local systems. They are not the primary public story.

## Public durable primitives
Tentative first-pass public nouns in `skg-run-core`:
- `RunId`
- `RunStatus` — `Running`, `Waiting`, `Completed`, `Failed`, `Cancelled`
- `RunView` / `RunRecord`
- `WaitPointId` (or resume token)
- `WaitReason` — non-exhaustive; likely `Approval`, `ExternalInput`, `Timer`, `ChildRun`, `Custom`
- `ResumeInput` — structured payload for satisfying a wait point
- `RunOutcome`

Tentative public traits:
- `RunStarter` — start a durable run
- `RunController` — inspect and control durable runs
- `RunQuery` / `RunLister` — optional if listing/wait queues should remain separate from control

### Important semantic split: `signal` vs `resume`
These should remain distinct.
- `signal(run_id, payload)` is asynchronous control-plane communication.
- `resume(run_id, wait_point, input)` satisfies a specific durable wait point.

Treating these as the same primitive would hide real backend differences.

## Durable kernel model
The durable core should look more like Elm than like an ad hoc runtime:
- pure transition function over durable run state
- emits orchestration commands / intents
- side effects executed outside the kernel

Conceptually:
- input event + current durable state
- -> next durable state + orchestration commands

This keeps replay/testability strong and keeps backend-specific machinery out of the semantic core.

## Lower-level pluggable components
### `RunStore`
Durable metadata and lifecycle transitions.
Stores:
- run identity
- current status
- parent/session/workflow linkage
- timestamps
- last wait point
- terminal outcome summary

### `WaitStore` / `InboxStore`
Durable pending external inputs.
Stores:
- resume payloads
- approvals
- supervisor decisions
- possibly signals that should wake waiting runs

### `TimerStore`
Durable wake-at deadlines.
A backend may implement this as a DB table, native platform timer, or derived query.

### `LeaseStore` (optional)
Claim/renew/release semantics for multi-worker execution.
Not required for a single-process SQLite runner, but the seam should exist.

### `ContinuationStore` / `CheckpointStore` (optional)
Opaque continuation payloads for backends that use snapshots/checkpoints.
This must not force Temporal-like or Restate-like backends into fake checkpoint models.

### `RunKernel`
Pure orchestration transition core.
No DB calls, no direct operator execution, no environment interaction.

### `RunDriver`
Executes runnable work:
- dispatch operator
- collect `OperatorOutput`
- interpret `Effect`s or hand them off
- commit the next durable state transition

## Relationship to current Skelegent primitives
### `Operator`
Unchanged. Operators remain reusable agentic units.

### `Dispatcher`
Still the immediate invocation primitive.
A durable orchestrator may also implement `Dispatcher`, but durable lifecycle control is a separate capability.

### `Effect`
Still the declaration boundary. Durable orchestration persists enough run state to safely resume effect processing, but it does not collapse effect semantics into run semantics.

### `StateStore`
Remains memory/data plane.
A durable implementation may use the same physical backend, but must not overload the `StateStore` contract with run lifecycle semantics.

### `Environment`
Remains isolation/credential execution plane.
Durable orchestration may delegate execution into environments, but the durable model should not be centered on environment APIs.

### `Context`
Remains the live turn substrate. Durable run state can resume into a fresh context; it is not reducible to a stored context.

## Backend composition model
### DIY/local backends
These should compose from lower-level stores and a local run driver.
Examples:
- `run/skg-run-memory`
- `run/skg-run-sqlite`
- assembled `orch/skg-orch-sqlite`

### Platform-native durable backends
These may implement the top-level run/control contract more directly.
Examples:
- `orch/skg-orch-temporal`
- future Restate-like backend

They should not be forced to expose identical checkpoint or journal semantics.

## Initial crate layout proposal
### In `skelegent/`
- `orch/skg-run-core` — public durable run/control model and core traits
- optional later: `orch/skg-run-kit` if common composition helpers deserve their own crate

### In `extras/`
Reusable run-state components:
- `run/skg-run-memory`
- `run/skg-run-sqlite`
- optional later: `run/skg-run-cozo`

Assembled durable orchestrators:
- `orch/skg-orch-sqlite`
- evolve `orch/skg-orch-temporal`

## Implementation sequencing
1. Update architecture/specs to define the durable run/control layer above Layer 0.
2. Add `skg-run-core` with public nouns and traits only.
3. Add the pure durable kernel and command model.
4. Add lower pluggable store/driver traits for DIY backends.
5. Implement SQLite reference durable composition.
6. Adapt Temporal backend to the same top-level contract.
7. Validate with a real system after primitives exist.

## Future note: Layer 0 split
Keep `layer0` as one slim stability-contract crate for now. Durable-run concerns (replay/checkpoint, long-lived worker control, orchestration queries/signals, supervision policy) should evolve above Layer 0 in orchestration/domain-core crates until they prove to be real cross-boundary contracts with multiple defended implementations and consumers. Revisit a protocol split only if a durable orchestration surface becomes both stable and mostly independent of the shared Layer 0 nouns; today the evidence points the other way — crash recovery is a backend concern, communication already fits existing seams, and splitting now would increase semver and coupling cost more than it improves composability.

## Open questions to settle before code stabilizes
1. Should `RunController` include listing/listing-by-status, or should inspection and enumeration stay separate?
2. Should wait reasons be closed enum + custom payload, or fully open tagged values from day one?
3. Should signals wake waiting runs automatically, or should wakeup remain an explicit kernel decision?
4. Do we want a public `RunEvent` envelope now, or keep event/journal/history backend-private at first?
5. How much of the lower pluggable store surface should be public versus crate-private until a second real backend consumes it?
