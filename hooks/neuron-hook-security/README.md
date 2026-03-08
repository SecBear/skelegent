# neuron-hook-security

> Security middleware for neuron — redaction and exfiltration detection

[![crates.io](https://img.shields.io/crates/v/neuron-hook-security.svg)](https://crates.io/crates/neuron-hook-security)
[![docs.rs](https://docs.rs/neuron-hook-security/badge.svg)](https://docs.rs/neuron-hook-security)
[![license](https://img.shields.io/crates/l/neuron-hook-security.svg)](LICENSE-MIT)

## Overview

`neuron-hook-security` provides ready-made middleware implementations that plug into
neuron's per-boundary middleware stacks to enforce security policies at operator lifecycle
boundaries.

Included middleware:

| Middleware | What it does |
|------------|-------------|
| `RedactionMiddleware` | Scans outgoing content for patterns (regex or literal) and redacts matches before they reach the model or any output sink |
| `ExfilGuardMiddleware` | Inspects tool results and model responses for data-loss-prevention (DLP) signals; configurable block-or-alert policy |

## Usage

```toml
[dependencies]
neuron-hook-security = "0.4"
layer0 = "0.4"
```

```rust
use neuron_hook_security::RedactionMiddleware;

let redaction = RedactionMiddleware::new(vec![
    r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b", // credit card pattern
])?;

// Add to a StoreStack or DispatchStack as appropriate
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
