# neuron-auth-file

> File-based auth provider — reads a bearer token from disk

[![crates.io](https://img.shields.io/crates/v/neuron-auth-file.svg)](https://crates.io/crates/neuron-auth-file)
[![docs.rs](https://docs.rs/neuron-auth-file/badge.svg)](https://docs.rs/neuron-auth-file)
[![license](https://img.shields.io/crates/l/neuron-auth-file.svg)](LICENSE-MIT)

## Overview

`neuron-auth-file` implements `AuthProvider` by reading a bearer token from a file on disk.
Each `token()` call re-reads the file, so rotating the token (e.g., via a sidecar that refreshes
a Kubernetes projected token) is transparent to the caller.

Typical use: Kubernetes workload identity projected tokens mounted at a path like
`/var/run/secrets/tokens/my-token`.

## Usage

```toml
[dependencies]
neuron-auth-file = "0.4"
neuron-auth = "0.4"
```

```rust
use neuron_auth_file::FileAuthProvider;
use neuron_auth::AuthProvider;
use std::sync::Arc;

let auth: Arc<dyn AuthProvider> = Arc::new(
    FileAuthProvider::new("/var/run/secrets/tokens/vault-token")
);
let token = auth.token().await?;
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
