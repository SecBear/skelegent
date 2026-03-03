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

Core:

- `layer0/` — protocol traits + wire contract
- `neuron/` — umbrella crate

Turn (`turn/`):

- `neuron-turn` — turn types + provider abstraction
- `neuron-turn-kit` — turn decomposition primitives
- `neuron-context` — prompt/context assembly
- `neuron-tool` — tool traits + `ToolRegistry`
- `neuron-mcp` — MCP client/server

Operators (`op/`):

- `neuron-op-react` — ReAct-style operator loop
- `neuron-op-single-shot` — single-shot operator

Orchestration (`orch/`):

- `neuron-orch-kit` — composition building blocks
- `neuron-orch-local` — local orchestrator

Effects (`effects/`):

- `neuron-effects-core` — effect executor trait
- `neuron-effects-local` — local effect interpreter

Hooks (`hooks/`):

- `neuron-hooks` — hook registry + lifecycle hooks
- `neuron-hook-security` — security-oriented hooks

State (`state/`):

- `neuron-state-memory` — in-memory state store
- `neuron-state-fs` — filesystem-backed state store

Environment (`env/`):

- `neuron-env-local` — local environment (process/tool execution glue)

Providers (`provider/`):

- `neuron-provider-anthropic`
- `neuron-provider-openai`
- `neuron-provider-ollama`

Security (`secret/`, `auth/`, `crypto/`):

- `neuron-secret` + backends (env, vault, aws, gcp, keystore, k8s)
- `neuron-auth` + backends (static, file, oidc, k8s)
- `neuron-crypto` + backends (vault, hardware)