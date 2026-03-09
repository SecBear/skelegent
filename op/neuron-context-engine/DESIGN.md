# neuron-context-engine Design Specification

## Philosophy

Agentic programming has three pillars: **context**, **inference**, and **infrastructure**.
Context is the first-class value that everything operates on. Inference is the irreducible
network boundary. Infrastructure is tool dispatch, state, effects.

Every agent follows three phases: **assembly** (build context), **inference** (call model),
**reaction** (branch on response). The context engine makes each phase composable and hookable.

## Core Types

### Context

The mutable substrate. Carries messages, typed extensions, metrics, effects, and rules.
Every mutation goes through `ctx.run(op)` which fires applicable rules.

```rust
pub struct Context {
    pub messages: Vec<Message>,          // layer0::context::Message
    pub extensions: Extensions,           // typed arbitrary state (HashMap<TypeId, Box<dyn Any>>)
    pub effects: Vec<Effect>,             // layer0::effect::Effect
    pub metrics: TurnMetrics,             // tokens, cost, timing
    rules: Vec<Rule>,                     // reactive participants
}
```

### ContextOp

The universal primitive. Everything implements this.

```rust
#[async_trait]
pub trait ContextOp: Send + Sync {
    type Output: Send;
    async fn execute(&self, ctx: &mut Context) -> Result<Self::Output, EngineError>;
}
```

### Rule

A ContextOp with a trigger. Same power as pipeline ops, different activation.

```rust
pub struct Rule {
    pub name: String,
    pub trigger: Trigger,
    pub priority: i32,            // higher = fires first
    op: Box<dyn ErasedOp>,        // type-erased ContextOp<Output=()>
}

pub enum Trigger {
    BeforeAny,                    // fires before every run()
    AfterAny,                     // fires after every run()
    Before(TypeId),               // fires before a specific op type
    After(TypeId),                // fires after a specific op type
    When(Box<dyn Fn(&Context) -> bool + Send + Sync>),  // predicate-based
}
```

Rules fire in priority order (highest first). Rules cannot trigger other rules (no recursion).

### TurnMetrics

Accumulated metrics for the current operator invocation.

```rust
pub struct TurnMetrics {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: Decimal,
    pub turns_completed: u32,
    pub tool_calls_total: u32,
    pub start: Instant,
}
```

### Extensions

Typed map for arbitrary state. Hand-rolled `HashMap<TypeId, Box<dyn Any + Send + Sync>>`.

```rust
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}
```

### EngineError

```rust
pub enum EngineError {
    Halted { reason: String },         // rule or op halted execution
    Provider(ProviderError),           // inference failed
    Operator(OperatorError),           // layer0 operator error
    Tool(ToolError),                   // tool dispatch failed
    Custom(Box<dyn Error + Send + Sync>),
}
```

## Phase Boundary: compile() -> infer()

### CompiledContext

Snapshot of messages + tools for the model. Does NOT consume Context — borrows/clones.

```rust
pub struct CompiledContext {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub system: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub extra: serde_json::Value,
}
```

`Context::compile()` produces this. Rules with `Before(TypeId::of::<Infer>())` fire
during compile (they see the context about to be sent). The context is NOT consumed.

### InferResult

What comes back from inference. Wraps InferResponse + updates the context.

```rust
pub struct InferResult {
    pub response: InferResponse,       // from neuron-turn
}
```

`CompiledContext::infer(provider)` calls `provider.infer(request)` and returns the result.
The response is NOT automatically appended to context. That's a context op the caller
chooses to run (or not).

## Fluent Assembly API

Extension traits that wrap `ctx.run(op)` for clean syntax:

```rust
#[async_trait]
pub trait AssemblyExt {
    async fn inject_system(&mut self, prompt: &str) -> Result<(), EngineError>;
    async fn inject_message(&mut self, msg: Message) -> Result<(), EngineError>;
    async fn inject_messages(&mut self, msgs: Vec<Message>) -> Result<(), EngineError>;
    async fn compact(&mut self, f: impl FnMut(&[Message]) -> Vec<Message> + Send + 'static) -> Result<(), EngineError>;
    async fn compact_if(&mut self, pred: impl Fn(&Context) -> bool, f: impl FnMut(&[Message]) -> Vec<Message> + Send + 'static) -> Result<(), EngineError>;
}
```

