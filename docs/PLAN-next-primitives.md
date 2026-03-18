> **ARCHIVED** — This plan was written before the EffectEmitter-to-DispatchContext
> migration. EffectEmitter references below are historical. The current Operator
> interface uses `execute(input: OperatorInput, ctx: &DispatchContext)` with effects
> declared via `Context::push_effect()` / `Context::extend_effects()`.

# Next Primitives Implementation Plan

All work targets skelegent/ and extras/. Breaking changes acceptable. DIY-first.

## Phase 1 — Dead code eradication (parallel, no dependencies)

### 1A: Kill redundant registry API
**File**: `skelegent/runner/skg-runner/src/registry.rs`
- Delete `OperatorRegistry::new()` and `OperatorRegistry::register(&mut self, ...)`
- Keep only `OperatorRegistry::builder()` + `OperatorRegistryBuilder::register(self, ...)` + `build()`
- Remove all `#[allow(dead_code)]` from this file
- Verify: `cargo check -p skg-runner`

### 1B: Remove false dead_code on InferRequest.extra
**File**: `skelegent/turn/skg-turn/src/infer.rs`
- Remove `#[allow(dead_code)]` from the `extra` field — providers read it
- Verify: `cargo check -p skg-turn`

### 1C: Kill Docker forward-planned dead code
**Files**: `skelegent/env/skg-env-docker/src/lib.rs`, `skelegent/env/skg-env-docker/src/lifecycle.rs`
- Delete `event_sink` field from `DockerEnvironment` and its construction sites
- Delete `stop_and_remove()` function (duplicated in `ContainerGuard::drop`)
- Remove associated `#[allow(dead_code)]` annotations
- Verify: `cargo check -p skg-env-docker`

### 1D: Remove struct-level dead_code on provider types
**Files**: `skelegent/provider/skg-provider-openai/src/types.rs`, `skelegent/provider/skg-provider-ollama/src/types.rs`
- Remove `#[allow(dead_code)]` from struct definitions (these derive Deserialize — serde uses all fields)
- Fields that are deserialized but never read: keep the fields, remove the allow — the compiler won't warn because serde generates code that reads them
- Verify: `cargo check -p skg-provider-openai -p skg-provider-ollama`

## Phase 2 — Protocol-level dispatch return (sequential, Layer 0 change)

### 2A: Synchronous child dispatch pattern
**File**: `skelegent/layer0/src/dispatch.rs`

Operators already hold `Arc<dyn Dispatcher>` via constructor injection. `Dispatcher::dispatch()` returns `DispatchHandle`. `DispatchHandle::collect()` returns `OperatorOutput`. The pieces exist — the gap is ergonomics and documentation.

What's needed:
- `collect()` currently discards `Progress` and `ArtifactProduced`. Add `collect_all()` that returns `(OperatorOutput, Vec<DispatchEvent>)` preserving all intermediate events
- Document the inline dispatch pattern: `let output = self.dispatcher.dispatch(&ctx, input).await?.collect().await?;`
- This is already possible today — the "gap" is that nothing documents or tests this pattern
- Add integration test in `skelegent/tests/poc.rs` demonstrating synchronous child dispatch within an operator

### 2B: Deadline propagation on DispatchContext
**File**: `skelegent/layer0/src/dispatch_context.rs`
- Add `deadline: Option<tokio::time::Instant>` to `DispatchContext`
- Add `with_deadline(instant)` and `remaining()` methods
- `DispatchContext::child()` propagates deadline from parent
- Dispatchers (LocalOrch) respect deadline via `tokio::time::timeout`
- Middleware can inspect `ctx.remaining()` for observability

### 2C: Retry as DispatchMiddleware
**File**: new `skelegent/hooks/skg-hook-retry/src/lib.rs`
- New crate `skg-hook-retry`
- `RetryMiddleware` implementing `DispatchMiddleware`
- Configurable: max_retries, backoff strategy (fixed, exponential), retryable error classification
- Wraps `next.dispatch()` calls — transparent to operators
- Uses `OrchError` classification (already has `is_retryable()` on some error types)

