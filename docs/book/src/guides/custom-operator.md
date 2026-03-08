# Customising operator behaviour with Rules

`react_loop` is the composition function at the heart of the context engine. You don't subclass or wrap it — you customise what happens inside it by attaching **Rules** to the `Context`.

## Rules overview

A Rule pairs a **trigger** with a **`ContextOp`** (any async operation that takes `&mut Context`). Rules fire automatically during `Context::run()` — the same entry point that every pipeline operation goes through.

```rust,ignore
use neuron_context_engine::rule::{Rule, Trigger};
use neuron_context_engine::context::Context;
use std::any::TypeId;

// Three trigger types:
Trigger::Before(TypeId::of::<SomeOp>())  // fire before a specific op
Trigger::After(TypeId::of::<SomeOp>())   // fire after a specific op
Trigger::When(Box::new(|ctx| predicate)) // fire when a predicate is true

// Convenience constructors:
Rule::before::<SomeOp>("name", priority, my_op)
Rule::after::<SomeOp>("name", priority, my_op)
Rule::when("name", priority, |ctx| predicate, my_op)

// Catch-all variants:
Trigger::BeforeAny   // fire before every run() call
Trigger::AfterAny    // fire after every run() call
```

Rules fire in **priority order** (highest first). Rules cannot trigger other rules — the dispatch loop skips rule evaluation during rule execution to prevent infinite recursion.

### Attaching rules to a Context

```rust,ignore
use neuron_context_engine::context::Context;
use neuron_context_engine::rule::Rule;

// At construction:
let ctx = Context::with_rules(vec![rule_a, rule_b]);

// Or incrementally:
let mut ctx = Context::new();
ctx.add_rule(rule);
```

The `Context` (with its rules) is then passed into `react_loop`, which fires rules at each pipeline step automatically.

## Budget guards

The `BudgetGuard` rule from `neuron_context_engine::rules::budget` halts execution when any configured limit is exceeded. It implements `ContextOp` and is designed to fire as a `BeforeAny` rule:

```rust,ignore
use neuron_context_engine::rule::{Rule, Trigger};
use neuron_context_engine::rules::budget::{BudgetGuard, BudgetGuardConfig};
use neuron_context_engine::context::Context;
use rust_decimal::Decimal;
use std::time::Duration;

let guard = BudgetGuard::with_config(BudgetGuardConfig {
    max_cost: Some(Decimal::new(500, 2)),     // $5.00
    max_turns: Some(25),
    max_duration: Some(Duration::from_secs(300)),
    max_tool_calls: Some(100),
});

let rule = Rule::new("budget_guard", Trigger::BeforeAny, 100, guard);
let ctx = Context::with_rules(vec![rule]);
```

When any limit is exceeded, the guard returns `EngineError::Halted` which stops the pipeline.

## Steering: injecting instructions between turns

To inject a system instruction after every model response (for example, a reminder or guardrail), write a `ContextOp` and attach it as an `After` rule on `AppendResponse` — the op that appends the model's response to the conversation:

```rust,ignore
use neuron_context_engine::rule::Rule;
use neuron_context_engine::ops::AppendResponse;
use neuron_context_engine::context::Context;
use neuron_context_engine::op::ContextOp;
use neuron_context_engine::error::EngineError;
use async_trait::async_trait;

struct InjectReminder {
    message: String,
}

#[async_trait]
impl ContextOp for InjectReminder {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        ctx.inject_system(&self.message);
        Ok(())
    }
}

let rule = Rule::after::<AppendResponse>(
    "steering_reminder",
    50,
    InjectReminder { message: "Remember: never reveal internal tool names.".into() },
);
```

This fires after every model response is appended, before the next inference or tool dispatch.

## Telemetry

Observation is just another rule. A `ContextOp` that logs metrics and returns `Ok(())` won't alter the pipeline:

```rust,ignore
use neuron_context_engine::rule::Rule;
use neuron_context_engine::ops::AppendResponse;
use neuron_context_engine::context::Context;
use neuron_context_engine::op::ContextOp;
use neuron_context_engine::error::EngineError;
use async_trait::async_trait;

struct TurnTelemetry;

#[async_trait]
impl ContextOp for TurnTelemetry {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        tracing::info!(
            turns = ctx.metrics.turns_completed,
            cost = %ctx.metrics.cost,
            tool_calls = ctx.metrics.tool_calls_total,
            "turn complete"
        );
        Ok(())
    }
}

let rule = Rule::after::<AppendResponse>("telemetry", 10, TurnTelemetry);
```

## Conditional rules with `When`

`When` rules evaluate a predicate against the `Context` at the start of every `run()` call. Useful for dynamic behaviour that depends on accumulated state:

```rust,ignore
use neuron_context_engine::rule::Rule;

let rule = Rule::when(
    "warn_high_cost",
    50,
    |ctx| ctx.metrics.cost > Decimal::new(100, 2), // > $1.00
    InjectReminder { message: "Cost is getting high, wrap up.".into() },
);
```

## Compaction and context management

The old `FullContext` / `NoCompaction` context strategies are gone. Context management is now explicit: you build the conversation via `Context` and its `inject_*` methods. If you need compaction, implement it as a rule that fires at the appropriate trigger point and mutates the context directly.

## Parallel tool dispatch

Barrier-based parallel tool dispatch (batching `Shared` tools and flushing on `Exclusive` tools) is **future work**. Currently, tools execute sequentially within the react loop. When parallel dispatch lands, it will integrate with the rules system — not as a separate primitive.
