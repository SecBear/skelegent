# Building a custom operator

`ReactOperator` ships with conservative defaults: every tool runs sequentially, nothing is injected between turns, and no budget events are forwarded. Three independent primitives let you layer in exactly the behaviour you need without rebuilding the loop from scratch.

## The three-primitive wiring pattern

Each primitive is opt-in and composes independently:

- **`with_planner`** — execution strategy: how sub-dispatches are batched and sequenced.
- **`with_steering`** — external control flow: inject messages mid-loop at batch boundaries.
- **`with_budget_sink`** — lifecycle events: receive step-limit, loop-detection, and timeout notifications.

A full builder chain wiring all three:

```rust,no_run
use std::sync::Arc;
use neuron_op_react::{ReactOperator, ReactConfig, BarrierPlanner, BudgetEventSink};
use neuron_op_react::SteeringSource;
use neuron_hooks::HookRegistry;
use neuron_tool::ToolRegistry;
use neuron_turn::context::ContextStrategy;

// (provider, tools, context_strategy, hooks, state_reader, config come from your setup)
let op = ReactOperator::new(provider, tools, context_strategy, hooks, state_reader, config)
    .with_planner(Box::new(BarrierPlanner))
    .with_steering(Arc::new(my_steering_source))
    .with_budget_sink(Arc::new(my_budget_sink));
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

## Implementing a HookKind-aware hook

Hooks attach to the turn's inner loop at typed `HookPoint`s. `HookKind` controls how a hook's action composes with others at the same point (see [Hooks guide](hooks.md) for dispatch rules).

### Guardrail: deny a tool by name

A guardrail short-circuits on `Halt` or `SkipDispatch`. Register one when you want hard policy enforcement:

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;
use std::sync::Arc;

struct DenyToolHook {
    denied: String,
}

#[async_trait]
impl Hook for DenyToolHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSubDispatch]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if ctx.operator_name.as_deref() == Some(&self.denied) {
            Ok(HookAction::Halt {
                reason: format!("tool {} is denied by policy", self.denied),
            })
        } else {
            Ok(HookAction::Continue)
        }
    }
}

// Register as a guardrail so it short-circuits on Halt:
// registry.add_guardrail(Arc::new(DenyToolHook { denied: "rm".into() }));
```

Use `HookAction::SkipDispatch` instead of `Halt` if you want the agent to continue after skipping — `SkipDispatch` replaces the tool result with a synthetic "skipped by policy" message and lets the loop proceed.

### Transformer: sanitize dispatch input

A transformer sees the context mutated by the previous transformer in the chain. Register one when you need to rewrite data before it reaches the tool:

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct StripSecretTransformer;

#[async_trait]
impl Hook for StripSecretTransformer {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSubDispatch]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if let Some(mut input) = ctx.operator_input.clone() {
            if let Some(obj) = input.as_object_mut() {
                obj.remove("api_key");
            }
            return Ok(HookAction::ModifyDispatchInput { new_input: input });
        }
        Ok(HookAction::Continue)
    }
}

// Register as a transformer so subsequent transformers see the sanitized input:
// registry.add_transformer(Arc::new(StripSecretTransformer));
```

### Observer: telemetry without side effects

An observer runs regardless of other hooks' actions, and its return value is discarded. Register one for logging, metrics, and tracing:

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct MetricsHook;

#[async_trait]
impl Hook for MetricsHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSubDispatch, HookPoint::PostSubDispatch]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        tracing::info!(
            tool = ?ctx.operator_name,
            point = ?ctx.point,
            cost = %ctx.cost,
            "hook fired"
        );
        Ok(HookAction::Continue)
    }
}

// Register as an observer — actions are discarded, errors are logged, never halt:
// registry.add_observer(Arc::new(MetricsHook));
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

**Empty is cheap:** returning an empty `Vec` from `drain` is the common case. The operator checks `is_empty()` before dispatching hooks.

Attach the source:

```rust,no_run
let op = ReactOperator::new(/* ... */)
    .with_steering(Arc::new(ChannelSteering {
        queue: Mutex::new(Vec::new()),
    }));
```

## Making steering observable

Steering and hooks are separate concerns with different control flows. You can observe (and optionally block) steering injection via two hook points.

### PreSteeringInject: log or block injection

Fires after `drain()` returns non-empty, before the messages enter context. Guardrails can return `Halt` to block the injection entirely.

`ctx.steering_messages` contains the messages as debug-formatted strings — one entry per `ProviderMessage` that `drain()` returned.

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct SteeringLogger;

#[async_trait]
impl Hook for SteeringLogger {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSteeringInject]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if let Some(msgs) = &ctx.steering_messages {
            for msg in msgs {
                tracing::info!(steering_message = %msg, "steering inject");
            }
        }
        Ok(HookAction::Continue) // return Halt to block injection
    }
}

// As an observer (logging only, cannot block):
// registry.add_observer(Arc::new(SteeringLogger));
//
// As a guardrail (can return Halt to block injection):
// registry.add_guardrail(Arc::new(SteeringLogger));
```

### PostSteeringSkip: observe skipped operators

Fires after tools are skipped because steering injected messages. `ctx.skipped_operators` contains the names of the operators that were skipped.

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct SkipAuditor;

#[async_trait]
impl Hook for SkipAuditor {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PostSteeringSkip]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if let Some(skipped) = &ctx.skipped_operators {
            tracing::warn!(tools = ?skipped, "tools skipped by steering");
        }
        Ok(HookAction::Continue)
    }
}

// register.add_observer(Arc::new(SkipAuditor));
```

`PostSteeringSkip` is observation-only by design: returning `Halt` at this point halts the turn, but it does not un-skip the tools — the skip already happened.