# neuron-secret

> Secret resolution traits and types for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-secret.svg)](https://crates.io/crates/neuron-secret)
[![docs.rs](https://docs.rs/neuron-secret/badge.svg)](https://docs.rs/neuron-secret)
[![license](https://img.shields.io/crates/l/neuron-secret.svg)](LICENSE-MIT)

## Overview

`neuron-secret` defines the `SecretResolver` trait and associated types (`SecretSource`,
`SecretLease`, `SecretValue`) that the neuron credential system is built on. Secret values
are held in `SecretValue`, a zeroize-on-drop wrapper that prevents sensitive bytes from
lingering in memory.

This crate contains **no implementations** — for concrete resolvers see the backend crates:

| Backend | Crate |
|---------|-------|
| Environment variable | [`neuron-secret-env`](../neuron-secret-env) |
| HashiCorp Vault KV | [`neuron-secret-vault`](../neuron-secret-vault) |
| AWS Secrets Manager | [`neuron-secret-aws`](../neuron-secret-aws) |
| GCP Secret Manager | [`neuron-secret-gcp`](../neuron-secret-gcp) |
| OS keystore | [`neuron-secret-keystore`](../neuron-secret-keystore) |
| Kubernetes Secrets | [`neuron-secret-k8s`](../neuron-secret-k8s) |

## Usage

```toml
[dependencies]
neuron-secret = "0.4"
```

### Implementing a custom resolver

```rust
use neuron_secret::{SecretResolver, SecretSource, SecretLease};
use async_trait::async_trait;

pub struct MyVaultResolver { /* ... */ }

#[async_trait]
impl SecretResolver for MyVaultResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, neuron_secret::SecretError> {
        // fetch from your secret store
        todo!()
    }
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
