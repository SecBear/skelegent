# neuron-auth-static

> Static auth provider — always returns the same token (dev/test)

[![crates.io](https://img.shields.io/crates/v/neuron-auth-static.svg)](https://crates.io/crates/neuron-auth-static)
[![docs.rs](https://docs.rs/neuron-auth-static/badge.svg)](https://docs.rs/neuron-auth-static)
[![license](https://img.shields.io/crates/l/neuron-auth-static.svg)](LICENSE-MIT)

## Overview

`neuron-auth-static` provides the simplest possible `AuthProvider`: it wraps a fixed token
string and returns it on every `token()` call without any expiry or refresh logic.

Use it for:
- Local development against services that accept a static dev token
- Unit tests that need an `AuthProvider` without network calls
- Prototyping

**Do not use in production.** Tokens don't rotate and are stored in plaintext.

## Usage

```toml
[dependencies]
neuron-auth-static = "0.4"
neuron-auth = "0.4"
```

```rust
use neuron_auth_static::StaticAuthProvider;
use neuron_auth::AuthProvider;
use std::sync::Arc;

let auth: Arc<dyn AuthProvider> = Arc::new(StaticAuthProvider::new("my-dev-token"));
let token = auth.token().await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
