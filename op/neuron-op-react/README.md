# neuron-op-react

> ReAct operator for neuron — model + tools in a reasoning loop

[![crates.io](https://img.shields.io/crates/v/neuron-op-react.svg)](https://crates.io/crates/neuron-op-react)
[![docs.rs](https://docs.rs/neuron-op-react/badge.svg)](https://docs.rs/neuron-op-react)
[![license](https://img.shields.io/crates/l/neuron-op-react.svg)](LICENSE-MIT)

## Overview

`neuron-op-react` implements the
[ReAct (Reason + Act)](https://arxiv.org/abs/2210.03629) operator pattern: a loop that
repeatedly calls a model, processes tool calls the model emits, feeds results back, and continues
until the model produces a final answer or a budget limit is hit.

The loop terminates when:
- The model returns a final text response with no tool calls
- A configured `max_turns` is exceeded
- A `max_cost` or `max_duration` budget is exhausted

All termination reasons are surfaced in the `OperatorOutput` via `ExitReason`.

## Usage

```toml
[dependencies]
neuron-op-react = "0.4"
neuron-turn = "0.4"
neuron-tool = "0.4"
neuron-hooks = "0.4"
```

```rust
use neuron_op_react::ReactOperator;
use neuron_turn::OperatorConfig;
use std::sync::Arc;

let operator = ReactOperator::new(
    Arc::new(my_provider),
    Arc::new(tool_registry),
    Arc::new(hook_registry),
);

let output = operator.invoke(input, &env).await?;
```

### Configuration

```rust
let config = OperatorConfig {
    max_turns: Some(10),
    max_cost: Some(dec!(0.50)),   // USD
    max_duration: Some(Duration::from_secs(30)),
    ..Default::default()
};
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
