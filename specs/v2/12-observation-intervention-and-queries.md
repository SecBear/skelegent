# Observation, Intervention, and Queries

## Purpose

Define the v2 distinction between semantic observation, active intervention,
guardrail policy, oracle-style consultation, and read-only queries.

These concerns are often collapsed into one "observer" concept in agent systems.
V2 keeps them separate so the architecture does not confuse visibility,
enforcement, and ordinary capability use.

## Core Distinctions

### Observation

Observation is read-only visibility into execution through semantic events.

Observation answers:

- what happened
- in what order
- with what correlation and provenance

Observation does not by itself modify runtime behavior.

### Intervention

Intervention is an explicit control-plane action that changes the course of an
active or future execution.

Examples:

- halt
- cancel
- inject context material
- approve or reject a wait
- redirect a task
- adjust budget or control policy through an orchestration surface

Intervention is not the same thing as observing the events that led to the
decision to intervene.

### Guardrail Policy

Guardrails are policy logic built on top of observation and intervention.

A guardrail may:

- inspect semantic events
- inspect typed runtime/control data at defined boundaries
- choose to allow, modify, suspend, or halt execution

Guardrails are not a new kernel primitive. They are a policy layer built on top
of event, query, and intervention surfaces.

### Oracle / Consultation

Oracle-style consultation is ordinary capability invocation used to obtain
guidance or analysis.

It is not observation, even if the consulted capability is advisory.

An oracle is represented as a normal capability:

- tool
- agent
- service

It is invoked explicitly through normal invocation surfaces.

### Query

A query is a read-only projection over current or durable state.

Queries answer things like:

- what is the current run state
- what is currently in active context
- what capabilities are available
- what wait is active

Queries do not subscribe to streams and do not mutate execution.

## Kernel Responsibilities

Layer 0 in v2 must standardize:

- semantic execution events
- event metadata and correlation fields
- shared value types needed to interpret outcomes and waits

Layer 0 must not standardize one universal observer runtime or one universal
query service.

## Runtime Responsibilities

The runtime owns observation surfaces for turn-local execution, including:

- active context event emission
- provider-to-semantic projection
- current-context introspection read models
- runtime-local intervention boundaries where allowed

The runtime must make clear which intervention points are supported and when they
are applied.

## Orchestration Responsibilities

Orchestration owns cross-turn and cross-agent observation and intervention:

- run-level subscriptions
- wait approval and resume decisions
- cancellation and redirection
- budget governance
- control-plane queries

Orchestration may expose richer control surfaces than the runtime, but it must
reuse the same shared value vocabulary where applicable.

## Queries

V2 requires explicit read-only query surfaces above Layer 0 for at least:

- active context introspection
- current invocation/run state
- capability inspection
- wait and suspension inspection

These are projections. They are not the same thing as:

- state search
- durable event history
- log aggregation

## Boundary Rules

- Observation is read-only.
- Intervention is explicit and typed.
- Guardrails are policy built on top of observation and intervention.
- Oracle consultation is ordinary invocation, not observation.
- Queries are read-only projections, not streams.

Any implementation that treats all five as one mechanism is architecturally
mixing concerns.

## Compatibility Rules

- Semantic event consumers must not need provider transport chunks to understand execution.
- Query surfaces must not mutate state or execution.
- Intervention surfaces must be explicit about timing and effect.
- Oracle-style capabilities must remain usable even when no observer or guardrail system is configured.

## Relationship to Current Specs

This spec refines the observation/intervention portions of
`specs/09-hooks-lifecycle-and-governance.md` and the current architecture's
composition discussion for the v2 track. It also sharpens the distinction between
event streaming, middleware policy, and query read models.

## Minimum Proving Tests

- A read-only observer can reconstruct turn progress from semantic events without intervention privileges.
- A guardrail can halt or suspend execution through an explicit intervention path without redefining the event model.
- A query surface can report current active context and current wait state without subscribing to live streams.
- An oracle capability can be invoked normally without requiring any observer infrastructure.
