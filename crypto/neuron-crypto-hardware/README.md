# neuron-crypto-hardware

> PKCS#11 / YubiKey PIV hardware crypto provider for neuron (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-crypto-hardware.svg)](https://crates.io/crates/neuron-crypto-hardware)
[![docs.rs](https://docs.rs/neuron-crypto-hardware/badge.svg)](https://docs.rs/neuron-crypto-hardware)
[![license](https://img.shields.io/crates/l/neuron-crypto-hardware.svg)](LICENSE-MIT)

## Overview

`neuron-crypto-hardware` will implement `CryptoProvider` backed by hardware security tokens
via the PKCS#11 interface. Supported devices include YubiKey (PIV), HSMs, and any PKCS#11
v2.40-compatible token.

> **Status: stub.** The trait implementation and config types are defined; the PKCS#11 client
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-crypto-hardware = "0.4"
neuron-crypto = "0.4"
```

```rust
use neuron_crypto_hardware::Pkcs11CryptoProvider;
use neuron_crypto::CryptoProvider;
use std::sync::Arc;

let crypto: Arc<dyn CryptoProvider> = Arc::new(Pkcs11CryptoProvider::new(
    "/usr/lib/libykcs11.so",  // PKCS#11 module path
    "01:02:AB",               // key label / CKA_ID
)?);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
