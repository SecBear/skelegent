# Scheduling and Turn Execution

## Purpose

Replace ad hoc tool concurrency with an explicit planning model.

## Existing Planner Vocabulary

V2 promotes the existing turn-kit vocabulary into the runtime contract:

- concurrency classification
- dispatch planning
- batch execution
- steering between execution boundaries

The runtime must stop using special-case heuristics such as "all tools are
parallel-safe, therefore `join_all`."

## Scheduling Rule

Scheduling is a Layer 1 runtime concern informed by capability descriptors.

Layer 0 exposes facts. The runtime planner decides batching and execution order.

## Required Runtime Behavior

- sequential and concurrent tool execution must pass through the same rule-firing boundaries
- telemetry and metrics must observe both sequential and concurrent executions uniformly
- approval checks must occur before dispatching planned tool calls
- partial planner failure semantics must be explicit and test-proven

## Default Planner

V2 requires a simple reference planner:

- shared capabilities may batch together
- exclusive capabilities form barriers
- order is preserved unless the planner explicitly documents reordering

More advanced planners may exist, but the default reference planner must be
documented and deterministic.

## Relationship to Current Specs

This spec supersedes the current ad hoc tool concurrency guidance in
`specs/04-operator-turn-runtime.md` for the v2 track and promotes the existing
turn-kit planning vocabulary into the main runtime story.

## Minimum Proving Tests

- Sequential and concurrent execution fire the same rule boundaries and telemetry updates.
- Mixed shared/exclusive batches preserve the documented barrier semantics.
- Planner choice changes execution strategy without changing capability descriptors or operator code.
