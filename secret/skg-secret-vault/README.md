# skg-secret-vault

> Secret resolver for HashiCorp Vault KV — skelegent backend (stub)

[![crates.io](https://img.shields.io/crates/v/skg-secret-vault.svg)](https://crates.io/crates/skg-secret-vault)
[![docs.rs](https://docs.rs/skg-secret-vault/badge.svg)](https://docs.rs/skg-secret-vault)
[![license](https://img.shields.io/crates/l/skg-secret-vault.svg)](LICENSE-MIT)

## Overview

`skg-secret-vault` will implement `SecretResolver` backed by
[HashiCorp Vault](https://www.vaultproject.io/) KV secrets engine. It reads the `mount`,
`path`, and `field` from the `SecretSource` config and fetches the value via the Vault HTTP API.

> **Status: stub.** The trait implementation and config types are defined; the HTTP client
> integration is in progress. The interface is stable — downstream code that wires this resolver
> into `skg-env-local` will not need to change when the implementation completes.

## Usage

```toml
[dependencies]
skg-secret-vault = "0.4"
skg-secret = "0.4"
```

```rust
use skg_secret_vault::VaultResolver;
use skg_secret::SecretResolver;
use std::sync::Arc;

let resolver: Arc<dyn SecretResolver> = Arc::new(
    VaultResolver::new("https://vault.example.com", "my-token")
);
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
