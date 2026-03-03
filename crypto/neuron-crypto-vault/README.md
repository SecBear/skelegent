# neuron-crypto-vault

> Vault Transit crypto provider for neuron (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-crypto-vault.svg)](https://crates.io/crates/neuron-crypto-vault)
[![docs.rs](https://docs.rs/neuron-crypto-vault/badge.svg)](https://docs.rs/neuron-crypto-vault)
[![license](https://img.shields.io/crates/l/neuron-crypto-vault.svg)](LICENSE-MIT)

## Overview

`neuron-crypto-vault` will implement `CryptoProvider` backed by
[HashiCorp Vault's Transit secrets engine](https://www.vaultproject.io/docs/secrets/transit).
Key material never leaves Vault — sign and verify operations are performed server-side.

> **Status: stub.** The trait implementation and config types are defined; the Vault API client
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-crypto-vault = "0.4"
neuron-crypto = "0.4"
```

```rust
use neuron_crypto_vault::VaultCryptoProvider;
use neuron_crypto::CryptoProvider;
use std::sync::Arc;

let crypto: Arc<dyn CryptoProvider> = Arc::new(VaultCryptoProvider::new(
    "https://vault.example.com",
    "transit/keys/my-signing-key",
    auth_provider,
));
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
