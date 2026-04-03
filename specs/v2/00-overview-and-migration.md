# V2 Overview and Migration

## Purpose

Define the target `v2` kernel for Skelegent and the migration contract from the
current architecture.

V2 is not a product rewrite. It is a protocol purification pass whose goal is to
make future agent systems map into Skelegent as adapters and policies rather than
core redesigns.

## Preserved Invariants

The following v1 decisions remain load-bearing and are preserved:

- operators declare intent; outer layers execute it
- `DispatchContext` is the universal cross-boundary execution context
- `Dispatcher` remains the immediate invocation primitive
- durable run/control remains above immediate dispatch, not inside `Dispatcher`
- Layer 0 stays minimal, stable, and technology-agnostic

## V2 Goals

V2 is successful when the core architecture has:

- one semantic event plane
- one invocation model
- one native capability model
- stream-first execution
- typed outcomes and waits shared across immediate and durable control
- backend extensibility through trait families rather than monolithic protocol growth

## Non-Goals

V2 does not require in its first implementation wave:

- shipping a fully rewritten runtime
- standardizing backend internals such as checkpoint blobs or replay journals
- adding workflow DSLs
- public protocol surfaces for every possible deployment concern

## Adoption Policy

- Current numbered specs remain authoritative for shipped behavior.
- V2 specs are the target architecture for the next core implementation phase.
- The first implementation of v2 is a deliberate breaking cutover.
- When a v2 slice is adopted, the default action is to remove the superseded
  v1 public surface rather than carry deprecated compatibility adapters beside
  it.
- Private one-off refactor helpers are allowed during implementation, but they
  are migration scaffolding only and must not become part of the merged public
  kernel contract unless a v2 spec explicitly authorizes them.
- Any implementation PR adopting v2 behavior must cite the relevant v2 spec and
  update or retire conflicting current specs.

## Migration Matrix

| Current Spec | V2 Successor |
|---|---|
| `00-vision-and-non-goals` | `v2/00-overview-and-migration` + `v2/01-kernel-principles-and-layers` |
| `01-architecture-and-layering` | `v2/01-kernel-principles-and-layers` |
| `02-layer0-protocol-contract` | `v2/01`, `v2/02`, `v2/03`, `v2/04`, `v2/07`, `v2/08`, `v2/10` |
| `03-effects-and-execution-semantics` | `v2/03-intents-and-semantic-events` |
| `04-operator-turn-runtime` | `v2/02`, `v2/05`, `v2/06` |
| `05-orchestration-core` | `v2/02`, `v2/09`, `v2/10` |
| `06-composition-factory-and-glue` | preserved conceptually; implementation-facing follow-on after v2 core adoption |
| `07-state-core` | `v2/07-state-base-and-extension-families` |
| `08-environment-and-credentials` | `v2/08-content-environment-and-artifacts` |
| `09-hooks-lifecycle-and-governance` | partially preserved; follow-on adoption after event plane implementation |
| `10-secrets-auth-crypto` | preserved; no v2 change in this pack |
| `11-testing-examples-and-backpressure` | `v2/10-errors-versioning-and-conformance` |
| `12-packaging-versioning-and-umbrella-crate` | `v2/10-errors-versioning-and-conformance` for versioning only |
| `13-documentation-and-dx-parity` | preserved; documentation obligations apply to v2 pack |
| `14-durable-orchestration-core` | `v2/09-durable-alignment-and-control` |

## First Implementation Order

Implement v2 in this order:

1. split intents from observations
2. replace flat exits with typed outcomes and shared waits
3. make execution stream-first
4. promote existing scheduling/planning vocabulary into the runtime
5. add native capability discovery
6. split state into base plus extension families
7. align immediate and durable control on shared value types
8. add conformance and golden-trace coverage
