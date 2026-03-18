# Operators

Operators are the core execution unit in skelegent. An operator implements `layer0::Operator` and encapsulates everything needed to process one agent cycle: context assembly, model calls, tool execution, and output construction.

## The Operator trait

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(
        &self,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OperatorError>;
}
```

skelegent ships a context engine (`skg-context-engine`) — a set of composable primitives around `react_loop` — and `SingleShotOperator` (one model call, no tools). External consumers wrap `react_loop` in their own `impl Operator` struct for the object-safe boundary.

## Context Engine

**Crate:** `skg-context-engine`

The context engine is **not** a monolithic struct. It is a set of composable primitives centered on `react_loop()`, which orchestrates the assembly → inference → reaction loop:

1. **Assemble context** -- Build the prompt from the system prompt, conversation history, tool definitions, and the new input message.
2. **Call the model** -- Send the assembled context to the provider.
3. **Check for tool use** -- If the model requested tool calls, execute them.
4. **Backfill results** -- Add tool results to the conversation context.
5. **Repeat** -- Loop back to step 2 until the model produces a final response or a limit is reached.

### Construction

To use the context engine as an `Operator`, create a wrapper struct that holds a `Provider`, `ToolRegistry`, and `ReactLoopConfig`, then implement `Operator` by constructing a `Context`, injecting the user message, and calling `react_loop()`:

```rust,no_run
use async_trait::async_trait;
use layer0::operator::{Operator, OperatorInput, OperatorOutput, OperatorError};
use layer0::context::{Message, Role};
use skg_context_engine::{Context, react_loop, ReactLoopConfig};
use skg_turn::provider::Provider;
use layer0::DispatchContext;
use skg_tool::ToolRegistry;

struct MyOperator<P: Provider> {
    provider: P,
    config: ReactLoopConfig,
    tools: ToolRegistry,
    tool_ctx: DispatchContext,
}

