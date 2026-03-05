# Operator (Turn) Runtime

## Purpose

The operator runtime is where the agent "thinks and acts." It is the inner loop.

In Neuron, this is implemented by crates like `neuron-op-react` and `neuron-op-single-shot` using provider implementations and tool/context infrastructure.

## Required Capabilities

Core capabilities expected from turn/operator implementations:

- accept `OperatorInput` with triggers/config/metadata
- assemble context using a `StateReader` (read-only state)
- call a provider model
- execute tools
- emit `OperatorOutput` with:
  - message content
  - exit reason
  - metadata (tokens, cost, turns, timing)
  - declared effects

## Three-Primitive Composition

Operators compose three independent, optional primitives:

```rust
let operator = ReactOperator::new(provider, config)
    .with_hooks(registry)        // observation + intervention
    .with_steering(source)       // external control flow
    .with_planner(barrier);      // execution strategy
```

Each is optional. Each composes independently. Default: no hooks, no steering, sequential planner. See `ARCHITECTURE.md` §Three-Primitive and `specs/09-hooks-lifecycle-and-governance.md` for full architectural position.

## Exit Reasons

Exit reasons are explicit and stable. Orchestrators use them to decide what happens next (retry, downgrade, halt, etc.).

### ExitReason Enum

| Variant | Trigger | HTTP side | Retriable? |
|---|---|---|---|
| `Complete` | Model returns no tool calls (natural end) | Provider HTTP 200, `EndTurn` | No |
| `MaxTurns` | `max_turns` counter reached | — | Yes (new turn) |
| `BudgetExhausted` | Cost limit or total tool call count (`max_tool_calls`) reached | — | No (without budget change) |
| `CircuitBreaker` | Consecutive failure counter trips | — | Possibly (with backoff) |
| `Timeout` | Wall-clock elapsed ≥ `max_duration` | — | Yes (new invocation) |
| `ObserverHalt { reason }` | ExitCheck hook returned `HookAction::Halt` | — | No |
| `Custom("stuck_detected")` | Identical consecutive tool calls exceed `max_repeat_calls` | — | No (without context change) |
| `Error` | Unrecoverable execution failure | — | Depends |

### SafetyStop

Provider safety stops are semantically distinct from all `ExitReason` variants above. They arrive as `StopReason::ContentFilter` in the provider response — an HTTP 200 from the provider, not a network or API error.

**Behavior**: `StopReason::ContentFilter` → `ExitReason::SafetyStop { reason: String }`. Semantically distinct from `Error` (not a transport or execution failure) and `Complete` (model did not finish naturally). Not retriable without modification to the context or request.

| Aspect | `SafetyStop` | `Error` | `Complete` |
|---|---|---|---|
| Provider HTTP | 200 | 4xx/5xx or execution fail | 200 |
| Model finished? | No — prevented | No — failed | Yes |
| Retriable? | No without context modification | Depends | N/A |
| Cause | Provider safety system | Execution/API failure | Natural completion |

Provider mapping: Anthropic `refusal`, OpenAI `content_filter`, Google `SAFETY` all map to `StopReason::ContentFilter` in Neuron's provider layer.

### Exit Priority Ordering

Priority is highest-first. ExitCheck hook fires before all limit checks:

1. **PreInference hook** — `ObserverHalt` if guardrail trips
2. **PostInference hook** — `ObserverHalt` if guardrail trips
3. **ExitCheck hook** — `ObserverHalt`; fires before any limit check
4. **Step limit** — `BudgetExhausted` when `total_tool_calls ≥ max_tool_calls`
5. **Loop detection** — `Custom("stuck_detected")` when identical calls exceed `max_repeat_calls`
6. **MaxTurns** — `MaxTurns`
7. **Budget** — `BudgetExhausted` when `total_cost ≥ max_cost`
8. **Timeout** — `Timeout` when `elapsed ≥ max_duration`

## Steering Observability

Steering (`SteeringSource`) is polled at defined boundaries. Hooks observe steering without owning it:

- `PreSteeringInject`: fires after drain returns messages, before they enter context. Guardrails can reject.
- `PostSteeringSkip`: fires after tools are skipped due to steering. Observers can log.

Steering poll-and-dispatch logic is extracted into a helper (`poll_steering`) shared across the ~6 polling sites in the main loop.

## Model Selection