Each method internally creates the corresponding op struct and calls `self.run(op)`.

## Reference Operations

All implement `ContextOp`:

| Op | Output | What it does |
|----|--------|-------------|
| `InjectSystem` | `()` | Inserts a system message at position 0 |
| `InjectMessage` | `()` | Appends a message |
| `InjectMessages` | `()` | Appends multiple messages |
| `Compact` | `CompactResult` | Runs compaction closure on messages |
| `AppendResponse` | `()` | Appends an InferResponse as assistant message |
| `ExecuteTool` | `Content` | Dispatches a tool call via ToolRegistry |

## Reference Rules

| Rule | Trigger | What it does |
|------|---------|-------------|
| `BudgetGuard` | `Before(Infer)` | Halts if cost/turns/time exceed limits |
| `AutoCompact` | `When(tokens > threshold)` | Runs compaction when context is large |
| `TelemetryRecorder` | `AfterAny` | Records metrics after every op |

## react_loop() — The ReAct Pattern as a Function

```rust
pub async fn react_loop<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    config: &ReactLoopConfig,
) -> Result<OperatorOutput, EngineError> {
    // assemble system prompt
    ctx.inject_system(&config.system_prompt).await?;

    // load history from state if session exists
    // ... (caller does this before calling react_loop)

    loop {
        let compiled = ctx.compile(&config.compile_config());
        let result = compiled.infer(provider).await?;

        // Append assistant response to context
        ctx.run(AppendResponse(result.response.clone())).await?;

        if !result.response.has_tool_calls() {
            return Ok(make_output(result.response, ExitReason::Complete, &ctx));
        }

        // Dispatch tools
        for call in &result.response.tool_calls {
            let tool_result = ctx.run(ExecuteTool::new(call.clone(), tools.clone(), tool_ctx.clone())).await?;
            ctx.inject_message(InferResponse::tool_result_message(&call.id, &call.name, tool_result)).await?;
        }

        ctx.metrics.turns_completed += 1;
    }
    // Budget rules fire on every run(), so budget exhaustion is handled by BudgetGuard halting
}
```

## Module Structure

```
src/
  lib.rs          — crate root, re-exports
  context.rs      — Context, Extensions, TurnMetrics
  op.rs           — ContextOp trait, ErasedOp
  rule.rs         — Rule, Trigger
  error.rs        — EngineError
  compile.rs      — CompiledContext, InferResult, compile(), infer()
  assembly.rs     — AssemblyExt trait + fluent methods
  ops/
    mod.rs        — re-exports
    inject.rs     — InjectSystem, InjectMessage, InjectMessages
    compact.rs    — Compact, CompactResult
    response.rs   — AppendResponse
    tool.rs       — ExecuteTool
  rules/
    mod.rs        — re-exports
    budget.rs     — BudgetGuard
    compact.rs    — AutoCompact
    telemetry.rs  — TelemetryRecorder
  react.rs        — react_loop(), ReactLoopConfig
```


## Compaction

### The primitive

`Compact` in `ops/compact.rs` takes a closure `FnMut(&[Message]) -> Vec<Message>` and applies
it to the context's message list. This is the building block all pre-built strategies use
internally.

### Pre-built strategies

`rules/compaction.rs` provides `CompactionRule` — a `Rule` that wraps a named strategy and
fires via a `When` trigger (predicate on message count or estimated token budget):

| Strategy | Provider needed | StateStore needed | What it does |
|---|---|---|---|
| `sliding_window` | no | no | Retains the N most recent messages; drops older ones |
| `policy_trim` | no | no | Drops messages by `CompactionPolicy` (`DiscardWhenDone` first, then `CompressFirst`) |
| `summarize` | yes | no | Summarizes a window of messages into a single assistant message |
| `cognitive_state_extract` | yes | yes | Extracts facts/summaries and writes them to `StateStore`; retrieval happens at assembly time |

`CompactionRule` fires when its `When` predicate returns true. Typical predicates: message
count exceeds a threshold, or estimated token count crosses a fraction of the model's context
window.

### Why not a separate crate

Strategies share `Context`, `Message`, and `CompactionPolicy` from `layer0` — the same types
the context engine already imports. Moving them to a separate crate would add a crate
boundary without adding isolation: the dependency footprint is identical and the type universe
is shared. The activation mechanism (`Rule` + `Trigger`) is defined in the context engine
itself. All of this belongs together.