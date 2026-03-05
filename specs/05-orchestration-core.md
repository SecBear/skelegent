# Orchestration Core

## Purpose

Orchestration is the outer control plane. It coordinates many operator cycles.

It owns:

- routing/topology (who runs next)
- concurrency (`dispatch_many`)
- workflow control surfaces (signals, queries)
- durability/replay boundary (implementation dependent)

## Protocol

`layer0::Orchestrator` is intentionally small:

- `dispatch`
- `dispatch_many`
- `signal`
- `query`

The protocol does not prescribe whether execution is local, remote, durable, or ephemeral.

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

Local orchestration provides no durability — acceptable for short, low-stakes tasks.
Durable orchestration (e.g. Temporal, Restate) provides checkpoint/replay recovery.
The same `Operator` implementation works in both deployments — durability is a
configuration and infrastructure choice, not an operator-level change.

## Required “Core Complete” Features

Even if technology-specific orchestrators are stubs, Neuron core needs a reference orchestration story that is testable.

Minimum requirements:

1. A local reference orchestrator implementation.
2. A reference composition/glue layer that produces runnable graphs.
3. A reference effect interpretation pipeline for delegate/handoff/signal/state.

## Current Implementation Status

- `neuron-orch-local` exists as an in-process dispatcher.
- Signals are tracked in-memory per workflow via a per-workflow signal journal; `query` returns the signal count.
- `neuron-orch-kit` provides composition wiring.

Still required:

- end-to-end examples exercising orchestration graph wiring

