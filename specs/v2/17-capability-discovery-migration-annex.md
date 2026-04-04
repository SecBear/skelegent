# Capability Discovery Migration Annex

## Purpose

Define the third implementation-grade `v2` migration slice for native capability
discovery:

- add `CapabilitySource` as the canonical discovery surface
- make `CapabilityDescriptor` the single discovery payload model
- keep discovery as a sibling to `Dispatcher`, not a method on it
- replace `ToolMetadata`-centric discovery assumptions with descriptor-first
  discovery for tools, prompts, resources, agents, and services

This annex is implementation-authoritative for the `v2` capability-discovery
cutover slice once a future change adopts the `v2` track.

## Migration Posture

This slice assumes a deliberate breaking cutover.

Rules:

- the merged public kernel surface MUST expose `CapabilitySource` and
  `CapabilityDescriptor` as the discovery model
- superseded `v1` discovery nouns are removed rather than retained as public
  compatibility shims
- canonical `v2` descriptor serde behavior does not have to accept old
  `ToolMetadata` wire forms
- private migration helpers are allowed during branch refactor but MUST be
  removed before merge

## Scope

This slice is intentionally narrow.

It covers:

- capability discovery trait and descriptor wire contract
- descriptor scheduling/approval/auth fact vocabulary
- projection rules from existing tool registries and MCP discovery
- query/read-model expectations for capability inspection

It does not cover:

- invocation semantics (`v2/15`)
- intent/event model (`v2/16`)
- runtime planner internals beyond descriptor facts
- durable lifecycle control surfaces

## Ownership

| Crate | Module | Owns in this slice |
|---|---|---|
| `layer0` | `src/capability.rs` | `CapabilitySource`, `CapabilityDescriptor`, `CapabilityFilter`, supporting enums/structs |
| `layer0` | `src/lib.rs` | re-export capability discovery nouns; removal of `ToolMetadata` as canonical discovery noun |
| `layer0` | `src/operator.rs` | removal of `ToolMetadata` from the canonical Layer 0 discovery contract |
| `turn/skg-tool` | `src/lib.rs` and `src/adapter.rs` | projection from tool registry/tool dyn abstractions into tool-kind `CapabilityDescriptor` |
| `turn/skg-mcp` | `src/client.rs` and `src/server.rs` | projection of MCP tools/prompts/resources into `CapabilityDescriptor`; discovery served through `CapabilitySource` |
| `orch/*` and runtime crates | relevant registration/discovery modules | consumption of descriptor facts for planning/approval policy decisions |

## Canonical Public Types

### Capability Source

`CapabilitySource` is a read-only sibling to invocation.

```rust
#[async_trait]
pub trait CapabilitySource: Send + Sync {
    async fn list(
        &self,
        filter: CapabilityFilter,
    ) -> Result<Vec<CapabilityDescriptor>, ProtocolError>;

    async fn get(
        &self,
        id: &CapabilityId,
    ) -> Result<Option<CapabilityDescriptor>, ProtocolError>;
}
```

Rules:

- discovery is read-only in the kernel contract
- registration/mutation remains implementation detail
- `Dispatcher` remains invocation-only and MUST NOT grow discovery methods

### Descriptor Family

