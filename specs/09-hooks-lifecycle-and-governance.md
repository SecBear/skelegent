# Middleware, Lifecycle, and Governance

## Purpose

Middleware and lifecycle vocabulary provide controlled intervention and cross-layer coordination.

This is how you enforce budget, tool policy, redaction, audit, and observability without baking those concerns into every operator.

## Middleware

Neuron uses per-boundary middleware traits defined in `layer0::middleware`:

- `DispatchMiddleware` — intercepts operator dispatch (pre/post inference, sub-dispatch, exit checks, steering)
- `StoreMiddleware` — intercepts state store operations (pre-memory-write)
- `ExecMiddleware` — intercepts effect execution

Each middleware trait follows a continuation-passing style: the middleware receives the request and a `next` function, and can inspect/modify the request, call `next`, and inspect/modify the response.

Middleware errors MUST be logged via `tracing::warn` — silent swallowing is prohibited.

### Middleware Boundaries

All middleware boundaries carry a common baseline context: `tokens_used`, `cost`, `turns_completed`,
and `elapsed`. The table lists only the fields that are *unique* to each boundary.

| Boundary | When | Key Context Fields |
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

### Middleware Stacks and Composition

Middleware is composed into stacks that determine execution order:

| Stack | Middleware Trait | Use for |
|---|---|---|
| `DispatchStack` | `DispatchMiddleware` | Operator dispatch interception: policy enforcement, safety gates, redaction, logging |
| `StoreStack` | `StoreMiddleware` | State store interception: memory write guardrails, redaction |
| `ExecStack` | `ExecMiddleware` | Effect execution interception: audit, policy |

Middleware in a stack runs in registration order. Each middleware calls `next` to continue
the chain, or returns early to short-circuit (e.g., to halt or skip an operation).

### Typed Interceptors

`neuron-context-engine` provides the `Rule` system for typed, operator-local interception
points. Rules offer a higher-level abstraction over `DispatchMiddleware` with typed access
to ReAct-specific context (tool calls, model responses, etc.). For cross-cutting concerns
that span operators, use the continuation-based middleware traits in `layer0::middleware`.

### Exit Priority

See `specs/04-operator-turn-runtime.md §Exit Priority Ordering` for the authoritative
priority table. ExitCheck middleware MUST fire before all limit checks in the operator
loop — a guardrail must not be checked after a limit has already returned.

### Middleware vs Steering

Middleware observes steering without being steering:

- `PreSteeringInject`: fires after `SteeringSource::drain()` returns messages but before they enter context. Guardrail middleware here can block malicious steering injection.
- `PostSteeringSkip`: fires after tools are skipped due to steering. Observer middleware logs what was skipped.

Steering is NOT middleware because the primitives are structurally different:

| | Middleware | Steering |
|---|---|---|
| Control flow | Continuation-passing | Poll-driven |
| Returns | Pass-through / halt / modify | Messages to inject |
| Composition | Stack (sequential chain) | Concatenate |
| Statefulness | Stateless per invocation | Buffers between polls |

### Security Middleware

`neuron-hook-security` provides two ready-made middleware implementations:

- `RedactionMiddleware` — scans content for sensitive patterns (regex or literal) and redacts matches before they reach the model or output sink.
- `ExfilGuardMiddleware` — inspects tool results and model responses for data-loss-prevention (DLP) signals; configurable block-or-alert policy. Detects exfiltration in any tool input via generic URL+sensitive-data patterns, shell-specific patterns, and base64 blobs.

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
| `CompactionSkipped` | Turn/Orchestrator | When compaction conditions are not met or middleware blocked it; carries reason |
| `FlushFailed` | Turn/Orchestrator | When a pre-compaction memory flush fails; carries scope, key, and error |
| `CompactionQuality` | Turn/Orchestrator | After compaction, reports quality metrics: tokens before/after, items preserved/lost |

## Current Implementation Status

- Middleware traits exist in layer0 (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`).
- `DispatchStack`, `StoreStack`, `ExecStack` compose middleware in registration order.
- All nine middleware boundaries — including `PreSteeringInject`, `PostSteeringSkip`, and `PreMemoryWrite` — are in layer0.
- Middleware error logging via `tracing::warn` is implemented.
- Security middleware exists in `neuron-hook-security`: `RedactionMiddleware` and `ExfilGuardMiddleware`.
- The `Rule` system in `neuron-context-engine` provides typed, operator-local interception.

Still required for "core complete":

- Explicit examples showing how orchestration consumes lifecycle vocab to coordinate compaction/budget
- Tests for edge middleware actions (skip tool, modify tool input/output) across the operator runtime

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
