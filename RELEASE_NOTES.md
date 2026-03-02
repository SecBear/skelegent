# Neuron Redesign (vNext) — Release Notes

This release introduces a composable, layered runtime with stable protocol boundaries, execution primitives for planner/steering/streaming, and a split effect executor. It is a breaking redesign intended to make multi‑agent systems predictable, testable, and durable‑ready.

## Highlights
- Layer 0 stability preserved: object‑safe traits (`Operator`, `Orchestrator`, `StateStore`, `Environment`, `Hook`) + serde wire types
- New turn engine primitives (`neuron-turn-kit`): `ToolExecutionPlanner`, `ConcurrencyDecider`, `BatchExecutor` (execution‑only), `SteeringSource`
- New effect executor split:
  - `neuron-effects-core` — trait + error + policy
  - `neuron-effects-local` — in‑process effect execution (in‑order, best‑effort)
- React Operator improvements (opt‑in; defaults unchanged):
  - BarrierPlanner (Rho‑style Shared/Exclusive batching)
  - SteeringSource (operator‑initiated mid‑loop injection + skip placeholders)
  - Streaming tools (`ToolDynStreaming`) + `HookPoint::ToolExecutionUpdate` (read‑only)
  - Tool‑level concurrency hints (`ToolConcurrencyHint`) with metadata‑based decider
- Local orchestrator: minimal `signal/query` semantics + reference effect execution
- Umbrella crate (`neuron`) and example custom operator included
- CI: fmt, clippy ‑D warnings, tests, coverage, audit, deny, docs

## New crates
- `neuron-turn-kit` — turn engine primitives
- `neuron-effects-core` — effect executor trait
- `neuron-effects-local` — local effect executor impl

## Backward compatibility
- Old runtime/durable context is superseded by Orchestrator + Effect executor separation. Operators remain pure and durable‑agnostic.
- `neuron-orch-kit` re‑exports the effect executor crates for back‑compat paths.
- `neuron-op-react` now depends on `neuron-turn-kit` but preserves public API and default behavior.

## Migration (for downstreams)
- Prefer the umbrella crate with features: `neuron = { version = "x.y.z", features = ["op-react", "provider-anthropic", "orch-local", "state-fs", "hooks"] }`
- If using reaction loop directly:
  - Swap to `neuron-turn-kit` traits for planning/steering; defaults remain sequential/no‑steering
  - Adopt `ToolDynStreaming` + `HookPoint::ToolExecutionUpdate` to observe tool chunk updates
- For effect execution: use `neuron-effects-local` or integrate a durable executor in your orchestrator
- See MIGRATION.md for full details

## Deprecations
- Legacy durable context patterns are deprecated; replace with Orchestrator implementations (Temporal/Restate) and a durable effect executor

## Known limitations
- Durable orchestrators (Temporal/Restate) are not included in this release; wiring guidance provided in docs

## Acknowledgements
Thanks to the Rho integration work for motivating `turn-kit` and the effect split.
