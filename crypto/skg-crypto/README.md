# skg-crypto

> Cryptographic provider traits for skelegent — signing and verification

[![crates.io](https://img.shields.io/crates/v/skg-crypto.svg)](https://crates.io/crates/skg-crypto)
[![docs.rs](https://docs.rs/skg-crypto/badge.svg)](https://docs.rs/skg-crypto)
[![license](https://img.shields.io/crates/l/skg-crypto.svg)](LICENSE-MIT)

## Overview

`skg-crypto` defines the `CryptoProvider` trait for signing and verifying data within
the skelegent system. It provides an abstraction over key material and signing backends, keeping
the operator code independent of whether keys are held in software, a hardware token, or a
remote KMS.

This crate contains **no implementations** — for concrete providers see the backend crates:

Additional backends planned.

## Usage

```toml
[dependencies]
skg-crypto = "0.4"
```

### Implementing a custom crypto provider

```rust
use skg_crypto::{CryptoProvider, SignatureBytes};
use async_trait::async_trait;

pub struct MySigningProvider;

#[async_trait]
impl CryptoProvider for MySigningProvider {
    async fn sign(&self, data: &[u8]) -> Result<SignatureBytes, skg_crypto::CryptoError> {
        // sign data with your key
        todo!()
    }

    async fn verify(&self, data: &[u8], sig: &SignatureBytes) -> Result<bool, skg_crypto::CryptoError> {
        todo!()
    }
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
