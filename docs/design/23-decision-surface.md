# 23-Decision Surface Map

Status: current inventory for Task 7 (2026-03-12)

This document normalizes where Skelegent answers each of the 23 golden architectural decisions today.

It is an inventory, not a proposal to grow Layer 0.

Normalization rule:
- Layer 0 carries only stable nouns and transport-safe data.
- Turn-local knobs live in operator/runtime crates.
- Composition and lifecycle policy lives in orchestration glue or application code.
- Backend-specific behavior stays inside backend crates.

The high-level `skelegent::agent()` builder is intentionally not the full 23-decision API. It exposes only slim defaults. The rest of the decision surface lives lower in the stack.

## How to read the map

- **Layer 0 noun**: the stable protocol type or boundary, if one exists.
- **Turn-local knob**: operator/runtime configuration or context-engine primitive.
- **Orchestration knob**: composition wiring, dispatcher/environment selection, or policy the caller owns.
- **Backend implementation point**: the crate or file family where the behavior is actually realized.
- **Status**: `implemented`, `partial`, or `pending`, with the reason.

## Turn decisions

| ID | Decision | Layer 0 noun | Turn-local knob | Orchestration knob | Backend implementation point | Status / Task 8 note |
|---|---|---|---|---|---|---|
| D1 | Trigger | `TriggerType`, `OperatorInput.trigger` | None. The turn runtime accepts the trigger uniformly. | Caller/orchestrator chooses the trigger when constructing `OperatorInput`. | Operator entrypoints; dispatcher/application ingress. | Implemented. No extra knob needed. |
| D2A | Identity | `OperatorConfig.system_addendum` | `TurnConfig.system_prompt`, `SingleShotConfig.system_prompt`, `AgentBuilder::system()` | Parent may set `OperatorInput.config.system_addendum` and/or curate `OperatorInput.context`. | `turn/skg-turn`, `op/skg-context-engine`, `skelegent/src/agent.rs` | Implemented. Base identity remains above Layer 0 by design. |
| D2B | History | `OperatorInput.session`, `OperatorInput.context` | `SaveConversation`, `LoadConversation`, `InjectFromStore`, `InjectSearchResults` | Orchestrator chooses session boundaries and whether to seed child context directly. | `op/skg-context-engine/src/ops/store.rs`, `state/skg-state-*` | Implemented. Existing seams are enough for validator resume/inherit cases. |
| D2C | Memory | `StateStore`, `Scope`, `StoreOptions` hints | `InjectFromStore`, `InjectSearchResults`, `fetch_search_results()` | Orchestrator/application chooses store backend, scope layout, and which memory tier to load. | `state/skg-state-*`, context-engine store ops | Partial. Three-tier memory is architectural, but there is no single shared tier-policy config object. That is acceptable until a second real consumer appears. |
| D2D | Tools | `ToolMetadata`, `Dispatcher`, `OperatorConfig.allowed_operators`, `SubDispatchRecord` | `ToolRegistry`, `ToolFilter`, `AgentBuilder::tools()` | Orchestrator decides which operators/tools are registered and discoverable. | `turn/skg-tool`, `turn/skg-mcp`, dispatcher implementations | Implemented. Tool surface is already explicit without Layer 0 growth. |
| D2E | Context budget | `OperatorConfig.max_turns`, `OperatorConfig.max_cost`, `OperatorConfig.max_duration`, `CompactionPolicy` | `BudgetGuardConfig`, `ReactLoopConfig.max_tokens`, compaction strategy configs | Orchestrator chooses reserve/compaction policy and whether to continue-as-new. | `op/skg-context-engine/src/rules/{budget,compaction}.rs`, `orch/skg-orch-kit/src/compaction.rs` | Partial. Local budget knobs exist; global policy remains orchestration-local, which is the right split for now. |
| D3A | Model selection | `OperatorConfig.model` | `TurnConfig.default_model`, `SingleShotConfig.default_model`, `AgentBuilder` model string, `ReactLoopConfig.model` | Parent/orchestrator may override per dispatch or route to different providers/operators. | `provider/skg-provider-*`, `provider/skg-provider-router`, `turn/skg-turn` | Implemented. No extra shared enum needed. |
| D3B | Durability | `Dispatcher` plus replay-safe effect declaration boundary | None in the turn crate beyond cooperating with the dispatch contract. | Orchestrator selects local vs environment-backed vs future durable dispatcher. | `orch/skg-orch-local`, `orch/skg-orch-env`, future durable orch crates | Partial. Local and environment-backed dispatch exist; durable replay backends are still pending. |
| D3C | Retry | `OperatorError::Retryable`, `OperatorError::NonRetryable` | Provider/runtime maps failures into retryable vs non-retryable classes. | Orchestrator owns the actual retry loop and retry budget. | `layer0/src/error.rs`, `skelegent/src/agent.rs`, `op/skg-op-single-shot`, future durable orch | Partial. Error classification exists; framework-level retry policy remains backend/application work. |
| D4A | Isolation | `Environment`, `EnvironmentSpec`, `IsolationBoundary` | None. The turn does not know its isolation level. | Environment binding per operator or per dispatcher registration. | `orch/skg-orch-env`, `env/skg-env-local`, `env/skg-env-docker` | Implemented. This is already explicit above Layer 0. |
| D4B | Credentials | `EnvironmentSpec.credentials`, `CredentialRef`, `CredentialInjection` | None. | Orchestrator/environment binding chooses which credentials are injected and how. | `layer0/src/environment.rs`, `env/skg-env-docker`, `skg-secret` | Implemented. Boundary injection is explicit already. |
| D4C | Backfill | `OperatorInput.context`, `OperatorOutput.message` | Tool/result formatting in the turn runtime (`format_tool_result()`, response append ops, structured-output helpers) | Parent/orchestrator decides raw inject vs summary vs two-path when feeding child or tool results back into context. | `op/skg-context-engine/src/{react.rs,ops/tool.rs,ops/response.rs}` | Partial. The seam is real; the policy is intentionally application/orchestration code, not a framework enum. |
| D5 | Exit | `ExitReason` | `BudgetGuardConfig`, structured-output retry/approval loops, runtime exit checks | Orchestrator decides what to do with the exit: retry, stop, escalate, handoff, or persist. | `op/skg-context-engine/src/{react.rs,stream_react.rs}`, middleware stacks | Implemented. Exit reasons are explicit and already usable by validator code. |

