# neuron-auth-oidc

> OIDC client credentials / token exchange auth provider for neuron (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-auth-oidc.svg)](https://crates.io/crates/neuron-auth-oidc)
[![docs.rs](https://docs.rs/neuron-auth-oidc/badge.svg)](https://docs.rs/neuron-auth-oidc)
[![license](https://img.shields.io/crates/l/neuron-auth-oidc.svg)](LICENSE-MIT)

## Overview

`neuron-auth-oidc` will implement `AuthProvider` using the OIDC
[client credentials grant](https://datatracker.ietf.org/doc/html/rfc6749#section-4.4) and
[token exchange](https://datatracker.ietf.org/doc/html/rfc8693) flows. It handles token
caching and refresh before expiry.

> **Status: stub.** The trait implementation and config types are defined; the HTTP client
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-auth-oidc = "0.4"
neuron-auth = "0.4"
```

```rust
use neuron_auth_oidc::OidcAuthProvider;
use neuron_auth::AuthProvider;
use std::sync::Arc;

let auth: Arc<dyn AuthProvider> = Arc::new(OidcAuthProvider::builder()
    .token_url("https://auth.example.com/token")
    .client_id("my-client")
    .client_secret("my-secret")
    .build()?);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
