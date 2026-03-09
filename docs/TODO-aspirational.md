# Aspirational Features

Features described in specs or architecture docs that are not yet implemented.
These were extracted during the pre-merge audit of `feature/unified-architecture`.
The specs have been updated to remove false "implemented" claims; these items
are tracked here as candidates for future work.

## A1: ScopedState capability type

**Source**: spec 03 (originally described injected `ScopedState` capability)
**Current reality**: Operators access own-scope state via `FlushToStore`/`InjectFromStore` ops or by receiving a `StateStore` reference at construction time. No dedicated `ScopedState` type exists.
**Question**: Is a dedicated type worth it, or is the current composition-based approach sufficient?

## A2: Turn-level middleware boundaries

**Source**: spec 04, spec 09 (described PreInference, PostInference, ExitCheck, SubDispatchUpdate, PreSteeringInject, PostSteeringSkip, PreMemoryWrite as named boundaries)
**Current reality**: Three middleware stacks exist at protocol boundaries (`DispatchMiddleware`, `ExecMiddleware`, `StoreMiddleware`). Turn-internal boundaries are handled by the Rule system in `neuron-context-engine` (before/after triggers on specific ops).
**Question**: Are named turn-level boundaries needed as first-class traits, or does the Rule system cover the use cases?

## A3: Four-zone TieredStrategy compaction

**Source**: spec 04 (Pinned/Active/Summary/Noise zones, `Summariser` trait, `max_messages` threshold, `CompactionError` types)
**Current reality**: `CompactionRule` with `sliding_window` and `policy_trim` strategies. Async `summarize`/`extract_cognitive_state` functions exist but are standalone, not wired into a zone-partitioned strategy. No `Summariser` trait, no `CompactionError` type, no `max_messages` config.
**Question**: Is the zone model the right abstraction, or does the current ops-based composition (rule + standalone functions + store ops) cover the same ground more flexibly?

## A4: Step/loop detection limits

**Source**: spec 04 (`max_sub_dispatches`, `max_repeat_dispatches`, `Custom("stuck_detected")`, `Custom("loop_detected")`)
**Current reality**: `BudgetGuard` checks cost, turns, duration, and tool calls. No sub-dispatch counting or repeat-detection logic exists.
**Question**: Is this needed at the BudgetGuard level, or should loop detection be a separate Rule?

## A5: Composition proof examples

**Source**: spec 11 (daily_digest, triage, provider_parity examples)
**Current reality**: Only `custom_operator_barrier` exists. `tests/poc.rs` and `tests/cross_provider.rs` cover composability and provider parity respectively.
**Question**: Should these be standalone binaries, workspace examples, or are the existing tests sufficient?

## A6: Conversation persistence

**Source**: implied by state architecture
**Current reality**: `Context` is in-memory only. `FlushToStore`/`InjectFromStore` can persist/restore specific data, but there's no save/restore for a full `Context` across sessions.
**Question**: What's the right granularity — full context serialization, or the current explicit-ops approach?

## A7: Vector search in state stores

**Source**: extras state architecture
**Current reality**: `StateStore::search()` does text search only. `neuron-state-sqlite` in extras could support `sqlite-vec` for vector similarity.
**Question**: Should this be a separate trait (`VectorStore`), an extension to `StateStore`, or a feature flag on specific backends?

## A8: MCP client integration

**Source**: architecture roadmap
**Current reality**: `neuron-mcp` exists as an MCP server (exposing neuron operators as MCP tools). No MCP client (discovering/calling external MCP tools).
**Question**: New crate? Extension to `neuron-tool`? How does tool discovery compose with `ToolFilter`?

## A9: Durable orchestrator

**Source**: spec 03 mentions durable orchestration serializing effects
**Current reality**: `neuron-orch-local` is in-process only. `neuron-orch-kit` provides composition but no persistence.
**Question**: Checkpoint/replay vs event-sourcing vs Temporal-style? What's the minimum viable durability?

## A10: HTTP serving harness

**Source**: architecture roadmap
**Current reality**: No HTTP layer. Operators are called programmatically.
**Question**: Axum? Tower middleware? SSE streaming? What's the API shape?
