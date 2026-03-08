# Building a custom operator

`ReactOperator` ships with conservative defaults: every tool runs sequentially, nothing is injected between turns, and no budget events are forwarded. Four independent primitives let you layer in exactly the behaviour you need without rebuilding the loop from scratch.

## The four-primitive wiring pattern

Each primitive is opt-in and composes independently:

- **`with_planner`** — execution strategy: how sub-dispatches are batched and sequenced.
- **`with_steering`** — external control flow: inject messages mid-loop at batch boundaries.
- **`with_budget_sink`** — lifecycle events: receive step-limit, loop-detection, and timeout notifications.
- **`with_interceptor`** — loop interception: observe or intervene at each hook point inside the ReAct loop.

A full builder chain wiring all four:

```rust,no_run
use std::sync::Arc;
use neuron_op_react::{ReactOperator, ReactConfig, BarrierPlanner, BudgetEventSink};
use neuron_op_react::SteeringSource;
use neuron_op_react::intercept::ReactInterceptor;
use neuron_tool::ToolRegistry;
use neuron_turn::context::ContextStrategy;

// (provider, tools, context_strategy, state_reader, config come from your setup)
let op = ReactOperator::new(provider, tools, context_strategy, state_reader, config)
    .with_planner(Box::new(BarrierPlanner))
    .with_steering(Arc::new(my_steering_source))
    .with_budget_sink(Arc::new(my_budget_sink))
    .with_interceptor(Arc::new(my_interceptor));
```

Each builder method is independent. Use only what you need.

### Concurrency with BarrierPlanner

The default planner runs every tool exclusively (one at a time). `BarrierPlanner` batches `Shared` tools together and flushes on `Exclusive` tools. It requires a `ConcurrencyDecider` to classify each tool:

```rust,no_run
use neuron_op_react::{BarrierPlanner, ConcurrencyDecider, Concurrency};

struct MyDecider;
impl ConcurrencyDecider for MyDecider {
    fn concurrency(&self, operator_name: &str) -> Concurrency {
        match operator_name {
            "read_file" | "search" => Concurrency::Shared,
            _ => Concurrency::Exclusive,
        }
    }
}

let op = ReactOperator::new(/* ... */)
    .with_planner(Box::new(BarrierPlanner))
    .with_concurrency_decider(Box::new(MyDecider));
```

If your tools carry `ToolConcurrencyHint` metadata, use `.with_metadata_concurrency()` instead of writing a custom decider — it reads that hint directly from the `ToolRegistry`.

## Implementing a ReactInterceptor

`ReactInterceptor` provides typed per-hook-point methods for observing and intervening in the ReAct loop. Every method has a default no-op implementation — override only what you need. See the [Middleware & Interception](hooks.md) guide for the full trait definition.

### Guardrail: deny a tool by name

Return `SubDispatchAction::Halt` or `SubDispatchAction::Skip` from `pre_sub_dispatch` to block a tool call:

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, SubDispatchAction, LoopState};
use serde_json::Value;

struct DenyToolInterceptor {
    denied: String,
}

#[async_trait]
impl ReactInterceptor for DenyToolInterceptor {
    async fn pre_sub_dispatch(
        &self,
        _state: &LoopState,
        tool_name: &str,
        _input: &Value,
    ) -> SubDispatchAction {
        if tool_name == self.denied {
            SubDispatchAction::Halt {
                reason: format!("tool {} is denied by policy", self.denied),
            }
        } else {
            SubDispatchAction::Continue
        }
    }
}
```

Use `SubDispatchAction::Skip` instead of `Halt` if you want the agent to continue after skipping — `Skip` replaces the tool result with a synthetic "skipped by policy" message and lets the loop proceed.

### Transformer: sanitize dispatch input

Return `SubDispatchAction::ModifyInput` from `pre_sub_dispatch` to rewrite tool input before it reaches the tool:

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, SubDispatchAction, LoopState};
use serde_json::Value;

struct StripSecretInterceptor;

#[async_trait]
impl ReactInterceptor for StripSecretInterceptor {
    async fn pre_sub_dispatch(
        &self,
        _state: &LoopState,
        _tool_name: &str,
        input: &Value,
    ) -> SubDispatchAction {
        if let Some(obj) = input.as_object() {
            if obj.contains_key("api_key") {
                let mut cleaned = obj.clone();
                cleaned.remove("api_key");
                return SubDispatchAction::ModifyInput {
                    new_input: Value::Object(cleaned),
                };
            }
        }
        SubDispatchAction::Continue
    }
}
```

