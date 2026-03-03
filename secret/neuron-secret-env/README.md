# neuron-secret-env

> Secret resolver that reads credentials from process environment variables

[![crates.io](https://img.shields.io/crates/v/neuron-secret-env.svg)](https://crates.io/crates/neuron-secret-env)
[![docs.rs](https://docs.rs/neuron-secret-env/badge.svg)](https://docs.rs/neuron-secret-env)
[![license](https://img.shields.io/crates/l/neuron-secret-env.svg)](LICENSE-MIT)

## Overview

`neuron-secret-env` provides a `SecretResolver` that reads secret values from the process
environment. The `SecretSource` config must specify a `var_name`; the resolver looks up that
variable and returns its value as a `SecretLease`.

Best for: local development, CI pipelines, and container environments where secrets are
injected as environment variables by the orchestration layer (Kubernetes, Docker Compose, etc.).

## Usage

```toml
[dependencies]
neuron-secret-env = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_env::EnvResolver;
use neuron_secret::{SecretResolver, SecretSource};
use std::sync::Arc;

let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver);

let source = SecretSource::Custom {
    provider: "env".into(),
    config: serde_json::json!({ "var_name": "MY_API_KEY" }),
};
let lease = resolver.resolve(&source).await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
