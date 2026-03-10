# Testing, Examples, and Backpressure

## Purpose

Skelegent should be provably composable.

The tests and examples are the backpressure that makes this architecture real.

## Required Example Suite (Core)

Skelegent must have a small set of “proof of composition” examples that demonstrate the primitives working together:

- scheduled + state + signal (daily digest)
- multi-agent escalation + policy controls (triage)
- provider swap with parity invariants (provider parity)

These examples must share composition factories so wiring does not drift.

## Mock vs Real Paths

- Mock path must be deterministic and required in CI.
- Real path must be opt-in and env-gated.

## Current Implementation Status

Examples:
- `examples/custom_operator_barrier/` — custom Operator with barrier scheduling and tool-use steering; no live API

Workspace tests:
- `tests/poc.rs` — mock-based composability: provider swap, state swap, operator swap, multi-agent orchestration; runs in CI
- `tests/cross_provider.rs` — provider parity against Anthropic, OpenAI, Ollama; `#[ignore]`, opt-in with API keys
- `tests/umbrella_skelegent.rs` — prelude compilation smoke test

Wiring kit: `skg-orch-kit` exists at `orch/skg-orch-kit/`.

Future examples (not yet implemented):

- `examples/daily_digest` — scheduled + state + signal composition proof
- `examples/triage` — multi-agent escalation + policy controls proof
- `examples/provider_parity` — provider swap with parity invariants as a standalone example
- failure/edge-case matrix test suite proving error paths and policy edge behavior
