# Durable Orchestration Core

## Purpose

Define the portable durable run/control contract that lives above Layer 0.

This spec is intentionally narrow. It covers the public semantics that durable orchestrators should share without forcing every backend into the same replay, checkpoint, or storage model.

## Scope

A portable durable orchestration surface MAY standardize:

- run identity
- lifecycle state and terminal outcome
- wait-point identity and wait reason
- control-plane operations such as start, inspect, signal, resume, and cancel
- read models for current run status

It MUST NOT standardize backend internals such as:

- event history schema
- checkpoint payload shape
- replay/journal substrate
- worker claiming or lease algorithms
- storage schema or table layout

Those remain backend implementation details.

## Relationship to Existing Layer 0 Primitives

`Dispatcher` remains the immediate invocation primitive. Calling `dispatch` asks an implementation to invoke an operator now; it is not the durable run/control contract.

Durable run/control lives above Layer 0 in `skg-run-core` via `RunStarter`, `RunController`, and the associated durable nouns. Signals remain asynchronous control-plane messages; durable `resume` is a different operation that satisfies a specific durable wait point with structured input. Signals and resume MUST NOT be collapsed into one primitive.

`StateStore` remains the memory/data-plane contract. A backend MAY store both memory and durable-run metadata in the same physical system, but durable lifecycle semantics MUST NOT be presented as `StateStore` behavior.

## Backend Freedom Below the Durable Boundary

DIY/local durable backends MAY compose a durable orchestrator from smaller stores and drivers.
Platform-native backends MAY implement the public durable run/control contract directly.

Neither style is canonical for all backends. Skelegent MUST NOT require SQLite-like checkpoints, Temporal-like histories, or any other single replay substrate as the universal model.

## Behavior and Serialization

Durable orchestration moves data across boundaries, not behavior descriptors. Skelegent does not define a workflow DSL, serialized middleware graph, or serialized rule language for durable execution.

If a durable backend needs per-run variation, that variation travels as data in the run input/configuration. The executing process constructs behavior locally from its own operator factories and middleware stacks.

## Current Implementation Status

Implemented today:

- immediate invocation via `Dispatcher`
- local orchestration wiring via `skg-orch-local` and `skg-orch-kit`
- portable durable run/control nouns, pure kernel transitions, and lower backend seams in `skg-run-core`
- local signal/query behavior sufficient for current in-process orchestration tests

Still required:

- backend implementations that map their own recovery substrate into that public contract
- tests proving signal, resume, cancel, and outcome semantics across distinct backends without standardizing backend internals
