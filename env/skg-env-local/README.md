# skg-env-local

> Local `Environment` implementation for skelegent — credential injection and audit

[![crates.io](https://img.shields.io/crates/v/skg-env-local.svg)](https://crates.io/crates/skg-env-local)
[![docs.rs](https://docs.rs/skg-env-local/badge.svg)](https://docs.rs/skg-env-local)
[![license](https://img.shields.io/crates/l/skg-env-local.svg)](LICENSE-MIT)

## Overview

`skg-env-local` implements the `Environment` trait from [`layer0`](../../layer0) for single-process
deployments. It resolves credentials on demand via a pluggable
[`skg-secret`](../../secret/skg-secret) `SecretResolver` and injects them into the operator's
process using one of three delivery modes:

| Mode | Delivery |
|------|----------|
| `EnvVar` | Set an environment variable for the duration of the operator call |
| `File` | Write credential bytes to a file path; clean up on drop |
| `Sidecar` | Pass a path hint; the sidecar process manages the credential |

Every credential access emits a `SecretAccessEvent` through the `EnvironmentEventSink` for
audit logging. Higher-level runtimes can adapt those signals into their own observability pipelines.

## Usage

```toml
[dependencies]
skg-env-local = "0.4"
skg-secret = "0.4"
```

```rust
use skg_env_local::LocalEnv;
use skg_secret_env::EnvResolver;
use std::sync::Arc;

let env = LocalEnv::new(Arc::new(EnvResolver));
// Pass env to operators and orchestrators
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