`CapabilityDescriptor` is the canonical discovery payload.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityId(pub String);

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Prompt,
    Resource,
    Agent,
    Service,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub accepts: Vec<CapabilityModality>,
    #[serde(default)]
    pub produces: Vec<CapabilityModality>,
    pub streaming: StreamingSupport,
    pub scheduling: SchedulingFacts,
    pub approval: ApprovalFacts,
    pub auth: AuthFacts,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityModality {
    Text,
    Json,
    Binary,
    Structured,
    Custom(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingSupport {
    None,
    Output,
    Bidirectional,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    Shared,
    Exclusive,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulingFacts {
    pub execution_class: ExecutionClass,
    pub ordering_sensitive: bool,
    pub idempotent: bool,
    pub interruptible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalFacts {
    None,
    Always,
    RuntimePolicy,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AuthFacts {
    Open,
    Caller,
    Service {
        #[serde(default)]
        scopes: Vec<String>,
    },
    Custom {
        scheme: String,
    },
}

#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<CapabilityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}
```

Rules:

- descriptors carry facts, not planner policy
- scheduling and approval facts are serializable
- runtime-only closures or predicates are not part of descriptor wire shape

## Exact Replacement Rules

### Removed Public Discovery Surfaces

The following replacements are locked:

| Remove | Replace with |
|---|---|
| `ToolMetadata` as canonical discovery payload | tool-kind `CapabilityDescriptor` |
| `ToolMetadata.parallel_safe` | `SchedulingFacts.execution_class` plus optional `max_concurrency` |
| non-serializable discovery policy closures in metadata | `ApprovalFacts` / `AuthFacts` serializable facts |
| discovery assumptions tied only to tools | one descriptor family across tools, prompts, resources, agents, and services |

No public type alias may preserve `ToolMetadata` as the canonical kernel
discovery noun in merged `v2`.

### Dispatcher / Discovery Boundary

Boundary is locked:

- `Dispatcher` is invocation-only
- `CapabilitySource` is discovery-only
- no method such as `Dispatcher::list_capabilities(...)` may be introduced

### Current Tool Metadata Projection

When refactoring current tool registries, project current metadata with this
exact mapping:

| Current `ToolMetadata` field | Tool-kind descriptor field |
|---|---|
| `name` | `id` (stable local naming convention) and `name` |
| `description` | `description` |
| `input_schema` | `input_schema` |
| `output_schema` | `output_schema` |
| `parallel_safe = true` | `scheduling.execution_class = Shared` |
| `parallel_safe = false` | `scheduling.execution_class = Exclusive` |
| `approval = None` | `approval = None` |
| `approval = Always` | `approval = Always` |
| `approval = Conditional(_)` | `approval = RuntimePolicy` |

## Bridge Projection Rules

Projection obligations are locked:

- static in-process registries must project to canonical descriptors
- MCP discovery results must project to canonical descriptors
- bridge-specific wire/protocol details belong in `extensions`, not in new
  parallel semantic descriptor models

Minimum kind mapping for MCP:

- MCP tool -> `CapabilityKind::Tool`
- MCP prompt -> `CapabilityKind::Prompt`
- MCP resource -> `CapabilityKind::Resource`

## Query and Runtime Consumption Rules

Capability query surfaces above Layer 0 MUST use `CapabilitySource` and
descriptor projections.

Runtime and orchestration components may consume descriptor facts for:

- batching and barrier planning (`SchedulingFacts`)
- approval gating (`ApprovalFacts`)
- transport and invocation preparation (`StreamingSupport`, `AuthFacts`)

Policy decisions remain above the descriptor contract.

## Wire and Serde Rules

Canonical writers MUST emit:

- `CapabilityDescriptor.kind`
- `CapabilityDescriptor.scheduling`
- `CapabilityDescriptor.approval`
- `CapabilityDescriptor.auth`

Because this slice is a breaking cutover:

- canonical `v2` deserializers are only required to accept canonical
  descriptor/filter shapes
- old `ToolMetadata` wire forms are not part of the merged `v2` serde contract
- pre-`v2` data import belongs in migration tooling outside the canonical kernel
  contract

## No Public Shim Rules

The merged `v2` surface MUST NOT include:

- `ToolMetadata` as the canonical public discovery type in `layer0`
- public discovery methods on `Dispatcher`
- duplicate discovery vocabularies per bridge protocol as parallel semantic
  models
- non-serializable closure fields inside canonical capability descriptors

## Proving Tests

The implementation PR adopting this annex MUST add all of the following.

### `layer0`

- `layer0/tests/v2_capability_descriptor_wire.rs`
  - all descriptor enums/structs round-trip with locked wire shapes
  - filter semantics serialize/deserialize as specified
- `layer0/tests/v2_dispatcher_discovery_boundary.rs`
  - `Dispatcher` remains invocation-only
  - `CapabilitySource` remains discovery-only

### `turn/skg-tool`

- `turn/skg-tool/tests/v2_tool_descriptor_projection.rs`
  - current tool metadata projects to tool-kind descriptor fields per locked
    mapping table
  - `parallel_safe` and approval policy map correctly to scheduling/approval
    facts

### `turn/skg-mcp`

- `turn/skg-mcp/tests/v2_mcp_descriptor_projection.rs`
  - MCP tool/prompt/resource discovery projects into canonical descriptor kinds
  - MCP-specific extras land in `extensions` without introducing alternate
    semantic descriptor models

## Golden Fixtures

The implementation PR adopting this annex MUST add JSON fixtures at these exact
paths:

- `layer0/tests/golden/v2/capability-descriptor-tool.json`
- `layer0/tests/golden/v2/capability-descriptor-resource.json`
- `layer0/tests/golden/v2/capability-filter.json`
- `turn/skg-mcp/tests/golden/v2/mcp-tool-descriptor.json`
- `turn/skg-mcp/tests/golden/v2/mcp-prompt-descriptor.json`
- `turn/skg-mcp/tests/golden/v2/mcp-resource-descriptor.json`

Fixture rules:

- fixtures MUST use canonical `v2` descriptor field names
- fixtures MUST NOT encode legacy `ToolMetadata` as canonical descriptor output

## Explicit Non-Goals

This annex does not authorize:

- making discovery writable in Layer 0
- forcing one registration backend or registry implementation
- embedding planner policy programs inside descriptors
- changing invocation semantics from `v2/15`
- changing intent/event semantics from `v2/16`

## Relationship to Existing Specs

This annex refines:

- `specs/v2/04-capability-sources-and-descriptors.md`
- `specs/v2/06-scheduling-and-turn-execution.md`
- `specs/v2/12-observation-intervention-and-queries.md` (query surfaces only)

For the `v2` track it supersedes:

- tool-metadata-centric discovery assumptions in
  `specs/02-layer0-protocol-contract.md`
- tool-only discovery assumptions in `specs/04-operator-turn-runtime.md`
