# Contributing to neuron

Thank you for your interest in contributing to neuron! This document covers the
process for contributing and the standards we maintain.

## Project overview

neuron is a Rust workspace implementing a 6-layer composable agentic AI
architecture. Each layer builds on the protocol traits defined in `layer0`.
See the root `Cargo.toml` for the full list of workspace members.

## Getting started

### Prerequisites

- **Rust 1.90+** (edition 2021)
- A working internet connection for downloading crate dependencies

### With Nix (recommended)

If you have [Nix](https://nixos.org/) installed:

```bash
nix develop
```

This provides the full development environment including Rust, clippy, rustfmt,
cargo-deny, lychee, and mdbook.

### Without Nix

Install Rust via [rustup](https://rustup.rs/) and ensure you have the stable
toolchain with clippy and rustfmt components:

```bash
rustup component add clippy rustfmt
```

### Fork and branch workflow

1. Fork the repository on GitHub.
2. Clone your fork locally:
   ```bash
   git clone https://github.com/<your-username>/neuron.git
   cd neuron
   ```
3. Create a feature branch from `main`:
   ```bash
   git checkout -b feat/my-feature main
   ```
4. Make your changes, following the conventions below.
5. Push your branch and open a Pull Request against `main`.

## Conventions

All coding conventions, architectural decisions, and design principles are
documented in [`AGENTS.md`](./AGENTS.md) at the repository root. Read it before
submitting your first PR. Key highlights:

### Rust standards

- **Edition 2021**, resolver 2, minimum Rust 1.90
- **`#[async_trait]`** for async trait methods (not native async traits)
- **`thiserror`** for error types, two levels of nesting maximum
- **`schemars`** for JSON Schema derivation on tool inputs
- No `unwrap()` in library code
- `#[must_use]` on Result-returning functions

### Workspace structure

The workspace is organized in 6 layers:

| Layer | Purpose | Crates |
|-------|---------|--------|
| 0 | Protocol traits + types | `layer0` |
| 1 | Turn implementations | `neuron-turn`, `neuron-context`, `neuron-tool`, `neuron-mcp`, `neuron-provider-*`, `neuron-op-*` |
| 2 | Orchestration | `neuron-orch-local`, `neuron-orch-kit` |
| 3 | State | `neuron-state-memory`, `neuron-state-fs` |
| 4 | Environment | `neuron-env-local`, `neuron-secret-*`, `neuron-auth-*`, `neuron-crypto-*` |
| 5 | Cross-cutting | `neuron-hooks`, `neuron-hook-security` |
| - | Umbrella | `neuron` |

### Documentation

- Inline `///` doc comments on **every** public item.
- Every trait must have a doc example.
- When adding or changing public API, update all documentation surfaces in the
  same commit: source doc comments, crate `AGENTS.md`, crate `README.md`,
  examples, root `AGENTS.md`, and `llms.txt` as applicable.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

Format:

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `ci`, `perf`.

Scope is typically the crate name without the `neuron-` prefix (e.g., `tool`,
`orch-local`, `provider-anthropic`). Use `layer0` for the trait crate. Use no
scope for workspace-wide changes.

Examples:

```
feat(tool): add middleware support for ToolRegistry
fix(orch-local): handle workflow cancellation correctly
docs: update all doc surfaces for v2 architecture
chore: add release-please config and initial CHANGELOGs
```

## Running checks

Before submitting a PR, run full verification:

```bash
./scripts/verify.sh
```

The canonical command set is defined in `AGENTS.md §Verification`. CI runs the same
checks. If `cargo doc` warnings matter for your change, also run:
`nix develop -c cargo doc --workspace --no-deps`.

## Pull request process

1. Fill out the PR template completely.
2. Ensure all CI checks pass.
3. Keep PRs focused -- one concern per PR.
4. If your change adds a public type or trait, confirm you have updated all
   documentation surfaces.
5. Add or update tests for any behavioral changes.

## License

By contributing to neuron, you agree that your contributions will be dual
licensed under the [MIT License](./LICENSE-MIT) and the
[Apache License 2.0](./LICENSE-APACHE), at the user's option. This is the same
license used by the project itself.
