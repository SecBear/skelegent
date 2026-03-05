# Hooks, Lifecycle, and Governance

## Purpose

Hooks and lifecycle vocabulary provide controlled intervention and cross-layer coordination.

This is how you enforce budget, tool policy, redaction, audit, and observability without baking those concerns into every operator.

## Hooks

`layer0::Hook` defines:

- hook points (pre/post inference, tool use, exit checks, steering, memory writes)
- actions (continue, halt, skip tool, modify input/output)

Hook errors should not implicitly halt execution; a hook must explicitly choose `Halt`.
Hook errors MUST be logged via `tracing::warn` — silent swallowing is prohibited.

### Hook Points

All hook points carry a common baseline context: `tokens_used`, `cost`, `turns_completed`,
and `elapsed`. The table lists only the fields that are *unique* to each point.

| HookPoint | When | Key Context Fields |
|---|---|---|
| `PreInference` | Before each model call | *(baseline only)* |
| `PostInference` | After model responds, before tool execution | `model_output` |
| `PreToolUse` | Before each tool executes | `tool_name`, `tool_input` |
| `PostToolUse` | After tool completes, before result enters context | `tool_name`, `tool_result` |
| `ExitCheck` | At each exit-condition check | *(baseline only)* |
| `ToolExecutionUpdate` | Streaming chunk available | `tool_name`, `tool_chunk` |
| `PreSteeringInject` | After steering drain, before messages enter context | `steering_messages` |
| `PostSteeringSkip` | After tools skipped due to steering | `skipped_tools` |
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
   `ModifyToolInput`/`ModifyToolOutput` actions are applied to `working_ctx` so
   the next transformer sees them. A `Halt` from any transformer escalates
   immediately and short-circuits the entire pipeline (no guardrails run).
   Errors are logged and treated as `Continue`.

3. **Guardrails** — Run in registration order against the **original, unmodified**
   context (not the transformer-modified working context). Policy must be enforced
   against what actually arrived, not what transformers produced. Short-circuit on
   the first `Halt` or `SkipTool`. Errors are logged and execution continues to
   the next guardrail.

If no phase produced a `Halt` or `SkipTool`, the last transformer modification
(if any) is returned; otherwise `Continue` is returned.

`HookKind` lives in `neuron-hooks` (Layer 1), NOT in `layer0`. The `Hook` trait
in Layer 0 does not know its kind — kind is a registration-time property of the
registry, not the hook itself. This preserves Layer 0 stability.

### Exit Priority

Safety halt (hook) > budget > max turns > model done. ExitCheck hooks MUST dispatch before limit checks in the operator loop. A guardrail that should block execution must not be checked after a budget limit has already returned.

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
| `StepLimitApproaching` | Operator | When tool call count approaches the configured `max_tool_calls` limit |
| `StepLimitReached` | Operator | When the step (tool call) limit is reached and the operator must exit |
| `LoopDetected` | Operator | When identical consecutive tool calls exceed the configured loop detection threshold |
| `TimeoutApproaching` | Operator | When elapsed time approaches the configured `max_duration` |
| `TimeoutReached` | Operator | When the elapsed time limit is reached and the operator must exit |

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