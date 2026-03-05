# AGENTS.md

Entrypoint for any coding agent (Codex, Claude Code, etc.) working in this repo.
Defines what to load, what the project is, and what quality gates must pass before claiming done.

## What This Project Is

Neuron is a Rust workspace implementing a 6-layer composable agentic AI architecture.
Layer 0 (`layer0` crate) defines the stability contract: four protocol traits
(Turn, Orchestrator, StateStore, Environment), two cross-cutting interfaces
(Hook, Lifecycle events), and the message types that cross every boundary.
Layers 1-5 build implementations on top.

## Prime Directive: One Task Per Context

Treat the context window like a fixed-size allocation: once you mix tasks, you lose coherence.

1. One task per context window. If scope changes, start a fresh session.
2. When you notice drift (conflicting goals, repeating mistakes, inventing APIs), stop and restart.
3. Each loop must re-load the same stable stack (specs + rules) deterministically.

## Required Load Order (Every Session)

Load these documents in order before doing any implementation work:

1. `AGENTS.md` (this file)
2. `SPECS.md` (spec index)
3. The specific spec(s) that govern the task in `specs/`
4. The relevant operational rules in `rules/`

If you are unsure which spec applies, read `specs/00-vision-and-non-goals.md` and
`specs/01-architecture-and-layering.md` first.

For deep architectural context, continue with:

5. `ARCHITECTURE.md` — Design philosophy, architectural positions, core values.

## Where Truth Lives

1. Architectural values and design positions live in `ARCHITECTURE.md`.
2. Requirements and intended behavior live in `specs/`.
3. Operational constraints (how we work, how to verify, how to avoid repeated failure modes)
   live in `rules/`.
4. Deep rationale lives in `docs/`.

If there is a conflict:

1. ARCHITECTURE.md overrides specs.
2. Specs override rules.
3. Rules override ad-hoc agent behavior.
4. If the specs are ambiguous, update the specs (do not invent behavior).

## Backpressure (Verification Gates)

This repo assumes Rust tooling is provided by Nix. Do not assume `cargo` exists on PATH.

Use these commands as your default backpressure:

1. Format: `nix develop -c nix fmt`
2. Build: `nix develop -c cargo build --workspace`
3. Tests: `nix develop -c cargo test --workspace --all-targets`
4. Lints: `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
5. Docs: `nix develop -c cargo doc --no-deps`

All must pass before any commit. For layer0 test-utils features:

```bash
nix develop -c cargo test --features test-utils -p layer0
```

Do not claim "done" unless you have fresh evidence from the relevant command(s) for the change.

## Project Rules

### Do

- Follow `ARCHITECTURE.md` for all structural decisions
- Match layer0 trait signatures exactly — they are the stability contract
- Use `#[deny(missing_docs)]` on every public item
- Test that every message type round-trips through serde_json
- Test that every trait is object-safe (`Box<dyn Trait>` compiles and is `Send + Sync`)
- Keep layer0 dependencies minimal (serde, async-trait, thiserror, rust_decimal — that's it)

### Do Not

- Add dependencies to layer0 beyond what's already there
- Add methods to layer0 protocol traits beyond what specs and `ARCHITECTURE.md` define
- Change layer0's trait signatures — they are the stability contract
- Make layer0 traits non-object-safe
- Skip phases — the phased approach is sequential
- Make undocumented decisions — update the plan first if deviating

## TDD Policy

When feasible:

1. Write a failing test that demonstrates the required behavior (RED).
2. Implement the minimum to pass (GREEN).
3. Refactor while keeping tests green (REFACTOR).

Exceptions are allowed only for:

1. Pure formatting changes.
2. Pure documentation changes.
3. Configuration-only changes where tests are not meaningful (but verification is still required).

## Architecture Quick Reference

Full architectural positions live in `ARCHITECTURE.md`. Specs govern behavior.
Before touching architecture, read `ARCHITECTURE.md` first. Key invariants:

- **Layer 0 is protocol only.** Object-safe traits + serde wire types. No execution policy.
- **Effects boundary is sacred.** Operators declare; orchestrators/environments execute.
- **Three independent primitives.** Hooks (event-driven), Steering (poll-driven), Planner (execution strategy). Not interchangeable. See `specs/04` and `specs/09`.
- **Defaults are slim.** Advanced behavior opt-in via composable traits.

## Codifying Learnings

When a failure mode repeats or an agent needs steering:

1. Fix the immediate issue.
2. Encode the lesson so it does not recur:
   - Behavior requirement: update/add a spec in `specs/` and link from `SPECS.md`.
   - Process constraint: update/add a rule in `rules/`.