`ReactConfig` supports an optional `model_selector` callback invoked before each inference. The selector sees the full `ProviderRequest` and returns a model override or `None` for the default. This enables task-type routing (route by complexity) without coupling model selection to provider implementation.

## Context Budget

Compaction reserve must never be zero. `ReactConfig.compaction_reserve_pct` (default 10%) ensures headroom to run compaction. Effective limit:

```
effective_limit = max_tokens * 4 * (1 - compaction_reserve_pct)
```

`compaction_reserve_pct` is validated to be in `0.01..=0.50`.

## Context Assembly

### AnnotatedMessage

`AnnotatedMessage` wraps `ProviderMessage` with per-message metadata enabling selective compaction. Defined in `turn/neuron-turn/src/context.rs`.

```rust
pub struct AnnotatedMessage {
    pub message: ProviderMessage,
    pub policy: Option<CompactionPolicy>,  // default: Normal
    pub source: Option<String>,            // e.g. "mcp:github", "user", "tool:shell"
    pub salience: Option<f64>,             // 0.0–1.0 write-time importance hint
}
```

An unannotated `ProviderMessage` wrapped via `AnnotatedMessage::from(msg)` behaves as if `policy = Normal`. Convenience constructors: `AnnotatedMessage::pinned(msg)`, `AnnotatedMessage::from_mcp(msg, server_name)`.

### CompactionPolicy

`CompactionPolicy` is defined in `layer0/src/lifecycle.rs` and attached to messages via `AnnotatedMessage`. All variants are advisory when used with strategies that don't inspect policy.

| Variant | Semantics |
|---|---|
| `Pinned` | Never compacted. For architectural decisions, constraints, user instructions. |
| `Normal` | Default. Subject to standard compaction. |
| `CompressFirst` | Compress preferentially. For verbose output, build logs. |
| `DiscardWhenDone` | Discard when the originating tool or MCP session ends. |

## Compaction Strategy

### TieredStrategy

`TieredStrategy` (`turn/neuron-turn/src/tiered.rs`) implements `ContextStrategy` using a four-zone partition. It eliminates recursive-summarization degradation: the summary is always first-generation, derived from original messages, never from a previous summary.

**Zone model**:

| Zone | Messages | Compaction action |
|---|---|---|
| Pinned | `CompactionPolicy::Pinned` | Never compacted; survive indefinitely |
| Active | Most-recent `active_zone_size` unpinned messages (default: 10) | Never compacted; always present |
| Summary | One first-generation summary of older unpinned messages | Replaced wholesale each compaction cycle |
| Noise | `DiscardWhenDone` or `CompressFirst` messages | Discarded on compaction |

**Configuration** (`TieredConfig`):

| Field | Default | Meaning |
|---|---|---|
| `max_messages` | 40 | Compaction fires when `messages.len() > max_messages` |
| `active_zone_size` | 10 | Number of most-recent unpinned messages kept as-is |

**Summariser**: Optional `Summariser` trait (`summarise(&[ProviderMessage]) -> Result<ProviderMessage, CompactionError>`). When absent, summary-candidate messages are discarded (lossy but no degradation). A real implementation wires in an LLM call.

**Failure modes**:

- *Recursive summarization degradation* (mitigated): Summarizing a summary of a summary is a lossy telephone game — critical architectural decisions, file paths, and conventions are lost after 2–3 cycles. TieredStrategy prevents this by always summarizing from original messages, never from a prior summary.
- `CompactionError::Transient` — API error during summarization; retriable.
- `CompactionError::Semantic` — Bad summary quality; not retriable with the same strategy.

### Pre-Compaction Flush

Before compaction destroys in-memory context, important state must be written to persistent storage. This is the `PreCompactionFlush` lifecycle event pattern.

Flow:
1. Context pressure detected → `CompactionEvent::ContextPressure` emitted
2. `CompactionEvent::PreCompactionFlush { agent, scope }` emitted — triggers memory flush
3. Operator/orchestrator writes important state to hot/warm memory tiers (via `Effect::WriteMemory`)
4. Compaction runs; context shrinks
5. New turns access persisted state via memory tools

This bridges compaction and persistent memory. The flush writes to persistent tiers; the compacted context reads them back via tools. Without the flush, compaction is irreversible information loss.

If the flush fails, `CompactionEvent::FlushFailed` is emitted with the scope and key that failed.

## Lifecycle Events

