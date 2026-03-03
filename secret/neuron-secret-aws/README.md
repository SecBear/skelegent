# neuron-secret-aws

> Secret resolver for AWS Secrets Manager — neuron backend (stub)

[![crates.io](https://img.shields.io/crates/v/neuron-secret-aws.svg)](https://crates.io/crates/neuron-secret-aws)
[![docs.rs](https://docs.rs/neuron-secret-aws/badge.svg)](https://docs.rs/neuron-secret-aws)
[![license](https://img.shields.io/crates/l/neuron-secret-aws.svg)](LICENSE-MIT)

## Overview

`neuron-secret-aws` will implement `SecretResolver` backed by
[AWS Secrets Manager](https://aws.amazon.com/secrets-manager/). It reads the secret ARN or name
from the `SecretSource` config, calls the AWS API with ambient credentials (IAM role, instance
profile, or explicit key), and returns the secret value.

> **Status: stub.** The trait implementation and config types are defined; the AWS SDK
> integration is in progress. The interface is stable.

## Usage

```toml
[dependencies]
neuron-secret-aws = "0.4"
neuron-secret = "0.4"
```

```rust
use neuron_secret_aws::AwsSecretsResolver;
use neuron_secret::SecretResolver;
use std::sync::Arc;

// Uses ambient AWS credentials (env vars, ~/.aws, instance metadata, etc.)
let resolver: Arc<dyn SecretResolver> = Arc::new(
    AwsSecretsResolver::from_env().await?
);
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
See the [book](https://secbear.github.io/neuron) for architecture and guides.
