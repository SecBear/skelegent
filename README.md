# Neuron (redesign/v2) — composable agentic runtime

Neuron is an experiment in building an agentic system that is **composable by construction**:
layered protocol contracts, swappable providers/tools/state, and deterministic backpressure via
tests and specs.

Specs are the source of truth: `SPECS.md` and `specs/`.

## Quickstart (Nix)

This repo assumes Rust tooling is provided by the Nix flake.

- Run tests: `nix develop -c cargo test`
- Run lints: `nix develop -c cargo clippy -- -D warnings`
- Format: `nix develop -c nix fmt`
- Full local verification: `./scripts/verify.sh`

## Ralph loop (agentic queue)

The repo has a deterministic “what next” queue at `ralph_queue.md`, driven by `PROMPT.md`.

- Claude Code: `./scripts/ralph.sh`
- Codex: `CODEX=1 ./scripts/ralph.sh`

## Crate map (workspace members)

Core protocol / runtime:

- `layer0/`: protocol traits + wire contract
- `neuron-turn/`: turn types + provider abstraction
- `neuron-tool/`: tool traits + `ToolRegistry`
- `neuron-hooks/`: hook registry + lifecycle hooks
- `neuron-context/`: prompt/context assembly helpers

Operators:

- `neuron-op-react/`: ReAct-style operator loop
- `neuron-op-single-shot/`: single-shot operator

Orchestration / environment:

- `neuron-orch-kit/`: orchestration building blocks
- `neuron-orch-local/`: local orchestrator implementation
- `neuron-env-local/`: local environment (process/tool execution glue)

State:

- `neuron-state-memory/`: in-memory state store
- `neuron-state-fs/`: filesystem-backed state store

Providers:

- `neuron-provider-openai/`
- `neuron-provider-anthropic/`
- `neuron-provider-ollama/`

Integration:

- `neuron-mcp/`: MCP client/server glue

Security building blocks:

- `neuron-secret/` and `neuron-secret-*`: secret store interfaces + backends
- `neuron-auth/` and `neuron-auth-*`: auth interfaces + backends
- `neuron-crypto/` and `neuron-crypto-*`: crypto interfaces + backends
- `neuron-hook-security/`: security-oriented hooks
