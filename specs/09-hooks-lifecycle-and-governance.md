# Middleware, Lifecycle, and Governance

## Purpose

Middleware provides controlled intervention at protocol boundaries. Lifecycle coordination currently lives above Layer 0 unless and until it becomes a real cross-boundary contract.

This is how you enforce tool policy, redaction, audit, observability, and runtime-local budget/compaction behavior without baking those concerns into every operator.

## Middleware

Skelegent uses per-boundary middleware traits defined in `layer0::middleware`:

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

`skg-context-engine` provides the `Rule` system for typed, operator-local interception
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

`skg-hook-security` provides two ready-made middleware implementations:

- `RedactionMiddleware` — scans content for sensitive patterns (regex or literal) and redacts matches before they reach the model or output sink.
- `ExfilGuardMiddleware` — inspects tool results and model responses for data-loss-prevention (DLP) signals; configurable block-or-alert policy. Detects exfiltration in any tool input via generic URL+sensitive-data patterns, shell-specific patterns, and base64 blobs.

## Lifecycle Coordination

Current approved model: Layer 0 stays protocol-only. It carries middleware traits/stacks and message-level hints that travel with data. Budget/compaction coordination, observation/intervention mechanics, and durable run/control semantics all live in runtime or orchestration code above Layer 0 unless promoted into a real cross-boundary contract.

### Budget Coordination

Local budget enforcement is runtime-local today.
- `BudgetGuard` in `skg-context-engine` enforces cost, turn, duration, and tool-call limits at the real inference boundary (`InferBoundary`, or `StreamInferBoundary` for streaming).
- `BudgetGuard` returns structured runtime exits (`MaxTurns`, `BudgetExhausted`, `Timeout`), and `react_loop()` surfaces them as structured `OperatorOutput` exits rather than generic inference failures.
- Broader halt/continue/downgrade policy belongs to orchestrator code above Layer 0.

### Compaction Coordination

Compaction coordination is also above Layer 0 today.
- `skg-context-engine` owns the local compaction rules and summarization flow.
- `skg-orch-kit::CompactionCoordinator` is the small orchestration-local coordinator for deciding skip vs compact vs flush-then-compact and for enforcing flush-before-compaction ordering.
- `CompactionPolicy` on `Message` remains a valid Layer 0 advisory hint because it travels with the message across boundaries.

### Observation and Intervention

Observation/intervention mechanics are above Layer 0.
- Layer 0 defines middleware traits and middleware boundaries at protocol seams.
- Context streams, observer channels, and intervention channels are wired by runtime/orchestrator code, not by a Layer 0 lifecycle event contract.
- Durable wait/resume/cancel semantics are orchestration control-plane concerns above Layer 0, not middleware events; satisfying a durable wait point remains distinct from sending a signal.

## Current Implementation Status

Implemented:
- Middleware traits exist in layer0 (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`).
- `DispatchStack`, `StoreStack`, `ExecStack` compose middleware in registration order.
- All nine middleware boundaries — including `PreSteeringInject`, `PostSteeringSkip`, and `PreMemoryWrite` — are in layer0.
- Middleware error logging via `tracing::warn` is implemented.
- Security middleware exists in `skg-hook-security`: `RedactionMiddleware` and `ExfilGuardMiddleware`.
- The `Rule` system in `skg-context-engine` provides typed, operator-local interception.
- Budget enforcement is implemented locally via `BudgetGuard` in `skg-context-engine`.
- Compaction is implemented above Layer 0 via context-engine rules plus `skg-orch-kit::CompactionCoordinator`; `CompactionPolicy` remains the Layer 0 message hint.

Still required for "core complete":
- Tests for edge middleware actions (skip tool, modify tool input/output) across the operator runtime.

## Observability

Observability is currently provided by a mix of:
- protocol-boundary middleware in Layer 0 (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`)
- runtime-local tracing and context streams above Layer 0
- orchestration-local sinks/adapters above Layer 0

There is no generic Layer 0 telemetry envelope in this branch. If one becomes necessary later, add it only after multiple implementations depend on it.
