# Neuron — composable agentic runtime

Neuron is an experiment in building an agentic system that is **composable by construction**:
layered protocol contracts, swappable providers/tools/state, and deterministic backpressure via
tests and specs.

Specs are the source of truth: `SPECS.md` and `specs/`.

## Quickstart (Nix)

This repo assumes Rust tooling is provided by the Nix flake.

- Full verification: `./scripts/verify.sh`
- Canonical commands: see `AGENTS.md §Verification`

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

Middleware (`hooks/`):

- `neuron-hook-security` — security middleware (RedactionMiddleware, ExfilGuardMiddleware)

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

- `neuron-secret` — secret resolution
- `neuron-secret-vault` — HashiCorp Vault backend
- `neuron-auth` — auth/credential framework
- `neuron-crypto` — cryptographic primitives

## Implementations

Heavy-dependency implementations — SQLite, CozoDB, Temporal, Git effects, sweep
operators, and auth providers — live in a separate repository to keep this core
dependency-free:

[**neuron-extras**](https://github.com/SecBear/neuron-extras) — provider ecosystem