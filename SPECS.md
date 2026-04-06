# Skelegent Specifications

Authoritative specifications for Skelegent.

## How To Use

- `specs/v2/` is the current architecture. All implementation work targets these.
- v1 specs have been archived to `../skg-archived/specs/`.
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

## v1 Specs (archived)

All v1 specs have been moved to `../skg-archived/specs/`. Consult v2 specs for current architecture.
