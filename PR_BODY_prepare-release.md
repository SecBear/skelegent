Title: Neuron redesign/v2 → main: turn-kit, effects split, opt-in planning/steering/streaming, local orchestration updates

Summary
- Layer 0 protocol stable/unchanged. New crates:
  - neuron-turn-kit: ToolExecutionPlanner, BarrierPlanner, ConcurrencyDecider, SteeringSource, BatchExecutor
  - neuron-effects-core / neuron-effects-local: effects contract + local executor
- neuron-op-react migrated to turn-kit primitives; defaults unchanged (sequential, no steering/streaming unless opted in)
- Tooling additions (opt-in):
  - ToolDynStreaming + HookPoint::ToolExecutionUpdate (non_exhaustive)
  - ToolConcurrencyHint + metadata-driven decider (default Exclusive)
- Orchestration:
  - neuron-orch-local: minimal signal/query + journal; effect executor wired via neuron-effects-local
- Docs/examples:
  - Updated guides: operators, orchestration, tools
  - Added example: examples/custom_operator_barrier
  - Added execution/composability principles in rules/09-execution-principles-and-composability.md and RFC
- Release prep:
  - Version bump to 0.4.0 across crates
  - Internal deps now include version alongside path for publish compatibility
  - .github/workflows/publish.yml updated to include new crates in order
  - RELEASE_NOTES.md and MIGRATION.md added

Compatibility
- Defaults remain slim: sequential tools, no steering/streaming unless explicitly enabled
- Layering preserved: no execution policy in Layer 0; durable execution belongs to orchestrator/effect executor implementations
- Rho UX unchanged (for later migration): no CLI/settings/TUI drift

Migration notes
- Prefer neuron-turn-kit APIs for planning/steering; operators declare effects only; effect executor handles side-effects
- See MIGRATION.md for crate mapping and version notes

Verification
- nix fmt (taplo/rustfmt): clean
- cargo clippy --workspace --all-targets -- -D warnings: clean
- cargo test --workspace --all-targets: green
- cargo publish --dry-run: leaf crates succeed (layer0, neuron-tool). Deeper crates require dependency presence in index and will pass during ordered publish workflow.

Checklist
- [x] Version bumps applied, internal deps include versions
- [x] Docs updated (guides, crate map, RFC, release notes, migration)
- [x] Publish workflow includes new crates
- [x] Local verify green
- [ ] CI green on PR
- [ ] Run publish workflow in dry-run mode after merge
- [ ] Publish in order after dry-run success

Notes
- Codex websocket transport can be forcibly disabled at runtime via env (PI_CODEX_WEBSOCKET=false; PI_CODEX_WEBSOCKET_V2=false) to avoid transient WS issues.