### Observer: telemetry without side effects

Override `post_inference` or `post_sub_dispatch` to log metrics. Since these methods return `Continue`, the loop proceeds normally:

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, ReactAction, SubDispatchResult, LoopState};
use layer0::content::Content;

struct MetricsInterceptor;

#[async_trait]
impl ReactInterceptor for MetricsInterceptor {
    async fn post_inference(&self, state: &LoopState, _response: &Content) -> ReactAction {
        tracing::info!(
            turns = state.turns_completed,
            cost = %state.cost,
            "inference complete"
        );
        ReactAction::Continue
    }

    async fn post_sub_dispatch(
        &self,
        state: &LoopState,
        tool_name: &str,
        _result: &str,
    ) -> SubDispatchResult {
        tracing::info!(
            tool = tool_name,
            cost = %state.cost,
            "tool dispatch complete"
        );
        SubDispatchResult::Continue
    }
}
```

## Implementing a SteeringSource

`SteeringSource` is a narrow bridge: it supplies `SteeringCommand` values (messages or context commands) to inject into the conversation at batch boundaries, without inspecting any internal turn state.

```rust,no_run
use neuron_turn::types::{ProviderMessage, Role, ContentPart};
use neuron_turn_kit::SteeringCommand;
use neuron_op_react::SteeringSource;
use std::sync::Mutex;

struct ChannelSteering {
    queue: Mutex<Vec<SteeringCommand>>,
}

impl SteeringSource for ChannelSteering {
    fn drain(&self) -> Vec<SteeringCommand> {
        self.queue.lock().unwrap().drain(..).collect()
    }
}
```

**When `drain` is called:** at every batch boundary before the next tool batch executes, and again after each tool within a shared batch. If `drain` returns a non-empty list, the messages are injected into the conversation context and the remaining tools in that batch are skipped with a synthetic result. The loop then re-runs from the model call with the injected context.

**Thread safety:** `drain` is called from the operator's async executor. Use a `Mutex` or `Arc<Mutex<_>>` for the internal queue. For lock-free variants, an `AtomicBool` flag plus `Mutex` draining works well.

**Empty is cheap:** returning an empty `Vec` from `drain` is the common case. The operator checks `is_empty()` before proceeding.

Attach the source:

```rust,no_run
let op = ReactOperator::new(/* ... */)
    .with_steering(Arc::new(ChannelSteering {
        queue: Mutex::new(Vec::new()),
    }));
```

## Making steering observable

Steering and interception are separate concerns. You can observe (and optionally block) steering injection via the interceptor.

### PreSteeringInject: log or block injection

`pre_steering_inject` is called after `drain()` returns non-empty, before the messages enter context. Return `ReactAction::Halt` to block the injection entirely.

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, ReactAction, LoopState};

struct SteeringLogger;

#[async_trait]
impl ReactInterceptor for SteeringLogger {
    async fn pre_steering_inject(&self, _state: &LoopState, messages: &[String]) -> ReactAction {
        for msg in messages {
            tracing::info!(steering_message = %msg, "steering inject");
        }
        ReactAction::Continue // return Halt to block injection
    }
}
```

### PostSteeringSkip: observe skipped operators

`post_steering_skip` is called after tools are skipped because steering injected messages. This is observation-only — the skip already happened.

```rust,no_run
use async_trait::async_trait;
use neuron_op_react::intercept::{ReactInterceptor, LoopState};

struct SkipAuditor;

#[async_trait]
impl ReactInterceptor for SkipAuditor {
    async fn post_steering_skip(&self, _state: &LoopState, skipped: &[String]) {
        tracing::warn!(tools = ?skipped, "tools skipped by steering");
    }
}
```
