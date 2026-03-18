# Layer 0 Protocol Contract

## Purpose

`layer0` is the stability contract. It defines:

- protocol traits (object-safe)
- message/effect types that cross boundaries
- error vocabulary
- IDs and scopes
- secret source vocabulary

It must be easy for any implementation to adopt and must avoid coupling to any specific runtime.

## Protocol Traits

Layer 0 defines four primary protocol traits:

- `Operator`: one unit of agent work
- `Dispatcher`: dispatch/invoke operators

`Signalable` (fire-and-forget inter-workflow messaging) and `Queryable` (read-only workflow state queries) are defined in Layer 2 (`skg-effects-core`), not Layer 0.

- `StateStore` + `StateReader`: persistence and retrieval

- `Environment`: isolated execution boundary

Middleware traits/stacks are cross-cutting protocol surfaces, but not separate primary runtime protocols. Lifecycle coordination and observability policy live above Layer 0 unless promoted into a real cross-boundary contract.

Layer 0 also defines the cross-boundary nouns that travel between components:

- `Message` / `MessageMeta` / `Role` for conversational state that crosses runtime, store, and dispatch boundaries
- `Effect` and `Scope` for declared side effects and state addressing
- `CompactionPolicy` as a message-level advisory hint that travels with the message

Budget enforcement, compaction coordination, observability plumbing, and other lifecycle policy decisions live above Layer 0 unless and until they become real cross-boundary contracts.
## Type Inventory

Core types that cross layer boundaries:

- `SubDispatchRecord` — record of a single sub-operator dispatch within a turn. Fields: `name: String`, `duration: DurationMs`, `success: bool`. Accumulated in `OperatorMetadata.sub_dispatches` and used to report what was dispatched during a turn.
- `ToolMetadata` — metadata that makes an operator callable as an LLM tool. Fields: `name: String`, `description: String`, `input_schema: serde_json::Value`, `parallel_safe: bool`. Not part of the `Operator` trait — attached at registration time by the orchestrator, so the same operator can be registered with different metadata in different contexts.

## Compatibility Rules

- Traits must remain object-safe and usable behind `dyn` for composition.
- Wire types must remain serde-serializable.
- Additive changes are preferred; breaking signature changes should be avoided until a planned breaking release.

## Current Implementation Status

Implemented in this repo:

- `layer0` exists and defines the above interfaces.

Still required to be considered “core complete”:

- A clear, user-facing contract doc describing semantics (not just trait signatures).
- A compatibility story for versioning and deprecation.

