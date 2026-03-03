# neuron-secret-vault

> Secret resolver for HashiCorp Vault KV — neuron backend (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-secret-vault.svg)](https://crates.io/crates/neuron-secret-vault)
[![docs.rs](https://docs.rs/neuron-secret-vault/badge.svg)](https://docs.rs/neuron-secret-vault)
[![license](https://img.shields.io/crates/l/neuron-secret-vault.svg)](LICENSE-MIT)

## Overview

`neuron-secret-vault` will implement `SecretResolver` backed by
[HashiCorp Vault](https://www.vaultproject.io/) KV secrets engine. It reads the `mount`,
`path`, and `field` from the `SecretSource` config and fetches the value via the Vault HTTP API.

> **Status: stub.** The trait implementation and config types are defined; the HTTP client
> integration is in progress. The interface is stable — downstream code that wires this resolver
> into `neuron-env-local` will not need to change when the implementation completes.

## Usage

```toml
[dependencies]
neuron-secret-vault = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_vault::VaultResolver;
use neuron_secret::SecretResolver;
use std::sync::Arc;

let resolver: Arc<dyn SecretResolver> = Arc::new(
    VaultResolver::new("https://vault.example.com", "my-token")
);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
