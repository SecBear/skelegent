# Neuron Specifications

This directory contains the functional and technical specifications for the Neuron redesign (`redesign/v2`).

These specs are written to ensure the project is *composable by construction*: once the core is complete, the only remaining work to build a fully fledged local or distributed agentic system should be (1) technology-specific implementations (Temporal, Docker/K8s, Postgres, Vault, etc.) and (2) thin glue/configuration.

## How To Use These Specs

- Treat `specs/` as the durable source of truth for intended behavior.
- Each spec states what is already implemented in this repo and what is still required.
- Tests and examples should be added to prove each spec section.

## Index

| Spec | Domain | Summary |
|------|--------|---------|
| `specs/00-vision-and-non-goals.md` | Product/Philosophy | What Neuron is for, and what it is not |
| `specs/01-architecture-and-layering.md` | Architecture | Layering model and responsibility boundaries |
| `specs/02-layer0-protocol-contract.md` | Layer0 | Protocol traits, wire types, compatibility rules |
| `specs/03-effects-and-execution-semantics.md` | Runtime | Effect vocabulary and required execution semantics |
| `specs/04-operator-turn-runtime.md` | Turn | Operator/turn behavior, metadata, tool loop requirements |
| `specs/05-orchestration-core.md` | Orchestration | Dispatch, topology, durability boundary, workflow control |
| `specs/06-composition-factory-and-glue.md` | Composition | Where composition/glue belongs and required factory APIs |
| `specs/07-state-core.md` | State | State store semantics, scopes, search, compaction coordination |
| `specs/08-environment-and-credentials.md` | Environment | Isolation, credential injection, resource/network policy |
| `specs/09-hooks-lifecycle-and-governance.md` | Governance | Hooks, lifecycle vocab, intervention semantics |
| `specs/10-secrets-auth-crypto.md` | Security | Secret/auth/crypto abstractions and integration points |
| `specs/11-testing-examples-and-backpressure.md` | DX/Quality | Example suite, test matrix, mock/real path strategy |
| `specs/12-packaging-versioning-and-umbrella-crate.md` | Release | Crate naming, umbrella crate, feature flags, publishing |
| `specs/13-documentation-and-dx-parity.md` | Docs/DX | Documentation requirements and parity targets |