## Phase 3 — Streaming as first-class primitive (sequential, depends on Phase 2A)

### 3A: Fix collect() to buffer all events
**File**: `skelegent/layer0/src/dispatch.rs`
- Current `collect()` throws away Progress/Artifact — wrong default
- New behavior: `collect()` returns `CollectedDispatch { output: OperatorOutput, events: Vec<DispatchEvent> }`
- Breaking change. All callers of `collect()` updated
- Callers who only want the output: `handle.collect().await?.output`

### 3B: Wire runner HTTP streaming
**File**: `skelegent/runner/skg-runner/src/http_adapter.rs`
- `execute_stream_handler`: use `Dispatcher::dispatch()` instead of direct operator call
- Forward `DispatchEvent`s as SSE events in real-time
- Remove the TODO comment
- The runner's `main.rs` execute paths: replace `EffectEmitter::noop()` with dispatcher-based execution

### 3C: Token-level streaming from providers
**Files**: provider crates' streaming implementations
- Providers already have streaming types (OpenAI `OpenAIStreamChunk`, Anthropic streaming)
- Gap: `Provider::infer()` returns complete `InferResponse`. No streaming variant
- Add `Provider::infer_stream()` returning `impl Stream<Item = InferChunk>`
- `InferChunk` enum: `TextDelta(String)`, `ToolCallDelta { id, name, input_delta }`, `Usage(UsageStats)`, `Done(InferResponse)`
- CognitiveOperator/react_loop: use streaming when available, forward chunks via `EffectEmitter::effect(Effect::Progress(...))`

## Phase 4 — Composition primitives (parallel after Phase 2)

### 4A: MCP client
**File**: `skelegent/turn/skg-mcp/src/client.rs` (new)
- MCP client that discovers and calls external MCP tools
- Returns `Vec<Arc<dyn ToolDyn>>` — tools from external MCP servers look like local tools
- Configuration: server URI, transport (stdio, streamable-http)
- Integrates with `ToolRegistry` — user adds MCP tools alongside local tools

### 4B: Conversation persistence
**Files**: `skelegent/op/skg-context-engine/src/context.rs`, new ops
- `Context::save(store, scope)` → serializes messages + extensions to StateStore
- `Context::load(store, scope)` → restores from StateStore
- Session continuity: operator can resume conversation from last checkpoint
- Uses existing `FlushToStore`/`InjectFromStore` ops under the hood but with ergonomic API

### 4C: Operator metadata trait
**File**: `skelegent/layer0/src/operator.rs`
- Add optional `OperatorMeta` trait (separate from `Operator`)
- Methods: `name()`, `description()`, `input_schema()`, `capabilities()` (what tools/state/auth it needs)
- Used by MCP server, A2A agent card, auto-discovery
- Default impl returns empty — backwards compatible

## Phase 5 — Error enrichment (parallel with Phase 4)

### 5A: Structured ToolError
**File**: `skelegent/turn/skg-tool/src/lib.rs`
- Current: `NotFound`, `ExecutionFailed`, `InvalidInput`, `Other`
- Add: `Transient(String)` (network timeout, rate limit), `RateLimited { retry_after: Option<Duration> }`
- React loop can distinguish transient vs permanent and retry appropriately
- Breaking change on `ToolError` enum (already `#[non_exhaustive]`)

### 5B: Structured approval context
**File**: `skelegent/turn/skg-tool/src/lib.rs`
- `requires_approval()` returns `ApprovalPolicy` instead of `bool`
- `ApprovalPolicy::None`, `ApprovalPolicy::Always`, `ApprovalPolicy::When(Box<dyn Fn(&Value) -> bool>)`
- Enables: auto-approve if cost < threshold, require human if destructive
- Breaking change
