# Capability Sources and Descriptors

## Purpose

Define native discovery for tools, prompts, resources, agents, and services
without overloading `Dispatcher`.

## CapabilitySource

V2 adds a sibling discovery protocol:

```rust
pub trait CapabilitySource: Send + Sync {
    async fn list(
        &self,
        filter: Option<CapabilityFilter>,
    ) -> Result<Vec<CapabilityDescriptor>, CapabilityError>;

    async fn get(
        &self,
        id: &CapabilityId,
    ) -> Result<Option<CapabilityDescriptor>, CapabilityError>;
}
```

This surface is read-only. Mutation and registration remain implementation details.

## CapabilityDescriptor

Every discoverable capability must project into one native descriptor shape.

Required fields:

- stable ID
- human-readable name and description
- capability kind
- input/output schema where relevant
- accepted and produced modalities/content types
- streaming support
- scheduling hints
- approval requirements
- auth/security metadata
- extension bag for protocol- or domain-specific extras

Capability kinds are:

- `tool`
- `prompt`
- `resource`
- `agent`
- `service`

## Scheduling Hints

Descriptors, not hard-coded runtime tables, carry scheduling-relevant facts such as:

- shared vs exclusive execution class
- ordering sensitivity
- idempotence
- interruptibility
- bounded concurrency hints

These are facts about the capability. Planners decide how to use them.

## Bridge Rule

MCP, A2A, and internal registries must translate into `CapabilityDescriptor`
instead of maintaining separate native semantic models.

Bridge-specific wire differences are allowed. Semantic duplication is not.

## Relationship to Current Specs

This spec supersedes the tool- and operator-metadata portions of
`specs/02-layer0-protocol-contract.md` and the tool registration assumptions in
`specs/04-operator-turn-runtime.md` for the v2 track.

## Minimum Proving Tests

- A static registry, MCP source, and A2A source all project into the same descriptor family.
- Scheduling and approval hints survive projection without protocol-specific branching.
