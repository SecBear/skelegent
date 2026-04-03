# V2 Core Philosophy

Skelegent v2 should be designed as a protocol kernel for programmable agentic
systems.

The goal is not to standardize one agent pattern. The goal is to standardize the
small set of cross-boundary nouns and transitions that remain useful when models,
providers, runtimes, protocols, memory systems, and execution substrates change.

This document is a design memo for the v2 track. It is not yet a replacement for
`ARCHITECTURE.md`. The current architecture remains authoritative for shipped
behavior. This memo explains the target philosophy the v2 specs are trying to
make concrete.

## What Skelegent Is

Skelegent is not:

- a ReAct framework
- a coding agent framework
- a workflow DSL
- an MCP wrapper
- a durable workflow engine
- a single product with one blessed orchestrator pattern

Skelegent is:

- a protocol kernel for invocation, state, capabilities, events, and control
- a runtime substrate for many agent patterns
- a foundation that lets products and adapters sit above stable boundaries

If a future system can be described in terms of the kernel nouns without
changing the kernel, Skelegent is doing its job.

## Core Commitments

### 1. Standardize Nouns, Not Patterns

The kernel should standardize stable nouns and transitions:

- invocation context
- invocation handles
- outcomes and waits
- executable intent
- semantic execution events
- capabilities and discovery
- content and artifacts
- scopes and identities
- structured protocol errors

The kernel should not standardize product patterns:

- ReAct loops
- evaluator/optimizer loops
- supervisor topologies
- coding-agent memory layouts
- one canonical routing graph

Patterns belong to runtimes, orchestrators, and products. Nouns belong to the
kernel.

### 2. Separate Facts From Policy

The kernel should expose facts:

- this invocation suspended for approval
- this capability is exclusive
- this artifact was produced
- this state backend supports search

The kernel should not hard-code policy:

- summarize at N tokens
- always delegate task-only
- always run ReAct
- always route hard tasks to the strongest model

Policies must be swappable without changing the kernel.

### 3. Separate Semantic Events From Transport Chunks

Provider token deltas, SSE frames, JSON-RPC notifications, and MCP streaming
chunks are transport-level details.

The kernel event plane should be semantic and replay-meaningful:

- inference started
- tool call assembled
- tool result received
- intent declared
- outcome reached
- artifact produced

Token deltas may be consumed by runtimes, but they are not the kernel event
plane.

### 4. Separate Executable Intent From Observation

If something can be executed or replayed as a side effect, it is an intent.
If something exists to be watched, queried, logged, audited, or rendered, it is
an event.

This split is fundamental:

- intents are for execution and replay
- events are for observation and control-plane visibility

Blurring the two creates confusion in middleware, replay, durable execution, and
external bridges.

### 5. Separate Discovery From Invocation

Capability discovery and capability invocation are different concerns.

`Dispatcher` should remain invocation-only.

`CapabilitySource` should answer what exists, what it accepts, and what facts are
known about it.

This separation lets Skelegent support:

- invocation without discovery
- discovery without invocation
- many discovery sources with one invocation path

### 6. Separate Substrate From Agent-Facing Projection

Many things agents experience as "tools" are projections of deeper substrate
concerns.

Examples:

- memory tools are one projection of persistent state and retrieval policy
- context snapshots are one projection of active context state
- search tools are one projection of state or environment indexes

The substrate belongs in the protocol architecture. The agent-facing tool surface
belongs in runtime and product layers.

### 7. Share Value Types Across Immediate and Durable Control

Immediate and durable execution should share value-level vocabulary:

- outcomes
- waits
- resume inputs
- semantic execution events

They should not be forced into the same lifecycle substrate.

Durable execution has backend-specific concerns:

- checkpoints
- replay
- leases
- timer persistence
- history storage

Those stay above the kernel.

### 8. Prefer Extension Families Over Monoliths

As the ecosystem grows, the kernel must stay stable.

That means base traits should remain small, while optional capabilities grow
through extension families or descriptor fields.

This applies especially to:

- state
- providers
- environments

The alternative is an ever-growing base trait that eventually encodes too many
implementation assumptions.

## The Cohesive Picture

Skelegent v2 should be organized around five layers of concern:

### Kernel

Stable protocol nouns:

- `DispatchContext`
- invocation handles
- `Outcome`
- `WaitReason`
- `Intent`
- `ExecutionEvent`
- `CapabilityDescriptor`
- `Content`
- `Artifact`
- `Scope`
- `ProtocolError`
- base state/environment traits

### Runtime

Turn-local machinery:

- context assembly
- provider projection
- scheduling and planning
- backfill
- compaction policy
- structured output handling

### Orchestration

Cross-turn and multi-agent control:

- delegation
- handoff
- result routing
- supervision
- retry and recovery policy
- budgets
- durable lifecycle

### Bridges

External protocol projections:

- MCP
- A2A
- provider-specific tool schemas
- transport adapters

These should translate kernel semantics. They should not define new internal
semantic worlds.

### Products

Opinionated systems built above the substrate:

- coding agents
- research agents
- CLI harnesses
- guardrail systems
- memory tools

Products are free to be opinionated. The kernel is not.

## Where Session State, Memory, and Context Fit

These concepts must be kept distinct.

### Session State

Session state is infrastructure-facing state tied to a session identity.

It includes:

- conversation/session identifiers
- session-scoped persistence
- approval and wait continuity
- resumable session metadata

This belongs to the kernel and state/orchestration boundaries, not only to
agent-facing tools.

### Active Context

Active context is the read model of what the agent currently sees.

It includes:

- assembled messages
- system and identity material
- current working set of tool results
- context-local annotations such as compaction policy and salience

This is primarily a runtime concern.

### Persistent State and Memory

Persistent state is the substrate.

Memory is the agent-relevant slice of that substrate plus retrieval and curation
policy.

Memory is not just a tool. Tools are one way for an agent to access memory.

### Structural State

Filesystems, repositories, APIs, and live environments are not the same as
persistent memory stores, even if agents use them as memory-like sources.

They should be represented as environment or capability surfaces, not collapsed
into one memory abstraction.

## How To Use This Memo

Use this memo as the architectural lens for evaluating any proposed v2 change.

Ask:

1. Is this a stable cross-boundary noun, or a pattern/policy?
2. Is this executable intent, or observation?
3. Is this semantic, or transport-level?
4. Is this substrate, or an agent-facing projection?
5. Does this belong in the kernel, runtime, orchestration, bridge, or product layer?

If the answer is unclear, the boundary is probably not yet designed cleanly
enough.
