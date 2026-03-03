# Cleanup Plan - Parallel Work Items

## Item 1: EffectExecutor Refactor in orch-kit
- Rename `EffectExecutor` trait in runner.rs to `EffectInterpreter`
- Rename `LocalEffectExecutor` struct in runner.rs to `LocalEffectInterpreter`
- Update lib.rs exports
- Update kit.rs imports and method signatures
- Update tests/runner.rs imports
- Remove `tracing` from Cargo.toml dependencies
- Remove `_touch_errors` function from runner.rs
- Update neuron-orch-kit/README.md with correct API

## Item 2: Dead code cleanup in op-react
- Remove `_tool_uses` variable (lines ~491-500)
- Leave `_steered` for now (deferred)

## Item 3: Fix stale READMEs (5 crates)
- layer0/README.md
- neuron-mcp/README.md
- neuron-hooks/README.md
- neuron-tool/README.md
- neuron-orch-local/README.md
