# skg-op-single-shot

> Single-shot operator — one model call, no tools, return immediately

[![crates.io](https://img.shields.io/crates/v/skg-op-single-shot.svg)](https://crates.io/crates/skg-op-single-shot)
[![docs.rs](https://docs.rs/skg-op-single-shot/badge.svg)](https://docs.rs/skg-op-single-shot)
[![license](https://img.shields.io/crates/l/skg-op-single-shot.svg)](LICENSE-MIT)

## Overview

`skg-op-single-shot` is the simplest possible operator: it calls the model exactly once with
the given input, returns the response, and exits. No tool calls. No loops. No state.

Use it for:
- Simple Q&A with an LLM
- Classification or extraction tasks
- Pipelines where you control tool calls externally
- Testing provider integrations

## Usage

```toml
[dependencies]
skg-op-single-shot = "0.4"
skg-turn = "0.4"
```

```rust
use skg_op_single_shot::SingleShotOperator;
use layer0::{Operator, OperatorInput};
use std::sync::Arc;

let operator = SingleShotOperator::new(Arc::new(my_provider));
let input = OperatorInput::new("What is the capital of France?");

let output = operator.invoke(input, &env).await?;
println!("{}", output.content.as_text().unwrap_or_default());
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
