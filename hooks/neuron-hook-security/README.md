# neuron-hook-security

> Security hooks for neuron — redaction and exfiltration detection

[![crates.io](https://img.shields.io/crates/v/neuron-hook-security.svg)](https://crates.io/crates/neuron-hook-security)
[![docs.rs](https://docs.rs/neuron-hook-security/badge.svg)](https://docs.rs/neuron-hook-security)
[![license](https://img.shields.io/crates/l/neuron-hook-security.svg)](LICENSE-MIT)

## Overview

`neuron-hook-security` provides ready-made `Hook` implementations that plug into a
[`neuron-hooks`](../neuron-hooks) `HookRegistry` to enforce security policies at the
operator lifecycle boundary.

Included hooks:

| Hook | What it does |
|------|-------------|
| `RedactionHook` | Scans outgoing content for patterns (regex or literal) and redacts matches before they reach the model or any output sink |
| `ExfiltrationHook` | Inspects tool results and model responses for data-loss-prevention (DLP) signals; configurable block-or-alert policy |

## Usage

```toml
[dependencies]
neuron-hook-security = "0.4"
neuron-hooks = "0.4"
```

```rust
use neuron_hook_security::RedactionHook;
use neuron_hooks::HookRegistry;

let mut registry = HookRegistry::new();
registry.register(RedactionHook::new(vec![
    r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b", // credit card pattern
])?);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