Lifecycle events (`layer0/src/lifecycle.rs`) are the cross-protocol coordination vocabulary. They are not a trait — each protocol emits and consumes them through whatever mechanism is appropriate.

### BudgetEvent

Emitted by operators (5 variants) and orchestrators (3 variants):

**Operator-emitted** (step and time limits):

| Variant | Emitter | Firing condition |
|---|---|---|
| `StepLimitApproaching { agent, current, max }` | Operator | Tool call count ≥ 80% of `max_tool_calls` and < `max_tool_calls` |
| `StepLimitReached { agent, total_tool_calls }` | Operator | Tool call count ≥ `max_tool_calls`; triggers `BudgetExhausted` exit |
| `LoopDetected { agent, tool_name, consecutive_count, max }` | Operator | All recent calls are identical (same name + input hash); triggers `Custom("stuck_detected")` exit |
| `TimeoutApproaching { agent, elapsed, max_duration }` | Operator | Elapsed ≥ 80% of `max_duration` and < `max_duration` |
| `TimeoutReached { agent, elapsed }` | Operator | Elapsed ≥ `max_duration`; triggers `Timeout` exit |

**Orchestrator-emitted** (cost tracking):

| Variant | Emitter | Firing condition |
|---|---|---|
| `CostIncurred { agent, cost, cumulative }` | Turn layer | After each model call |
| `BudgetWarning { workflow, spent, limit }` | Orchestrator | Workflow nearing its budget limit |
| `BudgetAction { workflow, action: BudgetDecision }` | Orchestrator | Orchestrator decides: `Continue`, `DowngradeModel`, `HaltWorkflow`, `RequestIncrease` |

BudgetEvents are delivered to an optional `BudgetEventSink` (trait in `neuron-op-react`):

```rust
pub trait BudgetEventSink: Send + Sync {
    fn on_budget_event(&self, event: BudgetEvent);
}
```

Attach via `ReactOperator::with_budget_sink(sink)`.

### CompactionEvent

Emitted by the operator to coordinate context compaction:

| Variant | Firing condition |
|---|---|
| `ContextPressure { agent, fill_percent, tokens_used, tokens_available }` | Context window filling; fill_percent is the fraction used |
| `PreCompactionFlush { agent, scope }` | Immediately before compaction runs; receiver must flush important state |
| `CompactionComplete { agent, strategy, tokens_freed }` | Compaction succeeded |
| `ProviderManaged { agent, provider, tokens_before, tokens_after, summary }` | Provider performed server-side compaction (e.g., Anthropic context compaction) |
| `CompactionFailed { agent, error, strategy }` | Compaction failed; error is human-readable; strategy identifies what was tried |
| `CompactionSkipped { agent, reason }` | Compaction conditions not met, or a hook blocked it |
| `FlushFailed { agent, scope, key, error }` | Pre-compaction memory flush failed for a specific key |
| `CompactionQuality { agent, tokens_before, tokens_after, items_preserved, items_lost }` | Post-compaction quality metrics |

CompactionEvents are delivered to an optional `CompactionEventSink` (trait in `neuron-op-react`):

```rust
pub trait CompactionEventSink: Send + Sync {
    fn on_compaction_event(&self, event: CompactionEvent);
}
```

Attach via `ReactOperator::with_compaction_sink(sink)`.

## Current Implementation Status

Implemented:
- `neuron-op-single-shot` — functional.
- `neuron-op-react` — full ReAct loop; emits effects.
- Steering integrated with boundary polling and skip semantics.
- Hook dispatch at PreInference, PostInference, PreToolUse, PostToolUse, ExitCheck, ToolExecutionUpdate.
- ExitCheck hook fires before all limit checks.
- Compaction reserve enforcement via `compaction_reserve_pct`.
- Step/loop limits (`max_tool_calls`, `max_repeat_calls`) with BudgetEvent emission.
- Model selector callback.
- `TieredStrategy` with zone-partitioned compaction.
- `AnnotatedMessage` and `CompactionPolicy` enabling per-message compaction metadata.
- `BudgetEventSink` and `CompactionEventSink` opt-in sinks on `ReactOperator`.
- `ExitReason::SafetyStop`; maps `StopReason::ContentFilter` to it.

Still required:
- Stronger documentation/examples for building custom operators.
- Explicit contracts on which effects are emitted in which situations.
