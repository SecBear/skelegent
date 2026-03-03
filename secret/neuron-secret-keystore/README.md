# neuron-secret-keystore

> Secret resolver for OS keystore — neuron backend (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-secret-keystore.svg)](https://crates.io/crates/neuron-secret-keystore)
[![docs.rs](https://docs.rs/neuron-secret-keystore/badge.svg)](https://docs.rs/neuron-secret-keystore)
[![license](https://img.shields.io/crates/l/neuron-secret-keystore.svg)](LICENSE-MIT)

## Overview

`neuron-secret-keystore` will implement `SecretResolver` backed by the operating system's
native credential store:

| Platform | Backend |
|----------|---------|
| macOS | Keychain |
| Windows | DPAPI / Credential Manager |
| Linux | Secret Service (GNOME Keyring / KWallet) |

> **Status: stub.** The trait implementation and config types are defined; the OS keystore
> integration (via `keyring` crate) is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-secret-keystore = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_keystore::KeystoreResolver;
use neuron_secret::SecretResolver;
use std::sync::Arc;

let resolver: Arc<dyn SecretResolver> = Arc::new(KeystoreResolver::new("my-service"));
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
