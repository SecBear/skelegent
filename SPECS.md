# Skelegent Specifications

This directory contains the functional and technical specifications for Skelegent.

These specs are written to ensure the project is *composable by construction*: once the core is complete, the only remaining work to build a fully fledged local or distributed agentic system should be (1) technology-specific implementations (Temporal, Docker/K8s, Postgres, Vault, etc.) and (2) thin glue/configuration.

## How To Use These Specs

- `specs/v2/` is the current architecture. Prefer these for implemented domains.
- `specs/` (top-level) contains v1 specs; those marked RETIRED are superseded by v2.
- Each spec states what is already implemented in this repo and what is still required.
- Tests and examples should be added to prove each spec section.

## v2 Specs (current)

| Spec | Domain | Summary |
|------|--------|---------|
| `specs/v2/00-overview-and-migration.md` | Overview | v2 design overview and migration guide |
| `specs/v2/01-kernel-principles-and-layers.md` | Architecture | Kernel principles and layer model |
| `specs/v2/02-invocation-outcomes-and-waits.md` | Runtime | Outcome types, wait semantics, priority ordering |
| `specs/v2/03-intents-and-semantic-events.md` | Runtime | Intent vocabulary and ExecutionEvent semantics |
| `specs/v2/04-capability-sources-and-descriptors.md` | Discovery | CapabilitySource, CapabilityDescriptor |
| `specs/v2/05-streaming-runtime-and-provider-projection.md` | Streaming | Provider chunk projection into ExecutionEvents |
| `specs/v2/06-scheduling-and-turn-execution.md` | Turn | Turn scheduling and execution model |
| `specs/v2/07-state-base-and-extension-families.md` | State | State store semantics and extension families |
| `specs/v2/08-content-environment-and-artifacts.md` | Environment | Content model, environment, artifacts |
| `specs/v2/09-durable-alignment-and-control.md` | Orchestration | Durable run/control semantics |
| `specs/v2/10-errors-versioning-and-conformance.md` | Protocol | ProtocolError, versioning, conformance |
| `specs/v2/11-session-state-memory-and-context.md` | State | Session state, memory tiers, context |
| `specs/v2/12-observation-intervention-and-queries.md` | Observation | Observer patterns, intervention, queries |
| `specs/v2/13-composition-patterns-and-control-surfaces.md` | Composition | Composition primitives and control surfaces |
| `specs/v2/14-lifecycle-policies-compaction-and-governance.md` | Governance | Lifecycle policies, compaction, governance |
| `specs/v2/15-layer0-invocation-outcome-migration-annex.md` | Migration | Layer0 Outcome migration annex |
| `specs/v2/16-intent-event-migration-annex.md` | Migration | Intent/ExecutionEvent migration annex |
| `specs/v2/17-capability-discovery-migration-annex.md` | Migration | Capability discovery migration annex |

## v1 Specs (partially retired)

Specs marked **RETIRED** are superseded by `specs/v2/`. Do not use them for new implementation work. Active specs (06, 09-13) remain valid where v2 has no equivalent.

| Spec | Domain | Status | Summary |
|------|--------|--------|---------|
| `specs/00-vision-and-non-goals.md` | Product/Philosophy | RETIRED | What Skelegent is for, and what it is not |
| `specs/01-architecture-and-layering.md` | Architecture | RETIRED | Layering model and responsibility boundaries |
| `specs/02-layer0-protocol-contract.md` | Layer0 | RETIRED | Protocol traits, wire types, compatibility rules |
| `specs/03-effects-and-execution-semantics.md` | Runtime | RETIRED | Effect vocabulary and required execution semantics |
| `specs/04-operator-turn-runtime.md` | Turn | RETIRED | Operator/turn behavior, metadata, tool loop requirements |
| `specs/05-orchestration-core.md` | Orchestration | RETIRED | Dispatch, topology, durability boundary, workflow control |
| `specs/06-composition-factory-and-glue.md` | Composition | Active | Where composition/glue belongs and required factory APIs |
| `specs/07-state-core.md` | State | RETIRED | State store semantics, scopes, search, and message-level compaction hints |
| `specs/08-environment-and-credentials.md` | Environment | RETIRED | Isolation, credential injection, resource/network policy |
| `specs/09-hooks-lifecycle-and-governance.md` | Governance | Active | Hooks, lifecycle vocab, intervention semantics |
| `specs/10-secrets-auth-crypto.md` | Security | Active | Secret/auth/crypto abstractions and integration points |
| `specs/11-testing-examples-and-backpressure.md` | DX/Quality | Active | Example suite, test matrix, mock/real path strategy |
| `specs/12-packaging-versioning-and-umbrella-crate.md` | Release | Active | Crate naming, umbrella crate, feature flags, publishing |
| `specs/13-documentation-and-dx-parity.md` | Docs/DX | Active | Documentation requirements and parity targets |
| `specs/14-durable-orchestration-core.md` | Orchestration | RETIRED | Portable durable run/control semantics above Layer 0 |
