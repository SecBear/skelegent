# skg-auth

> Authentication provider traits for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-auth.svg)](https://crates.io/crates/skg-auth)
[![docs.rs](https://docs.rs/skg-auth/badge.svg)](https://docs.rs/skg-auth)
[![license](https://img.shields.io/crates/l/skg-auth.svg)](LICENSE-MIT)

## Overview

`skg-auth` defines the `AuthProvider` trait that the skelegent secret and environment system
uses to obtain bearer tokens for authenticating outbound requests (e.g., to a Vault instance,
a k8s cluster, or a private API). Auth tokens are consumed by secret resolvers that need to
authenticate before they can fetch secrets.

This crate contains **no implementations** — for concrete providers see the backend crates:

Additional backends planned.

## Usage

```toml
[dependencies]
skg-auth = "0.4"
```

### Implementing a custom auth provider

```rust
use skg_auth::{AuthProvider, AuthToken};
use async_trait::async_trait;

pub struct MyAuthProvider;

#[async_trait]
impl AuthProvider for MyAuthProvider {
    async fn token(&self) -> Result<AuthToken, skg_auth::AuthError> {
        // fetch a fresh token
        todo!()
    }
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
