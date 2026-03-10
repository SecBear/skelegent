# Operator (Turn) Runtime

## Purpose

The operator runtime is where the agent "thinks and acts." It is the inner loop.

In Skelegent, this is implemented by crates like `skg-context-engine` and `skg-op-single-shot` using provider implementations and tool/context infrastructure.

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

Operators compose three independent primitives:

```rust
let mut ctx = Context::new();
ctx.inject_message(Message::new(Role::User, input.message)).await?;
let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config).await?;
```

Middleware stacks, operators registered with `ToolMetadata` (`tools`), context strategy, and state reader are required constructor parameters. Steering and planner are optional builder methods; the planner type is `DispatchPlanner` (renamed from `ToolExecutionPlanner`). Default: no steering, sequential planner. See `ARCHITECTURE.md` §Three-Primitive and `specs/09-hooks-lifecycle-and-governance.md` for full architectural position.

## Exit Reasons

Exit reasons are explicit and stable. Orchestrators use them to decide what happens next (retry, downgrade, halt, etc.).

### ExitReason Enum

| Variant | Trigger | HTTP side | Retriable? |
|---|---|---|---|
| `Complete` | Model returns no tool calls (natural end) | Provider HTTP 200, `EndTurn` | No |
| `MaxTurns` | `max_turns` counter reached | — | Yes (new turn) |
| `BudgetExhausted` | Cost limit or tool-call count (`max_tool_calls`) reached | — | No (without budget change) |
| `CircuitBreaker` | Consecutive failure counter trips | — | Possibly (with backoff) |
| `Timeout` | Wall-clock elapsed ≥ `max_duration` | — | Yes (new invocation) |
| `InterceptorHalt { reason }` | Middleware/rule halted execution | — | No |
| `AwaitingApproval` | Tool calls require human approval before execution | — | Yes (after approval) |
| `Error` | Unrecoverable execution failure | — | Depends |
| `Custom(String)` | Future/domain-specific exit reasons | — | Depends |

### SafetyStop

Provider safety stops are semantically distinct from all `ExitReason` variants above. They arrive as `StopReason::ContentFilter` in the provider response — an HTTP 200 from the provider, not a network or API error.

**Behavior**: `StopReason::ContentFilter` → `ExitReason::SafetyStop { reason: String }`. Semantically distinct from `Error` (not a transport or execution failure) and `Complete` (model did not finish naturally). Not retriable without modification to the context or request.

| Aspect | `SafetyStop` | `Error` | `Complete` |
|---|---|---|---|
| Provider HTTP | 200 | 4xx/5xx or execution fail | 200 |
| Model finished? | No — prevented | No — failed | Yes |
| Retriable? | No without context modification | Depends | N/A |
| Cause | Provider safety system | Execution/API failure | Natural completion |

Provider mapping: Anthropic `refusal`, OpenAI `content_filter`, Google `SAFETY` all map to `StopReason::ContentFilter` in Skelegent's provider layer.

### Exit Priority Ordering

Priority is highest-first:

1. Rule/interceptor halts — `InterceptorHalt`
2. Tool approval required — `AwaitingApproval`
3. Turn limit — `MaxTurns`
4. Cost budget / tool-call limit — `BudgetExhausted`
5. Timeout — `Timeout`

Orchestrators that need to distinguish cost exhaustion from tool-call exhaustion should inspect the `BudgetEvent` lifecycle events.

## Steering Observability

Steering (`SteeringSource` in `skg-turn-kit`) is an optional source of mid-loop control messages. The `SteeringSource` trait provides a `drain()` method called between turns to inject new instructions or skip tool execution.

## Model Selection

`ReactLoopConfig` holds the system prompt, model, max tokens, and temperature. Model selection can be overridden per-invocation via `OperatorConfig.model`. For task-type routing (route by complexity), callers set the model in the `OperatorInput.config` field.

## Context Budget

Context budget management is handled by the `BudgetGuard` rule in `skg-context-engine`. The guard checks four limits before each inference call:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `max_cost` | `Option<Decimal>` | `None` | Maximum cost in USD |
| `max_turns` | `Option<u32>` | `Some(10)` | Maximum inference turns |
| `max_duration` | `Option<Duration>` | `None` | Maximum wall-clock duration |
| `max_tool_calls` | `Option<u32>` | `None` | Maximum total tool calls |

When any limit is exceeded, `BudgetGuard` returns `EngineError::Halted`, which the react loop maps to `ExitReason::BudgetExhausted`.

## Context Assembly

### Message

`Message` (from `layer0::context`) wraps `ProviderMessage` with per-message metadata enabling selective compaction. Defined in `layer0/src/context.rs`.

