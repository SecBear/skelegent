# Secrets, Auth, and Crypto

## Purpose

Agentic systems need safe credential handling.

Skelegent separates this into:

- **Vocabulary** (Layer 0): where secrets live and how access is described (`SecretSource`, access events)
- **Behavior** (implementation crates): how secrets are resolved, how auth tokens are issued, how crypto operations are performed
- **Delivery** (Environment): how credentials are injected into a runtime boundary

## Current Implementation Status

- `layer0/src/secret.rs` exists (vocabulary).
- There are implementation crates for secret/auth/crypto interfaces and several backend stubs.

Stubs are acceptable.

Still required for “core complete”:

- a single coherent story for how secret resolution is requested by environment and audited via lifecycle/observable events
- tests that prove no secret material leaks into logs/errors by default

