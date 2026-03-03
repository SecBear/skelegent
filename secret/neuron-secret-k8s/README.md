# neuron-secret-k8s

> Secret resolver for Kubernetes Secrets — neuron backend (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-secret-k8s.svg)](https://crates.io/crates/neuron-secret-k8s)
[![docs.rs](https://docs.rs/neuron-secret-k8s/badge.svg)](https://docs.rs/neuron-secret-k8s)
[![license](https://img.shields.io/crates/l/neuron-secret-k8s.svg)](LICENSE-MIT)

## Overview

`neuron-secret-k8s` will implement `SecretResolver` backed by the Kubernetes Secrets API.
Running inside a Pod, it uses the projected service account token to authenticate against the
kube-apiserver and read the specified secret by namespace and name.

> **Status: stub.** The trait implementation and config types are defined; the `kube-rs`
> client integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-secret-k8s = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_k8s::K8sSecretsResolver;
use neuron_secret::SecretResolver;
use std::sync::Arc;

// Picks up in-cluster config automatically when running in a Pod
let resolver: Arc<dyn SecretResolver> = Arc::new(
    K8sSecretsResolver::in_cluster().await?
);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
