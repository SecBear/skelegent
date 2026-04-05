> **RETIRED** — versioning section superseded by `specs/v2/10-errors-versioning-and-conformance.md`; packaging obligations carried forward into v2 adoption.

# Packaging, Versioning, and Umbrella Crate

## Purpose

The redesign must be consumable.

Old skelegent shipped an umbrella `skelegent` crate with feature flags and a prelude.

The redesign needs an equivalent packaging story.

## Requirements

- Provide an umbrella crate (likely `skelegent`) that re-exports protocol + key implementations behind feature flags.
- Provide a stable set of feature flags for:
  - providers
  - MCP
  - orchestration implementations
  - state backends
  - hooks
  - environment implementations
- Provide a prelude that covers the happy path.

## Current Status

The `skelegent` umbrella crate exists with feature-gated re-exports and a `prelude` module.
