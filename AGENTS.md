# AGENTS.md

Entrypoint for any coding agent working in this repo.

## What This Project Is

Neuron is a Rust workspace implementing a 6-layer composable agentic AI architecture.
Layer 0 defines the stability contract. Layers 1-5 build implementations on top.

## Where Truth Lives

| What | Where |
|---|---|
| Architectural positions | `ARCHITECTURE.md` |
| Behavioral requirements | `specs/` (indexed by `SPECS.md`) |
| Operational constraints | `rules/` |
| Deep rationale | `docs/` |

Authority: ARCHITECTURE.md > specs > rules > agent judgment.
If specs are ambiguous, update the specs (do not invent behavior).

## Load Order

Before implementation work, load in order:

1. This file
2. `ARCHITECTURE.md`
3. `SPECS.md` then the specific spec(s) for your task
4. The relevant `rules/`

## Verification

This repo uses Nix-provided Rust tooling. All must pass before any commit:

```bash
nix develop -c nix fmt
nix develop -c cargo build --workspace
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

For layer0 test-utils: `nix develop -c cargo test --features test-utils -p layer0`

Do not claim "done" without fresh evidence from the relevant commands.

## Codifying Learnings

When a failure mode repeats:

1. Fix the immediate issue.
2. Encode: behavior requirement -> spec in `specs/`. Process constraint -> rule in `rules/`.
