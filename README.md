# Skelegent — composable agentic runtime

Skelegent is an experiment in building an agentic system that is **composable by construction**:
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
- `skelegent/` — umbrella crate

Turn (`turn/`):

- `skg-turn` — turn types + provider abstraction
- `skg-turn-kit` — turn decomposition primitives
- `skg-context` — prompt/context assembly
- `skg-tool` — tool traits + `ToolRegistry`
- `skg-mcp` — MCP client/server

Operators (`op/`):

- `skg-context-engine` — ReAct-style operator loop
- `skg-op-single-shot` — single-shot operator

Orchestration (`orch/`):

- `skg-orch-kit` — composition building blocks
- `skg-orch-local` — local orchestrator

Effects (`effects/`):

- `skg-effects-core` — effect executor trait
- `skg-effects-local` — local effect interpreter

Middleware (`hooks/`):

- `skg-hook-security` — security middleware (RedactionMiddleware, ExfilGuardMiddleware)

State (`state/`):

- `skg-state-memory` — in-memory state store
- `skg-state-fs` — filesystem-backed state store

Environment (`env/`):

- `skg-env-local` — local environment (process/tool execution glue)

Providers (`provider/`):

- `skg-provider-anthropic`
- `skg-provider-openai`
- `skg-provider-ollama`

Security (`secret/`, `auth/`, `crypto/`):

- `skg-secret` — secret resolution
- `skg-secret-vault` — HashiCorp Vault backend
- `skg-auth` — auth/credential framework
- `skg-crypto` — cryptographic primitives

## Implementations

Heavy-dependency implementations — SQLite, CozoDB, Temporal, Git effects, sweep
operators, and auth providers — live in a separate repository to keep this core
dependency-free:

[**skelegent-extras**](https://github.com/SecBear/skelegent-extras) — provider ecosystem