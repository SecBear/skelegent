# neuron-hooks

> Hook registry and pipeline for neuron operators

[![crates.io](https://img.shields.io/crates/v/neuron-hooks.svg)](https://crates.io/crates/neuron-hooks)
[![docs.rs](https://docs.rs/neuron-hooks/badge.svg)](https://docs.rs/neuron-hooks)
[![license](https://img.shields.io/crates/l/neuron-hooks.svg)](LICENSE-MIT)

## Overview

`neuron-hooks` provides `HookRegistry`, which collects multiple `Hook` implementations into an
ordered dispatch pipeline. At each hook point, hooks are called in registration order. The pipeline
short-circuits on `Halt`, `SkipTool`, or `ModifyToolInput` — subsequent hooks are not called. Hook
errors are logged and the pipeline continues.

The `Hook` trait and all associated types (`HookPoint`, `HookAction`, `HookContext`, `HookError`)
are defined in [`layer0`](../../layer0).

## Exports

- **`HookRegistry`** — `new()`, `add(Arc<dyn Hook>)`, `dispatch(&HookContext) -> HookAction`

Re-used from `layer0`: `Hook`, `HookPoint`, `HookAction`, `HookContext`, `HookError`

## Usage

```toml
[dependencies]
neuron-hooks = "0.4"
layer0 = "0.4"
async-trait = "0.1"
```

### Implementing a custom hook

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

pub struct BudgetHook {
    max_cost_usd: f64,
}

#[async_trait]
impl Hook for BudgetHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreInference]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        let cost: f64 = ctx.cost.try_into().unwrap_or(0.0);
        if cost >= self.max_cost_usd {
            return Ok(HookAction::Halt { reason: "budget exceeded".into() });
        }
        Ok(HookAction::Continue)
    }
}
```

### Registering hooks

```rust,no_run
use neuron_hooks::HookRegistry;
use std::sync::Arc;

let mut registry = HookRegistry::new();
registry.add(Arc::new(BudgetHook { max_cost_usd: 1.0 }));
```

### Dispatching

```rust,no_run
use layer0::hook::{HookContext, HookPoint};

let ctx = HookContext::new(HookPoint::PreInference);
let action = registry.dispatch(&ctx).await;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
