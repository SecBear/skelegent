# neuron-auth-k8s

> Kubernetes ServiceAccount projected token auth provider for neuron (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-auth-k8s.svg)](https://crates.io/crates/neuron-auth-k8s)
[![docs.rs](https://docs.rs/neuron-auth-k8s/badge.svg)](https://docs.rs/neuron-auth-k8s)
[![license](https://img.shields.io/crates/l/neuron-auth-k8s.svg)](LICENSE-MIT)

## Overview

`neuron-auth-k8s` will implement `AuthProvider` using a Kubernetes projected ServiceAccount
token. When running inside a Pod, the kubelet mounts a short-lived OIDC token at a known
path; this provider reads it and refreshes it automatically before expiry.

> **Status: stub.** The trait implementation and config types are defined; the token refresh
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-auth-k8s = "0.4"
neuron-auth = "0.4"
```

```rust
use neuron_auth_k8s::K8sTokenAuthProvider;
use neuron_auth::AuthProvider;
use std::sync::Arc;

// Token path matches your Pod's projected volume mount
let auth: Arc<dyn AuthProvider> = Arc::new(
    K8sTokenAuthProvider::new("/var/run/secrets/tokens/vault")
);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
