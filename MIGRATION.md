# Migration Guide — Neuron Redesign (vNext)

This guide helps downstreams move to the new crates and APIs with minimal friction.

## TL;DR
- Use the umbrella crate where possible:
  ```toml
  [dependencies]
  neuron = { version = "x.y.z", features = [
    "op-react", "provider-anthropic", "provider-openai", "provider-ollama",
    "orch-local", "state-fs", "hooks", "mcp"
  ] }
  ```
- Or pull crates directly:
  - `layer0` (protocol), `neuron-turn`, `neuron-tool`, `neuron-op-react`, `neuron-orch-local`, `neuron-state-fs`, `neuron-hooks`, `neuron-mcp`
  - New: `neuron-turn-kit`, `neuron-effects-core`, `neuron-effects-local`

## Operators
- ReactOperator keeps the same external API and defaults (sequential, no steering, no streaming)
- New opt‑in:
  - Planning: `with_planner(Box::new(BarrierPlanner))`
  - Concurrency: `with_metadata_concurrency()` (reads `ToolDyn::concurrency_hint()`)
  - Steering: `with_steering(Arc<dyn SteeringSource>)`
  - Streaming observation: rely on `HookPoint::ToolExecutionUpdate` and `ToolDynStreaming`

## Tools
- Implement `neuron_tool::ToolDyn` (and optionally `ToolDynStreaming`)
- Set `concurrency_hint()` to `Shared` for safe parallel tools; default is `Exclusive`

## Effects
- Operator declares Effects; choose executor at orchestration boundary
- Local: `neuron-effects-local::LocalEffectExecutor`
- Durable (future): integrate a `DurableEffectExecutor` in your orchestrator

## Orchestration
- Local: `neuron-orch-local` with minimal `signal/query` and reference effect execution
- Durable (future): create `neuron-orch-temporal` or `neuron-orch-restate` implementing `layer0::Orchestrator`

## State
- `neuron-state-fs` and `neuron-state-memory` unchanged; read via `StateReader`, write via Effects

## Hooks
- `neuron-hooks` and `neuron-hook-security` unchanged; note new read‑only hook point `ToolExecutionUpdate`

## MCP
- Use `neuron-mcp` to expose MCP tools as `ToolDyn`

## Breaking changes to be aware of
- Old durable context API is replaced by Orchestrator + Effect executor separation
- Some internal module paths moved; `neuron-orch-kit::effects` now re‑exports from `neuron-effects-*`

## Suggested refactor order
1) Swap operator and provider wiring to ReactOperator + neuron-turn
2) Migrate tools to `ToolDyn`; set `concurrency_hint`
3) Route Effects to `LocalEffectExecutor` and enable `orch-local`
4) Adopt `turn-kit` for planning/steering; only opt‑in if needed
5) Adopt streaming observation if you want chunk UI

## Verification
- Run: `cargo clippy --workspace -- -D warnings` and `cargo test --workspace`
- Add integration tests proving provider swap, state swap, operator swap, and orchestration patterns

## Questions?
Open an issue with your current wiring; we’ll help map it to the new crates.
