# Composition Patterns and Control Surfaces

## Purpose

Define how v2 treats multi-agent composition without turning one composition
pattern into a kernel requirement.

Composition is where agent systems differ most. V2 standardizes the control
surfaces and shared vocabulary needed to build composition patterns. It does not
standardize one workflow graph abstraction.

## Composition Primitives vs Kernel Nouns

The kernel provides nouns used by composition:

- invocation
- capabilities
- outcomes
- waits
- intents
- semantic events
- scopes and shared state

Composition patterns are built above those nouns.

Examples of composition patterns:

- chain
- fan-out / fan-in
- delegate
- handoff
- supervision
- evaluator/optimizer loops
- governed execution with observers/guardrails

These are orchestration patterns, not Layer 0 protocol kinds.

## Canonical Composition Decisions

Every composition system must answer at least:

- what context the child receives
- how results flow back
- how long children live
- how agents communicate
- who may observe or intervene

These are control-surface decisions, not kernel enum variants.

## Child Context Policy

V2 preserves the current preferred policy:

- task-only injection is the default
- summary injection is preferred over full inheritance for deeper delegation
- structural isolation is stronger than prompt-only isolation

Full context inheritance remains allowed as an orchestration policy, not a
kernel assumption.

## Result Routing Policy

V2 preserves the current preferred result routing:

- direct injection is acceptable for simple cases
- two-path routing is preferred at scale:
  - concise result to parent context
  - full result to storage or audit sink

This keeps parent contexts small without losing traceability.

## Child Lifecycle Policy

V2 preserves:

- ephemeral children as the default
- long-lived children as opt-in orchestration policy
- conversation-scoped handoff as distinct from delegation

Lifecycle duration is an orchestration choice, not a property of the kernel
invocation trait.

## Communication Surfaces

V2 composition may use:

- synchronous invocation/return
- shared state
- signals
- semantic event streams
- external protocols

The kernel must provide the nouns needed for these surfaces without forcing one
communications substrate.

## Observation and Governance in Composition

Observation and intervention compose orthogonally with every pattern.

An observer, guardrail, or supervisor may watch:

- a chain
- a fan-out
- a delegated child
- a long-lived team

This is why observation/intervention is a separate concern from topology.

## No Workflow Trait

V2 preserves the current position that there is no universal Workflow trait in
the kernel.

Typed workflows, supervisors, and graphs may exist in orchestration or product
layers, but the protocol kernel should not hard-code one workflow abstraction.

## Compatibility Rules

- Composition patterns must be expressible using shared kernel nouns plus orchestration policy.
- Child-context policy must remain configurable without changing the invocation contract.
- Result routing must remain separable from child lifecycle and communications choice.
- Observation/intervention must remain orthogonal to topology.

## Relationship to Current Specs

This spec refines `specs/05-orchestration-core.md`,
`specs/06-composition-factory-and-glue.md`, and the composition sections of the
current architecture for the v2 track while preserving the "no Workflow trait"
position.

## Minimum Proving Tests

- A delegate pattern and a handoff pattern can be implemented with the same shared kernel nouns but different orchestration policies.
- A fan-out/fan-in pattern can return summaries to the parent while storing full child outputs out of band.
- A supervisor can observe and redirect children without requiring a dedicated workflow DSL in Layer 0.
- A composition implementation can use synchronous invocation, signals, or shared state without changing kernel value types.
