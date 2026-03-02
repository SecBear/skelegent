# AGENTS.md

This file is the entrypoint for any coding agent (Codex, Claude Code, etc.) working in this repo.
It defines what to load, in what order, and what quality gates must be satisfied before claiming
work is complete.

## Prime Directive: One Task Per Context

Treat the context window like a fixed-size allocation: once you mix tasks, you lose coherence.

Rules:

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

## Where Truth Lives

1. Requirements and intended behavior live in `specs/`.
2. Operational constraints (how we work, how to verify, how to avoid repeated failure modes)
   live in `rules/`.
3. Deep rationale and history live in `docs/` and `DEVELOPMENT-LOG.md`.

If there is a conflict:

1. Specs override rules.
2. Rules override ad-hoc agent behavior.
3. If the specs are ambiguous, update the specs (do not invent behavior).

## Backpressure (Verification Gates)

This repo assumes Rust tooling is provided by Nix. Do not assume `cargo` exists on PATH.

Use these commands as your default backpressure:

1. Format: `nix develop -c nix fmt`
2. Tests: `nix develop -c cargo test --workspace --all-targets`
3. Lints: `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

Do not claim "done" unless you have fresh evidence from the relevant command(s) for the change.

## TDD Policy

When feasible:

1. Write a failing test that demonstrates the required behavior (RED).
2. Implement the minimum to pass (GREEN).
3. Refactor while keeping tests green (REFACTOR).

Exceptions are allowed only for:

1. Pure formatting changes.
2. Pure documentation changes.
3. Configuration-only changes where tests are not meaningful (but verification is still required).

## Codifying Learnings (Build Your Stdlib)

When a failure mode repeats or an agent needs steering:

1. Fix the immediate issue.
2. Encode the lesson so it does not recur:
   - If it's a behavior requirement: update/add a spec in `specs/` and link it from `SPECS.md`.
   - If it's a process constraint: update/add a rule in `rules/`.

Goal: make the correct outcome the path of least resistance.

## Related Documents

This repo already includes a deeper, Neuron-specific project guide in `CLAUDE.md`. Agents should
consult it after the spec/rules stack when doing any substantial work.

## Loop Files

This repo is designed to be run in a deterministic loop:

1. `PROMPT.md` is the loop prompt.
2. `ralph_queue.md` is the single prioritized queue.


## Composability Philosophy (Neuron v2)

- Layer 0 must stay stable: object-safe traits + serde wire types. Additive changes preferred; breaking changes are planned and versioned.
- Effects are the side‑effect boundary: operators declare, orchestrators/environments execute. No direct writes from operators.
- Hooks are for policy/observability/redaction, not control flow. Halt/Skip/Modify are explicit; do not encode scheduling in hooks.
- Execution mechanics are explicit and opt‑in:
  - ToolExecutionStrategy (default: sequential). Optional barrier scheduling with Shared/Exclusive + batch flush + parallel shared tools.
  - SteeringSource polled at defined boundaries to inject mid‑loop messages and optionally skip remaining tools.
  - Optional streaming tool API; forward chunks via a ToolExecutionUpdate hook point.
- Defaults must remain slim for simple use cases. Advanced behavior is opt‑in and modular; avoid boolean soup.
- Orchestrator owns the reference effect interpreter and minimal signal/query semantics (local first, durable later).
- Credentials are resolved/injected via Environment + secret/auth/crypto backends; tests must prove no secret leakage.
- Conformance: golden tests prove provider swap, state swap, operator swap, and orchestration compose deterministically.

## Architecture Principles (Execution + Composability)

- Layer 0 is protocol only: object-safe traits + serde wire types. No execution policy, no technology bindings, no durability semantics.
- Effects boundary is sacred: operators declare; orchestrators/environments execute. Operators must not write state directly.
- Hooks vs Steering:
  - Hooks are event-triggered observation/intervention at defined points (pre/post inference/tool, exit) with explicit actions.
  - Steering is operator-initiated control flow: the runtime decides when to poll, may inject messages, and may skip current batches. Keep steering out of hooks.
- Defaults stay slim: sequential tools, no steering, no streaming, local best-effort effects. Advanced behavior is opt-in via small, composable traits (no boolean soup).
- Turn engine decomposition: prefer composing these primitives over monolithic loops:
  - ContextAssembler, ToolExecutionPlanner, ConcurrencyDecider, BatchExecutor, SteeringSource, HookDispatcher, EffectSynthesizer, ExitController.
- Tool metadata is source of truth: concurrency hints (Shared/Exclusive) live on the tool definition; deciders read metadata first and may layer policy.
- Single authority for limits: budget/time/turns live in ExitController; planners only observe remaining budget/time (read-only).
- Local vs durable: keep LocalEffectExecutor lean (in-order, best-effort). Durable semantics (idempotency keys, retries, sagas) belong to durable orchestrators, not Layer 0 or local executors.
- Streaming is observation-only: ToolExecutionUpdate is read-only; it must not alter control flow.
- Invariants: preserve tool_use → tool_result pairing; on steering, emit placeholders for skipped tools.
- Refactor guardrail: behavior-preserving refactors must pass the full test suite before adding new capabilities via decomposed traits.
- Conformance: prove composition patterns (provider/state/operator/orchestration swaps) with golden tests; enforce CI backpressure (fmt, clippy -D warnings, tests).