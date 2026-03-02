# Operators

Operators are the core execution unit in neuron. An operator implements `layer0::Operator` and encapsulates everything needed to process one agent cycle: context assembly, model calls, tool execution, and output construction.

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

neuron ships two operator implementations: `ReactOperator` (full reasoning loop with tools) and `SingleShotOperator` (one model call, no tools).

## ReactOperator

**Crate:** `neuron-op-react`

The ReAct operator implements the Reason-Act-Observe cycle:

1. **Assemble context** -- Build the prompt from the system prompt, conversation history, tool definitions, and the new input message.
2. **Call the model** -- Send the assembled context to the provider.
3. **Check for tool use** -- If the model requested tool calls, execute them.
4. **Backfill results** -- Add tool results to the conversation context.
5. **Repeat** -- Loop back to step 2 until the model produces a final response or a limit is reached.

### Construction

```rust,no_run
use neuron_op_react::{ReactConfig, ReactOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_hooks::HookRegistry;
use neuron_tool::ToolRegistry;

let config = ReactConfig {
    system_prompt: "You are a coding assistant.".into(),
    default_model: "claude-haiku-4-5-20251001".into(),
    default_max_tokens: 4096,
    default_max_turns: 10,
};

let provider = AnthropicProvider::new("sk-ant-...");
let tools = ToolRegistry::new();   // add tools as needed
let hooks = HookRegistry::new();   // add hooks as needed

let operator = ReactOperator::new(provider, tools, hooks, config);
```

### Configuration

`ReactConfig` sets the static defaults for the operator instance:

| Field | Default | Description |
|-------|---------|-------------|
| `system_prompt` | `""` | Base system prompt prepended to every request |
| `default_model` | `""` | Model identifier (e.g., `"claude-haiku-4-5-20251001"`) |
| `default_max_tokens` | `4096` | Max tokens per model response |
| `default_max_turns` | `10` | Max ReAct loop iterations before stopping |

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

The ReAct loop stops when:

- **`Complete`** -- The model produced a final text response without requesting any tool use.
- **`MaxTurns`** -- The `max_turns` limit was reached.
- **`BudgetExhausted`** -- Accumulated cost exceeded `max_cost`.
- **`Timeout`** -- Wall-clock time exceeded `max_duration`.
- **`ObserverHalt`** -- A hook returned `HookAction::Halt`.
- **`CircuitBreaker`** -- Too many consecutive failures (provider errors or tool errors).
- **`Error`** -- An unrecoverable error occurred.

### Effects

The `ReactOperator` supports effect-producing tools. If a tool is registered in the operator's `EffectTools` configuration, calling it produces an `Effect` in the `OperatorOutput` instead of executing the tool directly. This is useful for tools that should be executed by the orchestrator or environment rather than inline (e.g., spawning a sub-agent, signaling a workflow).

## SingleShotOperator

**Crate:** `neuron-op-single-shot`

The single-shot operator makes exactly one model call with no tool use. It is useful for:

- Classification tasks
- Summarization
- Structured data extraction
- Any task where tool use is not needed

```rust,no_run
use neuron_op_single_shot::{SingleShotConfig, SingleShotOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_hooks::HookRegistry;

let config = SingleShotConfig {
    system_prompt: "Classify the following text into one of: positive, negative, neutral.".into(),
    default_model: "claude-haiku-4-5-20251001".into(),
    default_max_tokens: 100,
};

let provider = AnthropicProvider::new("sk-ant-...");
let hooks = HookRegistry::new();

let operator = SingleShotOperator::new(provider, hooks, config);
```

### Behavior

1. Assemble context from the system prompt and input message.
2. Call the model once.
3. Return the response immediately.

There is no loop, no tool execution, and no iteration. The exit reason is always `Complete` on success.

## Choosing between operators

| Use case | Operator | Why |
|----------|----------|-----|
| Agent with tools | `ReactOperator` | Needs the reasoning loop to call tools and iterate |
| Classification/extraction | `SingleShotOperator` | One model call is sufficient |
| Summarization | `SingleShotOperator` | No tools needed |
| Code generation with testing | `ReactOperator` | May need to run tests, read errors, and iterate |
| Multi-step research | `ReactOperator` | Needs to search, read, and synthesize |

## Using operators as trait objects

Both operators implement `layer0::Operator`, which is object-safe. You can use them interchangeably behind `Box<dyn Operator>` or `Arc<dyn Operator>`:

```rust,no_run
use layer0::operator::Operator;
use std::sync::Arc;

let react_op: Arc<dyn Operator> = Arc::new(react_operator);
let single_op: Arc<dyn Operator> = Arc::new(single_shot_operator);

// Orchestrator doesn't know or care which operator it's dispatching to
orchestrator.register_agent("coder", react_op);
orchestrator.register_agent("classifier", single_op);
```

The provider's generic type parameter is erased at the `Operator` boundary. Callers never see the concrete provider type.


## Custom operators: barrier scheduling and steering

Some systems (Rho-like) prefer explicit, opt-in execution mechanics where the operator owns batching and when-to-call-tools decisions, and the orchestrator owns effect execution, signals, and queries. Neuron keeps defaults slim by putting that behavior behind explicit operator implementations.

- Barrier scheduling: accumulate `tool_use` requests, execute them in batches at explicit barriers.
- Steering: inject guidance/messages between batches without changing the default ReAct semantics.
- Effects boundary: the operator declares `effects`, the orchestrator executes them.

Planned extension points (kept out of defaults):
- ToolExecutionStrategy — per-batch policies (parallel vs sequential, retry/backoff).
- SteeringSource — injects steering content between batches (policy-, safety-, or topology-driven).

Example operator skeleton (see the `custom-operator-barrier` example crate for a runnable version):
```rust
use std::sync::Arc;
use layer0::content::{Content, ContentBlock};
use layer0::operator::{Operator, OperatorInput, OperatorOutput, ExitReason};
use neuron_tool::ToolRegistry;

struct BarrierOperator { tools: ToolRegistry }

# #[allow(dead_code)]
impl BarrierOperator {
    fn new(tools: ToolRegistry) -> Self { Self { tools } }
}

# #[allow(dead_code)]
#[async_trait::async_trait]
impl Operator for BarrierOperator {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, layer0::error::OperatorError> {
        let mut out = vec![];
        let mut batch: Vec<(String, String, serde_json::Value)> = vec![];
        // Treat `text == "BARRIER"` as a flush point
        if let Content::Blocks(blocks) = input.message {
            for b in blocks {
                match b {
                    ContentBlock::ToolUse { id, name, input } => batch.push((id, name, input)),
                    ContentBlock::Text { text } if text.trim() == "BARRIER" => {
                        // flush(batch) — call tools then inject steering text
                        out.push(ContentBlock::Text { text: "[steer] batch flushed".into() });
                    }
                    other => out.push(other),
                }
            }
            // final flush(batch)
        }
        Ok(OperatorOutput::new(Content::Blocks(out), ExitReason::Complete))
    }
}
```

This keeps ReAct defaults unchanged. If you want this behavior, you opt into a different operator (or swap the inner loop via composition).

### Migration from Rho

- `rho-ai` model/providers → `neuron-turn` + concrete providers in `neuron-provider-*`.
- `rho-tools` → implement `neuron_tool::ToolDyn` and register in a `ToolRegistry`.
- `rho` loop → implement a custom operator that owns batching/steering (see example above).