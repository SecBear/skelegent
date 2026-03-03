# neuron-env-local

> Local `Environment` implementation for neuron — credential injection and audit

[![crates.io](https://img.shields.io/crates/v/neuron-env-local.svg)](https://crates.io/crates/neuron-env-local)
[![docs.rs](https://docs.rs/neuron-env-local/badge.svg)](https://docs.rs/neuron-env-local)
[![license](https://img.shields.io/crates/l/neuron-env-local.svg)](LICENSE-MIT)

## Overview

`neuron-env-local` implements the `Environment` trait from [`layer0`](../../layer0) for single-process
deployments. It resolves credentials on demand via a pluggable
[`neuron-secret`](../../secret/neuron-secret) `SecretResolver` and injects them into the operator's
process using one of three delivery modes:

| Mode | Delivery |
|------|----------|
| `EnvVar` | Set an environment variable for the duration of the operator call |
| `File` | Write credential bytes to a file path; clean up on drop |
| `Sidecar` | Pass a path hint; the sidecar process manages the credential |

Every credential access emits a `SecretAccessEvent` through the `EnvironmentEventSink` for
audit logging, and an `ObservableEvent` for lifecycle observability.

## Usage

```toml
[dependencies]
neuron-env-local = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_env_local::LocalEnv;
use neuron_secret_env::EnvResolver;
use std::sync::Arc;

let env = LocalEnv::new(Arc::new(EnvResolver));
// Pass env to operators and orchestrators
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
