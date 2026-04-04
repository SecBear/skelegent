# Kernel Principles and Layers

## Purpose

Define what belongs in the v2 kernel and what must remain outside it.

## Kernel Purity Rules

Layer 0 in v2 contains only stable cross-boundary nouns:

- IDs and provenance metadata
- `DispatchContext`
- invocation request/handle/outcome/wait value types
- executable intents
- semantic execution events
- capability descriptors and discovery traits
- content and artifact wire types
- base protocol traits and extension accessors

Layer 0 must not contain:

- provider token-chunk wire formats
- workflow DSLs
- backend internals such as checkpoints, leases, replay logs, or worker claims
- product-specific routing policy
- observational data disguised as executable intent

## Revised Layer Model

- Layer 0: protocol kernel
- Layer 1: turn/runtime semantics, provider projection, scheduling, and context mutation
- Layer 2: orchestration and durable control
- Layer 3: state and environment implementations
- Layer 4: protocol bridges and adapters such as MCP and A2A
- Layer 5: products, examples, and governance surfaces built on top

This keeps the semantic kernel distinct from both backend implementations and
external protocol adapters.

## Retained Core Traits

V2 keeps these surfaces conceptually intact:

- `DispatchContext` as the shared execution context
- `Dispatcher` as the immediate invocation primitive
- a declarative environment contract rather than hard-coding container or platform semantics

## New Sibling Protocol

V2 introduces `CapabilitySource` as a sibling read surface to `Dispatcher`.

`Dispatcher` answers: "invoke this now."

`CapabilitySource` answers: "what exists and how may it be invoked."

The two concerns stay separate so callers may have:

- discovery without invocation
- invocation without discovery
- multiple discovery sources feeding one dispatcher

## Relationship to Current Specs

This spec supersedes the architectural direction in `specs/01-architecture-and-layering.md`
for the v2 track while preserving the core invariants around `DispatchContext`,
`Dispatcher`, and the declaration/execution boundary.

## Compatibility Rule

If a proposed v2 addition can live as an adapter or extension trait instead of a
new Layer 0 noun, it must not be added to Layer 0.

## Minimum Proving Tests

- A v2 reference index proving every Layer 0 noun defined here is technology-agnostic.
- A dependency audit proving protocol bridges and products do not leak back into Layer 0.
