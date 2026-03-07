> **SUPERSEDED:** The Hook system described here is being replaced by per-boundary
> continuation-based middleware (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`).
> See `docs/plans/MIDDLEWARE-REDESIGN-BRIEFING.md` for the new design.
> This spec remains for reference until migration is complete.

# Hooks, Lifecycle, and Governance

## Purpose

Hooks and lifecycle vocabulary provide controlled intervention and cross-layer coordination.

This is how you enforce budget, tool policy, redaction, audit, and observability without baking those concerns into every operator.

## Hooks

`layer0::Hook` defines:

- hook points (pre/post inference, sub-dispatch, exit checks, steering, memory writes)
- actions (continue, halt, skip tool, modify input/output)

Hook errors should not implicitly halt execution; a hook must explicitly choose `Halt`.
Hook errors MUST be logged via `tracing::warn` — silent swallowing is prohibited.

### Hook Points

All hook points carry a common baseline context: `tokens_used`, `cost`, `turns_completed`,
and `elapsed`. The table lists only the fields that are *unique* to each point.

| HookPoint | When | Key Context Fields |
|---|---|---|
| `PreInference` | Before each model call | *(baseline only)* |
| `PostInference` | After model responds, before sub-operator dispatch | `model_output` |
| `PreSubDispatch` | Before each tool executes | `operator_name`, `operator_input` |
| `PostSubDispatch` | After tool completes, before result enters context | `operator_name`, `operator_result` |
| `ExitCheck` | At each exit-condition check | *(baseline only)* |
| `SubDispatchUpdate` | Streaming chunk available | `operator_name`, `operator_chunk` |
| `PreSteeringInject` | After steering drain, before messages enter context | `steering_messages` |
| `PostSteeringSkip` | After tools skipped due to steering | `skipped_operators` |
| `PreMemoryWrite` | Before WriteMemory effect executes | `memory_key`, `memory_value`, `memory_options` |

### Hook Kinds and Composition

Hooks are registered with a `HookKind` that determines how multiple hooks at the same point compose:

| HookKind | Composition | Use for |
|---|---|---|
| `Guardrail` | Sequential, short-circuits on Halt/Skip | Policy enforcement, safety gates |
| `Transformer` | Sequential chain — each feeds modified context to next | Redaction, formatting, sanitization |
| `Observer` | All run, actions ignored | Logging, telemetry, audit |

#### Dispatch order

At each hook point, the registry runs three phases in this order:

1. **Observers** — All run regardless of what any observer returns. Actions are
   discarded. Errors are logged via `tracing::warn` and execution continues.
   Observers cannot affect the pipeline.

2. **Transformers** — Run in registration order. Each transformer receives the
   context as *modified by the previous transformer* (chaining). Accumulated
   `ModifyDispatchInput`/`ModifyDispatchOutput` actions are applied to `working_ctx` so
   the next transformer sees them. A `Halt` from any transformer escalates
   immediately and short-circuits the entire pipeline (no guardrails run).
   Errors are logged and treated as `Continue`.

3. **Guardrails** — Run in registration order against the **original, unmodified**
   context (not the transformer-modified working context). Policy must be enforced
   against what actually arrived, not what transformers produced. Short-circuit on
   the first `Halt` or `SkipDispatch`. Errors are logged and execution continues to
   the next guardrail.

If no phase produced a `Halt` or `SkipDispatch`, the last transformer modification
(if any) is returned; otherwise `Continue` is returned.

`HookKind` lives in `neuron-hooks` (Layer 1), NOT in `layer0`. The `Hook` trait
in Layer 0 does not know its kind — kind is a registration-time property of the
registry, not the hook itself. This preserves Layer 0 stability.

### Exit Priority

See `specs/04-operator-turn-runtime.md §Exit Priority Ordering` for the authoritative
priority table. ExitCheck hooks MUST dispatch before all limit checks in the operator
loop — a guardrail must not be checked after a limit has already returned.

### Hooks vs Steering

Hooks observe steering without being steering:

- `PreSteeringInject`: fires after `SteeringSource::drain()` returns messages but before they enter context. A guardrail here can block malicious steering injection.
- `PostSteeringSkip`: fires after tools are skipped due to steering. Observers log what was skipped.

Steering is NOT a HookKind because the primitives are structurally different:

| | Hooks | Steering |
|---|---|---|
| Control flow | Event-driven | Poll-driven |
| Returns | Actions (Halt/Skip/Modify) | Messages to inject |
| Composition | By kind (short-circuit/chain/parallel) | Concatenate |
| Statefulness | Stateless per invocation | Buffers between polls |

## Lifecycle Vocabulary

Lifecycle types define coordination events. These are shared vocabulary types,
not a separate lifecycle service — the orchestrator listens, applies policies,
and takes action.

### BudgetEvent

| Variant | Emitted by | When |
|---|---|---|
| `CostIncurred` | Turn | After each model inference call; carries per-call and cumulative cost |
| `BudgetWarning` | Orchestrator | When a workflow's cumulative spend nears its configured limit |
| `BudgetAction` | Orchestrator | When the orchestrator decides how to respond to budget pressure (continue / downgrade model / halt / request increase) |
| `StepLimitApproaching` | Operator | When sub-dispatch count approaches the configured `max_sub_dispatches` limit |
| `StepLimitReached` | Operator | When the step (sub-dispatch) limit is reached and the operator must exit |
| `LoopDetected` | Operator | When identical consecutive sub-dispatches exceed the configured loop detection threshold |
| `TimeoutApproaching` | Operator | When elapsed time approaches the configured `max_duration` |
| `TimeoutReached` | Operator | When the elapsed time limit is reached and the operator must exit |


> **Loop detection is a two-part mechanism:** the operator emits
> `BudgetEvent::LoopDetected` to sinks (observability notification) **and** returns
> `ExitReason::Custom("stuck_detected")` (control-flow exit). These are complementary:
> the event is for observability and audit; the exit reason is for orchestrators deciding
> what to do next. Similarly, step limit (`max_sub_dispatches`) emits
> `BudgetEvent::StepLimitReached` and returns `ExitReason::BudgetExhausted`.

### Budget Governance Authority

Budget decisions have a single authority chain (from `ARCHITECTURE.md §Lifecycle`):

- **Turn** emits `CostIncurred` after each model inference call.
- **Orchestrator** tracks aggregate cost and emits `BudgetWarning` / `BudgetAction`.
- **Lifecycle coordinator** (orchestrator role) makes halt/continue/downgrade decisions.
- **Planners** observe remaining budget read-only — they MUST NOT make halt decisions.

Single authority is required: if the SDK also applies automatic retry on budget exhaustion,
it conflicts with the orchestrator's halt decision. SDK-level automatic retry MUST be
disabled when the orchestrator manages budget governance.

### CompactionEvent

| Variant | Emitted by | When |
|---|---|---|
| `ContextPressure` | Turn | When context window fill percentage crosses a threshold; carries fill_percent, tokens_used, tokens_available |
| `PreCompactionFlush` | Turn/Orchestrator | Before compaction begins, to trigger a memory flush of the given scope |
| `CompactionComplete` | Turn/Orchestrator | After compaction finishes successfully; carries strategy used and tokens freed |
| `ProviderManaged` | Turn | When the model provider (e.g. Anthropic server-side compaction) compacted the context; carries tokens before/after and optional summary |
| `CompactionFailed` | Turn/Orchestrator | When compaction fails with an error; carries strategy and error description |
| `CompactionSkipped` | Turn/Orchestrator | When compaction conditions are not met or a hook blocked it; carries reason |
| `FlushFailed` | Turn/Orchestrator | When a pre-compaction memory flush fails; carries scope, key, and error |
| `CompactionQuality` | Turn/Orchestrator | After compaction, reports quality metrics: tokens before/after, items preserved/lost |

## Current Implementation Status

- Hook traits exist in layer0.
- HookKind-aware three-phase dispatch (Observer → Transformer → Guardrail) is implemented in `neuron-hooks`.
- All nine hook points — including `PreSteeringInject`, `PostSteeringSkip`, and `PreMemoryWrite` — are in layer0 and tested.
- Hook error logging via `tracing::warn` is implemented in `neuron-hooks` dispatch.
- Policy/security hooks exist in `neuron-hook-security`; `ExfilGuardHook` detects exfiltration in any tool input via generic URL+sensitive-data patterns, shell-specific patterns, and base64 blobs.

Still required for "core complete":

- Explicit examples showing how orchestration consumes lifecycle vocab to coordinate compaction/budget
- Tests for edge hook actions (skip tool, modify tool input/output) across the operator runtime

## Observability: Common Event Interface

All layers emit observability data through `layer0::lifecycle::ObservableEvent`. Every
event carries:

| Field | Type | Meaning |
|---|---|---|
| `source` | `EventSource` | Which protocol emitted this event |
| `event_type` | `String` | Event type (namespaced by convention, e.g. `"turn.cost_incurred"`) |
| `timestamp` | `DurationMs` | Milliseconds since workflow start |
| `trace_id` | `Option<String>` | Correlation ID for cross-layer tracing |
| `data` | `serde_json::Value` | Event payload |

All new lifecycle event types MUST be emitted via `ObservableEvent` so that tracing,
logging, and audit infrastructure has a single integration point. `BudgetEvent` and
`CompactionEvent` are domain-specific vocabularies for sink callbacks; `ObservableEvent`
is the cross-cutting emission interface.