#[async_trait]
impl<P: Provider> Operator for MyOperator<P> {
    async fn execute(
        &self,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OperatorError> {
        // Context is the conversation store — create one per invocation
        let mut ctx = Context::new();

        // Inject domain context (shell history, file state, etc.) via assembly ops
        // ctx.inject_system("Additional context here").await?;

        // Inject the user input
        ctx.inject_message(Message::new(Role::User, input.message))
            .await
            .map_err(OperatorError::context_assembly)?;

        // react_loop composes Context, CompileConfig, AppendResponse,
        // and ExecuteTool internally — you just hand it the primitives
        react_loop(&mut ctx, &self.provider, &self.tools, &self.tool_ctx, &self.config)
            .await
            .map_err(|e| OperatorError::non_retryable(e.to_string()))
    }
}
```

The key integration pattern:

```text
Your domain context (shell history, file state, user prefs)
    ↓ feeds into
Context via inject_system(), inject_message(), or system_addendum in OperatorConfig
    ↓ manages
LLM conversation turns (Message with Role + Content)
    ↓ compiles to
CompiledContext → infer(provider) → InferResult
    ↓ response goes through
ContextOps (AppendResponse, ExecuteTool) → rules fire automatically
```

### Configuration

`ReactLoopConfig` sets the static defaults for the loop:

| Field | Default | Description |
|-------|---------|-------------|
| `system_prompt` | `""` | Base system prompt prepended to every request |
| `model` | `None` | Model identifier (e.g., `Some("claude-haiku-4-5-20251001".into())`) |
| `max_tokens` | `None` | Max tokens per model response |
| `temperature` | `None` | Sampling temperature |
These defaults can be overridden per-invocation via `OperatorConfig` in the `OperatorInput`:

```rust
use layer0::operator::{OperatorConfig, OperatorInput, TriggerType};
use layer0::content::Content;
use rust_decimal_macros::dec;

let mut input = OperatorInput::new(
    Content::text("Refactor this module"),
    TriggerType::User,
);
input.config = Some(OperatorConfig {
    max_turns: Some(20),           // Allow more iterations
    max_cost: Some(dec!(0.50)),    // Budget: $0.50
    model: Some("claude-sonnet-4-20250514".into()), // Use a different model
    ..Default::default()
});
```

### Exit reasons

The context engine loop stops when:

- **`Complete`** -- The model produced a final text response without requesting any tool use.
- **`MaxTurns`** -- The `max_turns` limit was reached.
- **`BudgetExhausted`** -- Accumulated cost exceeded `max_cost` or tool-call step limit exceeded.
- **`Timeout`** -- Wall-clock time exceeded `max_duration`.
- **`InterceptorHalt { reason }`** -- An interceptor (including a Rule that returns `RuleAction::Halt`) stopped execution.
- **`CircuitBreaker`** -- Too many consecutive failures (provider errors or tool errors).
- **`Error`** -- An unrecoverable error occurred.
- **`SafetyStop { reason }`** -- Provider safety system stopped generation (content filter or safety mechanism triggered).
- **`AwaitingApproval`** -- One or more tool calls require human approval before execution.
- **`Custom(String)`** -- Operator-defined exit reason.

### Effects

The context engine supports effect-producing tools. If a tool is registered in the operator's `EffectTools` configuration, calling it produces an `Effect` in the `OperatorOutput` instead of executing the tool directly. This is useful for tools that should be executed by the orchestrator or environment rather than inline (e.g., spawning a sub-agent, signaling a workflow).

## SingleShotOperator

**Crate:** `skg-op-single-shot`

The single-shot operator makes exactly one model call with no tool use. It is useful for:

- Classification tasks
- Summarization
- Structured data extraction
- Any task where tool use is not needed

```rust,no_run
use skg_op_single_shot::{SingleShotConfig, SingleShotOperator};
use skg_provider_anthropic::AnthropicProvider;

let config = SingleShotConfig {
    system_prompt: "Classify the following text into one of: positive, negative, neutral.".into(),
    default_model: "claude-haiku-4-5-20251001".into(),
    default_max_tokens: 100,
};

let provider = AnthropicProvider::new("sk-ant-...");

let operator = SingleShotOperator::new(provider, config);
```

### Behavior

1. Assemble context from the system prompt and input message.
2. Call the model once.
3. Return the response immediately.

There is no loop, no tool execution, and no iteration. The exit reason is always `Complete` on success.

## Choosing between operators

| Use case | Operator | Why |
|----------|----------|-----|
| Agent with tools | Context Engine | Needs the reasoning loop to call tools and iterate |
| Classification/extraction | `SingleShotOperator` | One model call is sufficient |
| Summarization | `SingleShotOperator` | No tools needed |
| Code generation with testing | Context Engine | May need to run tests, read errors, and iterate |
| Multi-step research | Context Engine | Needs to search, read, and synthesize |

## Using operators as trait objects

Both operators implement `layer0::Operator`, which is object-safe. You can use them interchangeably behind `Box<dyn Operator>` or `Arc<dyn Operator>`:

```rust,no_run
use layer0::id::OperatorId;
use layer0::operator::Operator;
use std::sync::Arc;

let engine_op: Arc<dyn Operator> = Arc::new(my_operator);
let single_op: Arc<dyn Operator> = Arc::new(single_shot_operator);

// Orchestrator doesn't know or care which operator it's dispatching to
orchestrator.register(OperatorId::new("coder"), engine_op);
orchestrator.register(OperatorId::new("classifier"), single_op);
```

The provider's generic type parameter is erased at the `Operator` boundary. Callers never see the concrete provider type.


## Custom operators: Rules as extension points

The primary extension mechanism for the context engine loop is **Rules**. Rules fire during the react loop and can inspect context, modify messages, or halt execution. For detailed guidance on building a custom operator, see:

**[Building a custom operator](custom-operator.md)**

That guide covers:
- Implementing Rules for loop interception
- Using `ContextOp` to compose assembly, inference, and reaction
- Wiring domain-specific logic into the react loop

The brief example skeleton below shows the shape of a custom operator that wraps `react_loop` with additional rule-based behavior:

```rust,no_run
use skg_context_engine::{Context, react_loop, ReactLoopConfig};
use skg_tool::ToolRegistry;

// Build your operator struct wrapping Provider + ToolRegistry + ReactLoopConfig
// (see the Construction example above), then add Rules to the Context
// before calling react_loop() to customize loop behavior.
```
