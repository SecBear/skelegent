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

