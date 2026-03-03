# neuron-crypto

> Cryptographic provider traits for neuron — signing and verification

[![crates.io](https://img.shields.io/crates/v/neuron-crypto.svg)](https://crates.io/crates/neuron-crypto)
[![docs.rs](https://docs.rs/neuron-crypto/badge.svg)](https://docs.rs/neuron-crypto)
[![license](https://img.shields.io/crates/l/neuron-crypto.svg)](LICENSE-MIT)

## Overview

`neuron-crypto` defines the `CryptoProvider` trait for signing and verifying data within
the neuron system. It provides an abstraction over key material and signing backends, keeping
the operator code independent of whether keys are held in software, a hardware token, or a
remote KMS.

This crate contains **no implementations** — for concrete providers see the backend crates:

| Backend | Crate |
|---------|-------|
| HashiCorp Vault Transit | [`neuron-crypto-vault`](../neuron-crypto-vault) |
| PKCS#11 / YubiKey PIV | [`neuron-crypto-hardware`](../neuron-crypto-hardware) |

## Usage

```toml
[dependencies]
neuron-crypto = "0.4"
```

### Implementing a custom crypto provider

```rust
use neuron_crypto::{CryptoProvider, SignatureBytes};
use async_trait::async_trait;

pub struct MySigningProvider;

#[async_trait]
impl CryptoProvider for MySigningProvider {
    async fn sign(&self, data: &[u8]) -> Result<SignatureBytes, neuron_crypto::CryptoError> {
        // sign data with your key
        todo!()
    }

    async fn verify(&self, data: &[u8], sig: &SignatureBytes) -> Result<bool, neuron_crypto::CryptoError> {
        todo!()
    }
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
