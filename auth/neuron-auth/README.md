# neuron-auth

> Authentication provider traits for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-auth.svg)](https://crates.io/crates/neuron-auth)
[![docs.rs](https://docs.rs/neuron-auth/badge.svg)](https://docs.rs/neuron-auth)
[![license](https://img.shields.io/crates/l/neuron-auth.svg)](LICENSE-MIT)

## Overview

`neuron-auth` defines the `AuthProvider` trait that the neuron secret and environment system
uses to obtain bearer tokens for authenticating outbound requests (e.g., to a Vault instance,
a k8s cluster, or a private API). Auth tokens are consumed by secret resolvers that need to
authenticate before they can fetch secrets.

This crate contains **no implementations** — for concrete providers see the backend crates:

| Backend | Crate |
|---------|-------|
| Static token (dev/test) | [`neuron-auth-static`](../neuron-auth-static) |
| File-based token | [`neuron-auth-file`](../neuron-auth-file) |
| OIDC client credentials | [`neuron-auth-oidc`](../neuron-auth-oidc) |
| Kubernetes ServiceAccount | [`neuron-auth-k8s`](../neuron-auth-k8s) |

## Usage

```toml
[dependencies]
neuron-auth = "0.4"
```

### Implementing a custom auth provider

```rust
use neuron_auth::{AuthProvider, AuthToken};
use async_trait::async_trait;

pub struct MyAuthProvider;

#[async_trait]
impl AuthProvider for MyAuthProvider {
    async fn token(&self) -> Result<AuthToken, neuron_auth::AuthError> {
        // fetch a fresh token
        todo!()
    }
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
