# skg-provider-anthropic

> Anthropic Claude API provider for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-provider-anthropic.svg)](https://crates.io/crates/skg-provider-anthropic)
[![docs.rs](https://docs.rs/skg-provider-anthropic/badge.svg)](https://docs.rs/skg-provider-anthropic)
[![license](https://img.shields.io/crates/l/skg-provider-anthropic.svg)](LICENSE-MIT)

## Overview

`skg-provider-anthropic` implements the `Provider` trait from
[`skg-turn`](../../turn/skg-turn) for the
[Anthropic Messages API](https://docs.anthropic.com/en/api/messages). It handles request
serialization, response parsing, tool call routing, and cost accounting for Claude models.

Supports: `claude-opus-4`, `claude-sonnet-4`, `claude-haiku-3-5`, and any future model
accepted by the Messages API.

## Usage

```toml
[dependencies]
skg-provider-anthropic = "0.4"
skg-turn = "0.4"
```

### Setup

Three constructors are available:

```rust
// Static key
let provider = AnthropicProvider::new("sk-ant-...");

// Environment variable (resolved per request)
let provider = AnthropicProvider::from_env_var("ANTHROPIC_API_KEY");

// AuthProvider — pi coding agent OAuth or OMP (recommended)
use std::sync::Arc;
use skg_auth_pi::PiAuthProvider; // from skelegent-extras

let auth = PiAuthProvider::from_env().expect("~/.pi/agent/auth.json not found");
let provider = AnthropicProvider::with_auth(Arc::new(auth));
```

For proxy or test overrides, use `provider.with_url(url)`.

### OAuth tokens (Claude Max / pi coding agent)

`with_auth()` calls the provider at every request, so token refresh is transparent — no
manual re-initialization required.

OAuth tokens (`sk-ant-oat*`) are automatically sent as `Authorization: Bearer` with the
`anthropic-beta: oauth-2025-04-20` header, which the Anthropic API requires for Claude Max
subscription tokens. Regular API keys continue to use `x-api-key`.

`PiAuthProvider` and `OmpAuthProvider` live in `skelegent-extras` (separate repo:
<https://github.com/SecBear/skelegent-extras>).

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/skelegent) for architecture and guides.
