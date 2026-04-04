> **RETIRED — superseded by specs/v2/. Do not use for new implementation work.**

# Orchestration Core

## Purpose

Orchestration is the outer control plane. It coordinates many operator cycles.

It owns:

- routing/topology (who runs next)
- concurrency (`dispatch_many`)
- immediate invocation through `Dispatcher`
- workflow control surfaces (`signal`, `query`)
- optional durable run/control above Layer 0
- retry authority

## Protocol

`layer0::Dispatcher`, `skg_effects_core::Signalable`, and `skg_effects_core::Queryable` together form the orchestration boundary:



**Dispatcher:**

- `dispatch` — invoke an operator by ID



**Signalable:**

- `signal` — fire-and-forget inter-workflow messaging



**Queryable:**

- `query` — read-only workflow state inspection



Related: `dispatch_many()` is a free function in `skg-orch-kit` for concurrent dispatch.


Durable run/control lives above Layer 0 in `skg-run-core` via traits such as `RunStarter` and `RunController`. Those surfaces cover starting a run, inspecting status, signalling it, resuming a specific wait point, cancelling it, and reading its terminal outcome.

The Layer 0 protocol does not prescribe whether execution is local, remote, durable, or ephemeral. Durable run/control is a higher-level orchestration contract above Layer 0; it must not be smuggled into `Dispatcher` or `StateStore`.

## Durable Run/Control Boundary

Portable durable orchestration surfaces describe run lifecycle and control-plane semantics such as starting a run, inspecting status, signalling it, resuming a specific wait point, cancelling it, and reading its terminal outcome.

They intentionally do **not** standardize backend internals such as:

- event history schema
- checkpoint blob shape
- replay substrate
- journal format
- worker lease/claim model
- storage schema

`signal` and durable `resume` are distinct concepts. Signals are asynchronous control-plane messages. Resume satisfies a specific durable wait point with structured input.

`StateStore` remains the memory/data-plane contract even when a backend reuses the same physical database for both memory and durable orchestration data.

See `specs/14-durable-orchestration-core.md` for the narrow durable-run contract this architecture intends to stabilize above Layer 0.

## Retry and Durability

### Retry Authority

Retry has a single authority — the orchestrator. SDK-level automatic retry and
orchestrator-level retry MUST NOT coexist: if both fire, a single operator failure
produces multiple retry attempts with conflicting backoff and budget accounting.

Concrete requirement: when wiring a provider client, disable any built-in SDK-level
automatic retry. The orchestrator (or the caller of `dispatch`) is the sole site that
decides whether and when to retry.

The turn classifies errors as retriable or not:
- `ExitReason::BudgetExhausted` and `ExitReason::SafetyStop` — never retry without
  changing the budget limit or context respectively.
- `ExitReason::Error` — retriable depending on the error kind (transient vs permanent).
- `ExitReason::Timeout` — retriable via a new invocation.

### Durability Boundary

Local orchestration provides no durability — acceptable for short, low-stakes tasks. Durable orchestration adds a run/control layer above Layer 0. Backends may implement crash recovery through replay, checkpoints, append-only journals, platform-native event history, or other mechanisms. The same `Operator` implementation works in all of these deployments — durability is a configuration and infrastructure choice, not an operator-level change.

Skelegent does not define a workflow DSL or serialized behavior-descriptor format for durable orchestration. Durable backends move data across boundaries; behavior stays local to the executing process.

## Required “Core Complete” Features

Even if technology-specific orchestrators are stubs, Skelegent core needs a reference orchestration story that is testable.

Minimum requirements:

1. A local reference orchestrator implementation.
2. A reference composition/glue layer that produces runnable graphs.
3. A reference effect interpretation pipeline for delegate/handoff/signal/state.

## Current Implementation Status

- `skg-orch-local` exists as an in-process dispatcher for immediate invocation.
- Local orchestration currently tracks signals in memory for testing/querying; that is an implementation detail, not the durable run contract.
- `skg-orch-kit` provides composition wiring.
- `skg-run-core` defines a portable durable run/control surface above Layer 0 without standardizing replay/checkpoint/journal internals.

Still required:

- end-to-end examples exercising orchestration graph wiring
- backend implementations that map their own recovery substrate into the durable run/control surface while preserving backend freedom over replay/checkpoint/journal internals