## Composition decisions

| ID | Decision | Layer 0 noun | Turn-local knob | Orchestration knob | Backend implementation point | Status / Task 8 note |
|---|---|---|---|---|---|---|
| C1 | Child context | `OperatorInput.context` | Summary/compaction primitives can produce the material passed onward, but there is no turn-owned policy type. | Parent/orchestrator explicitly chooses task-only, summary injection, or fuller inheritance when building child input. | Application composition code; `docs/architecture/orchestrator-composition-guide.md` | Implemented as explicit wiring. No `ChildContextPolicy` enum added because Task 8 can express this directly with `OperatorInput.context`. |
| C2 | Result return | `Dispatcher::dispatch -> OperatorOutput`, `Effect::WriteMemory` | None. | Parent/orchestrator chooses direct inject, summary inject, two-path, or shared-state handoff. | `orch/skg-orch-kit/src/runner.rs`, local effect interpreter, application composition code | Implemented as explicit wiring. No reusable `ResultRoutingPolicy` type is required yet. |
| C3 | Lifecycle | `Effect::Delegate`, `Effect::Handoff`, `SessionId`, `WorkflowId` | None. | Orchestrator decides ephemeral follow-up vs handoff vs long-lived/durable worker. | `orch/skg-orch-kit/src/runner.rs`, future durable orchestrators | Partial. Ephemeral/local composition exists; long-lived durable supervision remains backend work. |
| C4 | Communication | `Dispatcher`, `Effect::Signal`, `StateStore` | `Context::with_stream()`, `Context::with_intervention()` on a live worker context | Orchestrator decides sync dispatch, signals, shared state, and channel wiring. | `orch/skg-orch-local`, `orch/skg-orch-env`, `orch/skg-orch-kit`, state backends | Implemented. Current seams are enough for supervisor/worker communication tests. |
| C5 | Observation | Middleware boundaries (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`), context stream | Live context stream + intervention channel attachment on `Context` | `ContextObserver`, `ContextIntervenor`, middleware stack assembly, supervisor/oracle code | `orch/skg-orch-kit/src/{observe.rs,intervene.rs}`, `op/skg-context-engine/src/context.rs`, security middleware | Implemented. This is the most important existing seam for Task 8. |

## Lifecycle decisions

| ID | Decision | Layer 0 noun | Turn-local knob | Orchestration knob | Backend implementation point | Status / Task 8 note |
|---|---|---|---|---|---|---|
| L1 | Memory writes | `Effect::WriteMemory`, `StoreOptions` | `FlushToStore`, `SaveConversation`, `MemoryNote`, other explicit persistence ops | Orchestrator decides flush timing and attaches store middleware / effect interpreter policy. | `op/skg-context-engine/src/ops`, `orch/skg-orch-kit/src/runner.rs`, state backends | Partial. The write seam exists; policy for when to flush remains orchestration-owned. |
| L2 | Compaction | `CompactionPolicy` on `MessageMeta` | `CompactionRule`, `sliding_window`, `policy_trim`, `SummarizeConfig`, `ExtractCognitiveStateConfig` | `CompactionSnapshot`, `CompactionCoordinator`, flush-before-compact sequencing | `op/skg-context-engine/src/rules/compaction.rs`, `orch/skg-orch-kit/src/compaction.rs` | Partial. Primitives exist; validator should prove real coordination, not require new Layer 0 types. |
| L3 | Crash recovery | None beyond stable dispatch/effect contracts | None. | Durable orchestrator chooses checkpoint/replay strategy. | Current: `orch/skg-orch-local` = none. Future durable orch backend = replay/checkpoint. | Pending. This is a backend gap, not a missing protocol noun. |
| L4 | Budget governance | `OperatorConfig.max_turns`, `OperatorConfig.max_cost`, `OperatorConfig.max_duration`, budget-related `ExitReason`s | `BudgetGuardConfig`, local runtime limits | `BudgetSnapshot` and app-owned global budget policy/routing decisions | `op/skg-context-engine/src/rules/budget.rs`, `orch/skg-orch-kit/src/budget.rs` | Partial. Task 8 can split local vs global budget using existing local guard + orchestration snapshot. |
| L5 | Observability | Middleware traits, context event stream | Context event stream on `Context` | `ContextObserver`, `ExecutionTrace`, middleware selection, durable audit subscribers | `orch/skg-orch-kit/src/{observe.rs,runner.rs}`, middleware crates, context stream | Implemented. Enough surface exists to observe worker execution without Layer 0 expansion. |

## Immediate conclusions

### No new shared code knobs are required for Task 8

The upcoming supervising-validator task already has the key seams it needs:

- **C1 child context**: use `OperatorInput.context` explicitly.
- **C2 result routing**: use `Dispatcher::dispatch` for direct return and `Effect::WriteMemory` for audit/two-path routing.
- **C5 observation/intervention**: use `ContextObserver` and `ContextIntervenor`.
- **L2 compaction coordination**: use `CompactionSnapshot` + `CompactionCoordinator`.
- **L4 split budget governance**: use local `BudgetGuardConfig` plus orchestration-local `BudgetSnapshot`.

### What is intentionally still policy-in-code

The following are explicit by construction but are not promoted to shared framework enums yet:

- child-context policy
- result-routing policy
- global retry policy
- durable crash-recovery policy
- multi-agent lifecycle policy

That is deliberate. Today each has one real consumer path, and Task 8 can express them directly in supervising system code. Promoting them now would create speculative API surface.

### When to add a reusable knob later

Only add a reusable shared type above Layer 0 if all three are true:

1. Task 8 exposes a real mismatch between two callers that should share the same policy surface.
2. The policy cannot be expressed cleanly with existing `OperatorInput`, `OperatorOutput`, effect, observer, or orchestration-kit primitives.
3. The new type belongs above Layer 0 and has at least two defended consumers.