```rust
pub struct Message {
    pub message: ProviderMessage,
    pub policy: Option<CompactionPolicy>,  // default: Normal
    pub source: Option<String>,            // e.g. "mcp:github", "user", "tool:shell"
    pub salience: Option<f64>,             // 0.0–1.0 write-time importance hint
}
```

An unannotated `ProviderMessage` wrapped via `Message::from(msg)` behaves as if `policy = Normal`. Convenience constructors: `Message::pinned(msg)`, `Message::from_mcp(msg, server_name)`.

### CompactionPolicy

`CompactionPolicy` is defined in `layer0/src/lifecycle.rs` and attached to messages via `Message`. All variants are advisory when used with strategies that don't inspect policy.

| Variant | Semantics |
|---|---|
| `Pinned` | Never compacted. For architectural decisions, constraints, user instructions. |
| `Normal` | Default. Subject to standard compaction. |
| `CompressFirst` | Compress preferentially. For verbose output, build logs. |
| `DiscardWhenDone` | Discard when the originating tool or MCP session ends. |

## Compaction Strategy

Compaction in `skg-context-engine` is handled via two mechanisms:

### Rule-based compaction

`CompactionRule` fires as a `When` trigger, checking message count against a threshold. When triggered, it runs one of two built-in strategies:

- **`sliding_window(n)`** — keeps the last `n` messages, respecting `CompactionPolicy::Pinned`
- **`policy_trim`** — removes `DiscardWhenDone` and `CompressFirst` messages first, then oldest `Normal` messages

### Async LLM-driven strategies

Standalone async functions (not `ContextOp`s) called explicitly between turns:

- **`summarize` / `summarize_with`** — summarizes messages via a provider; returns a `Pinned` summary message
- **`extract_cognitive_state` / `extract_cognitive_state_with`** — extracts structured state from context via a provider; returns JSON

These compose freely with `FlushToStore` (persist extracted state) and `InjectFromStore` (retrieve persisted state into context).

### Store integration ops

- **`FlushToStore`** — runs an extractor closure over context, writes results to a `StateStore`
- **`InjectFromStore`** — searches a `StateStore`, injects results as system/user messages at configurable positions

See `op/skg-context-engine/DESIGN.md` for the full composition pattern.

### Pre-Compaction Flush

Pre-compaction flush is mandatory. Before compaction destroys in-memory context, important state MUST be written to persistent storage. This is the `PreCompactionFlush` lifecycle event pattern.

Flow:
1. Context pressure detected → `CompactionEvent::ContextPressure` emitted
2. `CompactionEvent::PreCompactionFlush { operator, scope }` emitted — triggers memory flush
3. Operator/orchestrator writes important state to hot/warm memory tiers (via `Effect::WriteMemory`)
4. Compaction runs; context shrinks
5. New turns access persisted state via memory tools

This bridges compaction and persistent memory. The flush writes to persistent tiers; the compacted context reads them back via tools. Without the flush, compaction is irreversible information loss.

If the flush fails, `CompactionEvent::FlushFailed` is emitted with the scope and key that failed.

## Lifecycle Events

Operators emit `BudgetEvent` and `CompactionEvent` lifecycle events. See `specs/09-hooks-lifecycle-and-governance.md` for the full vocabulary and semantics.

Budget and compaction events are handled via the Rule system in `skg-context-engine`. Attach a `BudgetGuard` rule to the `Context` before calling `react_loop()`.

## Current Implementation Status

Implemented:
- `skg-op-single-shot` — functional.
- `skg-context-engine` — full ReAct loop with streaming; emits effects.
- `react_loop()` and `react_loop_structured()` for regular and structured output.
- `stream_react_loop()` for streaming responses.
- Steering integrated via `SteeringSource` trait.
- Three middleware stacks at protocol boundaries: `DispatchMiddleware`, `ExecMiddleware`, `StoreMiddleware`.
- `BudgetGuard` rule with cost, turn, duration, and tool-call limits.
- `CompactionRule` with `sliding_window` and `policy_trim` strategies.
- Async `summarize` and `extract_cognitive_state` functions.
- `FlushToStore` and `InjectFromStore` context ops with configurable injection position.
- `TelemetryRule` for turn-level metrics emission.
- Model selection via `ReactLoopConfig.model` and per-request `InferRequest::with_model()`.
- `Message` and `CompactionPolicy` enabling per-message compaction metadata.
- `ExitReason::SafetyStop`; maps `StopReason::ContentFilter` to it.
- `ExitReason::AwaitingApproval` and `Effect::ToolApprovalRequired` for human-in-the-loop.
- Dynamic tool availability via `ToolFilter` callback.

Still required:
- Stronger documentation/examples for building custom operators.
- Explicit contracts on which effects are emitted in which situations.