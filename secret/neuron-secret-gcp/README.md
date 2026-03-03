# neuron-secret-gcp

> Secret resolver for GCP Secret Manager — neuron backend (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-secret-gcp.svg)](https://crates.io/crates/neuron-secret-gcp)
[![docs.rs](https://docs.rs/neuron-secret-gcp/badge.svg)](https://docs.rs/neuron-secret-gcp)
[![license](https://img.shields.io/crates/l/neuron-secret-gcp.svg)](LICENSE-MIT)

## Overview

`neuron-secret-gcp` will implement `SecretResolver` backed by
[Google Cloud Secret Manager](https://cloud.google.com/secret-manager). It reads the project,
secret name, and version from the `SecretSource` config, and authenticates via Application
Default Credentials (ADC).

> **Status: stub.** The trait implementation and config types are defined; the GCP client
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-secret-gcp = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_gcp::GcpSecretsResolver;
use neuron_secret::SecretResolver;
use std::sync::Arc;

// Uses Application Default Credentials (GOOGLE_APPLICATION_CREDENTIALS or metadata server)
let resolver: Arc<dyn SecretResolver> = Arc::new(
    GcpSecretsResolver::from_adc().await?
);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
