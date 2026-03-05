# Packaging, Versioning, and Umbrella Crate

## Purpose

The redesign must be consumable.

Old Neuron shipped an umbrella `neuron` crate with feature flags and a prelude.

The redesign needs an equivalent packaging story.

## Requirements

- Provide an umbrella crate (likely `neuron`) that re-exports protocol + key implementations behind feature flags.
- Provide a stable set of feature flags for:
  - providers
  - MCP
  - orchestration implementations
  - state backends
  - hooks
  - environment implementations
- Provide a prelude that covers the happy path.

## Current Status

The `neuron` umbrella crate exists with feature-gated re-exports and a `prelude` module.
