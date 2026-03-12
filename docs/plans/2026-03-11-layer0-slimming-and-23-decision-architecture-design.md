# Layer0 Slimming and 23-Decision Architecture Design

**Date:** 2026-03-11
**Status:** Approved design direction

## Goal

Reduce Layer 0 to protocol-only essentials, move lifecycle coordination above Layer 0, make the runtime boundary honest at inference time, and validate full 23-decision composability with a real supervising multi-agent system under `golden/projects/skelegent/`.

## Design Decisions

### 1. Layer 0 is protocol-only

Layer 0 exists only for:
- stable protocol traits
- stable invocation/result wire types
- stable cross-boundary nouns shared by multiple implementations
- message-level hints that travel with data

Layer 0 must not contain aspirational runtime vocabulary, internal coordination models, or implementation-local event taxonomies.

### 2. Keep only current cross-boundary essentials in Layer 0

Keep in Layer 0:
- protocol traits: `Operator`, `Dispatcher`, `StateStore`, `StateReader`, `Environment` / `EnvironmentSpec`
- invocation/result types: `OperatorInput`, `OperatorOutput`, `OperatorConfig`, `ExitReason`
- shared substrate types: `Message`, `Role`, `Effect`, `Scope`, `OperatorId`, `SessionId`, `WorkflowId`
- message-level hint: `CompactionPolicy`

Remove from Layer 0 unless a current cross-boundary need can be defended:
- `BudgetEvent`
- `CompactionEvent`
- other runtime-local lifecycle vocabularies
- observation/intervention mechanics
- compaction strategies and telemetry structures

### 3. Support all 23 golden decisions above Layer 0

The 23 decisions are not expressed by bloating the protocol. They are expressed by constructor-time wiring, runtime rules, configuration surfaces, and orchestration policy.

#### `skg-context-engine` owns turn-local mechanics
- context assembly substrate
- inference boundary
- backfill
- rule system
- turn-local budget guards
- compaction rules and strategies
- streaming observation
- intervention channel

#### `skg-orch-kit` owns orchestration/composition policy
- typed dispatch
- child-context policy
- result routing policy
- budget aggregation and governance
- effect middleware
- observation adapters and supervisor/oracle patterns
- compaction coordination and pre-flush policy

#### `extras` owns reusable heavy implementations
- provider routers
- durable orchestration backends
- sandboxed execution environments
- concrete state backends
- reusable policy implementations

#### `golden/projects/skelegent/` owns real validating systems
This is where actual systems using skelegent should be built to validate the architecture under realistic pressure.

### 4. Make inference a real runtime boundary

Current mismatch:
- rules and intervention drain only at `Context::run()` boundaries
- actual model inference currently occurs outside `Context::run()` in `react_loop()`
- therefore budget guards, compaction checks, intervention, and telemetry are not governing the actual inference boundary honestly

Required design change:
- introduce first-class runtime ops for inference, e.g. `Infer` and `StreamInfer`
- route model invocation through `ctx.run(...)`
- attach budget guards and other pre-inference controls to the real inference op boundary

This is required for honest composability of:
- D2E context budget
- D3A model routing
- D4C backfill policy
- D5 exit control
- L2 compaction timing
- C5 observation/intervention

### 5. Separate hints from coordination

Keep in Layer 0 only semantic hints that travel with already-stable data.

Example:
- `CompactionPolicy` belongs in Layer 0 because it travels with `Message`

Do not keep coordination/event vocabularies in Layer 0 unless they are already real shared contracts between multiple implementations.

Examples to move above Layer 0:
- budget lifecycle event vocabulary
- compaction lifecycle event vocabulary
- stream event vocabulary
- intervention channel contracts

### 6. Budget governance is split by authority

#### Turn-local authority (`skg-context-engine`)
- per-turn cost limits
- turn-count limits
- per-turn duration limits
- per-turn tool/sub-dispatch limits
- enforced before inference/tool execution boundaries

#### System/workflow authority (`skg-orch-kit`)
- aggregate cost governance across turns/operators
- downgrade/halt/continue policy
- orchestration-level budget policy and reporting

This preserves single-authority semantics:
- local runtime protects local boundaries
- orchestration governs aggregate policy

### 7. Compaction is a runtime/orchestration mechanism, not a protocol mechanism

Keep in runtime/orchestration layers:
- `CompactionRule`
- `sliding_window`
- `policy_trim`
- summarization and extraction helpers
- pre-compaction flush coordination
- compaction outcome reporting

Compaction must become a full lifecycle mechanism above Layer 0:
- explicit strategy selection
- explicit pre-flush policy
- explicit failure behavior
- no pretending that Layer 0 lifecycle vocab means the behavior exists

### 8. Observation/intervention stay above Layer 0

Intervention and streaming observation are correct as construction-time side channels on `skg-context-engine::Context`.

Needed next:
- orchestration-friendly adapters in `skg-orch-kit`
- `ObserveTool` / `InterveneTool`
- supervisor/oracle composition patterns

Layer 0 should not absorb these mechanics.

### 9. Primary validator: supervising multi-agent system

Build a new real system in `golden/projects/skelegent/` that validates the architecture.

This system should include at minimum:
- a worker operator
- a supervisor operator
- composition/orchestration glue

It must exercise:
- C1 child context policy
- C2 result return policy
- C5 observation
- live intervention
- D3A model routing
- D2E/L4 budget governance split
- L2 compaction under pressure
- L5 observability

This validator is preferred over sweep as the first proof system because it stresses the uncertain parts of the architecture more directly: observation, intervention, child context, result routing, and true multi-agent governance.

## Current Architecture Gaps

### Gap 1: No real pre-inference governance boundary
Rules do not currently govern the model call itself.

### Gap 2: Layer 0 still contains non-protocol lifecycle vocabulary
These types are not yet earning stable-contract status.

### Gap 3: No coherent decision-surface API
The codebase has many of the pieces, but not yet a normalized split between:
- turn-local knobs
- orchestration knobs
- backend knobs

### Gap 4: Observation/intervention primitives exist, but composition adapters are missing
The raw context-engine channels exist, but real tools and orchestration-friendly attachment patterns do not.

### Gap 5: Compaction is implemented, but lifecycle coordination is not
Strategies exist, but pre-flush and coordination are still mostly documentary.

## Validation Strategy

After primitive cleanup and runtime-boundary fixes, validate the architecture with one serious system in `golden/projects/skelegent/` before freezing more protocol surface.

The validator should prove that all required variability can be expressed without expanding Layer 0.

## Non-Goals

This design does not attempt to:
- fully implement all production backends immediately
- move all heavy implementation work into `extras` before validating core primitives
- preserve speculative Layer 0 vocabulary just because it may become useful later

## Summary

The path forward is:
1. keep Layer 0 ruthlessly protocol-only
2. move lifecycle coordination above Layer 0
3. make inference a real governed runtime boundary
4. expose the 23 decisions through runtime/orchestration knobs rather than protocol bloat
5. validate the architecture with a real supervising multi-agent system in `golden/projects/skelegent/`
