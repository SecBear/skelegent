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

Layer 0 defines `MaxTurns`, `BudgetExhausted`, and `Timeout` as the structured exit vocabulary. The current `skg-context-engine` budget guard still halts with `EngineError::Halted`; callers that want structured exit reasons must wrap or extend the runtime above Layer 0 until the runtime boundary work lands.

## Steering Observability

Steering (`SteeringSource` in `skg-turn-kit`) is an optional source of mid-loop control messages. The `SteeringSource` trait provides a `drain()` method called between turns to inject new instructions or skip tool execution.

## Model Selection

`ReactLoopConfig` holds the system prompt, model, max tokens, and temperature. Model selection can be overridden per-invocation via `OperatorConfig.model`. For task-type routing (route by complexity), callers set the model in the `OperatorInput.config` field.

## Context Budget

Context budget management is currently handled by the `BudgetGuard` rule in `skg-context-engine`. The guard checks four limits at runtime governance boundaries today:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `max_cost` | `Option<Decimal>` | `None` | Maximum cost in USD |
| `max_turns` | `Option<u32>` | `Some(10)` | Maximum inference turns |
| `max_duration` | `Option<Duration>` | `None` | Maximum wall-clock duration |
| `max_tool_calls` | `Option<u32>` | `None` | Maximum total tool calls |

When any limit is exceeded, `BudgetGuard` returns `EngineError::Halted`. Honest pre-inference enforcement and canonical structured exit mapping are implementation work tracked above this spec baseline.

## Context Assembly

### Message

`Message` (from `layer0::context`) is the protocol-level conversational unit the runtime compiles into provider requests. It carries role/content plus per-message metadata that survives store, dispatch, and compaction boundaries.

```rust
pub struct Message {
    pub role: Role,
    pub content: Content,
    pub meta: MessageMeta,
}

pub struct MessageMeta {
    pub policy: CompactionPolicy,
    pub source: Option<String>,
    pub salience: Option<f64>,
    pub version: u64,
}
```

Use `Message::new(role, content)` for default metadata and `Message::pinned(role, content)` for pinned messages. Compaction/source/salience annotations live under `meta`, not as top-level `Message` fields.

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

Pre-compaction flush is mandatory. Before compaction destroys in-memory context, important state MUST be written to persistent storage.

Current implementation model: this coordination lives above Layer 0. `skg-context-engine` owns the local compaction rules, and orchestrators may add their own coordination around them. Layer 0 contributes the message-level hints that travel with the data (`Message` + `CompactionPolicy`), not a compaction event vocabulary.

Flow:
1. Context pressure is detected by runtime-local compaction logic
2. Runtime/orchestrator code flushes important state to hot/warm memory tiers (via `Effect::WriteMemory`) before destructive compaction
3. Compaction runs; context shrinks
4. New turns access persisted state via memory tools

This bridges compaction and persistent memory. The flush writes to persistent tiers; the compacted context reads them back via tools. Without the flush, compaction is irreversible information loss.

## Lifecycle Coordination

Budget and compaction coordination are currently runtime-local or orchestrator-local behaviors above Layer 0.
- Local budget enforcement is handled by `BudgetGuard` in `skg-context-engine` before each inference call.
- Compaction coordination is handled by context-engine rules and orchestration code; `CompactionPolicy` on `Message` remains the Layer 0 hint that travels with data.
- Observation and intervention mechanics live above Layer 0 via middleware, context streams, and orchestrator wiring.

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