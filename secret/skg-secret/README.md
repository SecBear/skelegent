# skg-secret

> Secret resolution traits and types for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-secret.svg)](https://crates.io/crates/skg-secret)
[![docs.rs](https://docs.rs/skg-secret/badge.svg)](https://docs.rs/skg-secret)
[![license](https://img.shields.io/crates/l/skg-secret.svg)](LICENSE-MIT)

## Overview

`skg-secret` defines the `SecretResolver` trait and associated types (`SecretSource`,
`SecretLease`, `SecretValue`) that the skelegent credential system is built on. Secret values
are held in `SecretValue`, a zeroize-on-drop wrapper that prevents sensitive bytes from
lingering in memory.

This crate contains **no implementations** — for concrete resolvers see the backend crates:

| Backend | Crate |
|---------|-------|
| HashiCorp Vault KV | [`skg-secret-vault`](../skg-secret-vault) |

## Usage

```toml
[dependencies]
skg-secret = "0.4"
```

### Implementing a custom resolver

```rust
use skg_secret::{SecretResolver, SecretSource, SecretLease};
use async_trait::async_trait;

pub struct MyVaultResolver { /* ... */ }

#[async_trait]
impl SecretResolver for MyVaultResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<SecretLease, skg_secret::SecretError> {
        // fetch from your secret store
        todo!()
    }
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
