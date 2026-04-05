# Declarative Agent System Design

## Purpose

This document specifies an optional, in-repo composition layer — provisionally
`op/skg-system` — that lets users declare a complete agent stack in a
serializable manifest and compile it to the existing runtime primitives in the
Skelegent v2 workspace.

The layer does not extend Layer 0, does not change the `skelegent::agent()`
facade, and does not introduce scheduling, control-plane, or deployment
infrastructure. It is a composition utility: take a `SystemSpec`, validate it,
and wire it into the types that already exist. A developer who wants a
single-agent script keeps using `skelegent::agent()`. A team that needs named
models, multiple agent roles, shared memory backends, tool registries, and
environment policies in one coherent config file uses this layer.

The first implementation targets local composition only. Remote deployment is a
later concern and is explicitly out of scope here.

---

## Design Constraints

- **Do not add to Layer 0.** `SystemSpec` and its compiler are not public
  protocol types. They live exclusively in `op/skg-system`.
- **Do not expand `skelegent::agent()`.** That function remains a one-shot
  builder for a single operator. System-level concerns are not its job.
- **Compile-to-existing-primitives only.** Every field in `SystemSpec` maps to
  an already-existing runtime type. If no mapping exists, the field does not
  belong in v1.
- **Serializable manifest, not a DSL.** The spec is TOML/YAML/JSON — a data
  file. No macros, no proc-macros, no custom syntax.
- **Local first.** The v1 compiler wires everything in-process using `Arc`
  and existing orchestration crates. Remote placement and cross-process dispatch
  are post-v1 concerns.
- **No live secrets in the manifest.** Credential references describe where
  secrets live (`SecretSource`) and how they are injected (`CredentialInjection`).
  Actual secret values are never present in a `SystemSpec`.
- **Optional crate.** `op/skg-system` is not included in any default feature
  set. Users opt in explicitly.

---

## Compile Targets

The table below maps each declarative concept to the runtime primitive it
compiles to. A `SystemSpec` that contains only these concepts can be fully
materialized from already-shipped code.

| Declarative concept | Compiled runtime primitive |
|---|---|
| Named model / provider declaration | `RoutingProvider` + `ModelMapPolicy` (`skg-provider-router`) |
| Tool / resource declaration by reference | `ToolRegistry` + `Arc<dyn ToolDyn>` (`skg-tool`) |
| MCP server declaration | `CapabilitySource` + MCP client in `skg-mcp` |
| Memory backend declaration | `Arc<dyn StateStore>` + backend impl (`skg-state-memory`, `skg-state-fs`, `skg-state-proxy`) |
| Agent role declaration | `CognitiveBuilder` / `ReactLoopConfig` (`skg-context-engine`) |
| Workflow wiring | `WorkflowBuilder` → `Arc<dyn Operator>` (`skg-orch-patterns`) |
| Orchestration / execution | `OrchestratedRunner` + `Arc<dyn Dispatcher>` (`skg-orch-kit`, `skg-orch-local`) |
| Environment / isolation policy | `EnvironmentSpec` with `IsolationBoundary`, `ResourceLimits`, `NetworkPolicy` (`layer0`) |
| Credential binding | `CredentialRef` { `SecretSource`, `CredentialInjection` } (`layer0`) |
| Secret resolution | `SecretRegistry` with per-source `SecretResolver` impls (`skg-secret`) |
| Prompt / asset loading | File path resolved to `String` at compilation time |
| HTTP runtime serving | `OperatorRegistry` + `RunnerServiceImpl` (`skg-runner`) |

---

## Proposed IR Schema

The intermediate representation is a Rust struct hierarchy that is
`Deserialize`-able from TOML, YAML, or JSON. All field names use
`snake_case` by convention to match the serde defaults. The struct names below
are the proposed canonical names; they may be adjusted during implementation but
the shape should be preserved.

```rust
use layer0::{
    environment::{CredentialRef, EnvironmentSpec},
    secret::SecretSource,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level manifest. Deserialize from a `system.toml` / `system.yaml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemSpec {
    /// Metadata: name, version, description.
    pub meta: SystemMeta,

    /// Named model/provider declarations. Keys are logical names
    /// referenced by agent roles (e.g., "primary", "fast", "embed").
    #[serde(default)]
    pub models: HashMap<String, ModelSpec>,

    /// Named tool registries. Keys are logical names referenced by agent roles.
    #[serde(default)]
    pub tools: HashMap<String, ToolRegistrySpec>,

    /// Named memory/state backends. Keys are logical names referenced by
    /// agent roles.
    #[serde(default)]
    pub memory: HashMap<String, MemorySpec>,

    /// Named prompt assets. Keys are logical names; values are file paths
    /// resolved relative to the manifest file.
    #[serde(default)]
    pub prompts: HashMap<String, PromptSpec>,

    /// Agent role declarations.
    #[serde(default)]
    pub agents: HashMap<String, AgentSpec>,

    /// Workflow topology (how agents are wired into pipelines).
    #[serde(default)]
    pub workflows: HashMap<String, WorkflowSpec>,

    /// System-wide environment policy. Agent roles may override individual
    /// fields; overrides are merged, not replaced.
    #[serde(default)]
    pub environment: EnvironmentSpec,
}

/// Manifest metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMeta {
    /// Human-readable system name.
    pub name: String,
    /// Semver or free-form version string.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

/// A named model/provider declaration.
///
/// At compile time this maps to either a single provider or a
/// `RoutingProvider` if multiple `routes` are present.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    /// Provider tag: "anthropic", "openai", "ollama", or a custom tag
    /// that matches a registered provider factory.
    pub provider: String,
    /// Model identifier forwarded to the provider (e.g.,
    /// "claude-sonnet-4-20250514").
    pub model: String,
    /// Optional routing rules. When present, compiles to `RoutingProvider`
    /// + `ModelMapPolicy` with this spec as the default backend.
    #[serde(default)]
    pub routes: Vec<ModelRoute>,
    /// Credential reference for the provider's API key. The name must
    /// match a key in the system environment's `credentials` list.
    #[serde(default)]
    pub credential: Option<String>,
}

/// A single routing rule within a `ModelSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    /// Model name pattern to match (exact string, no glob in v1).
    pub model: String,
    /// Provider tag to delegate to when the pattern matches.
    pub provider: String,
}

/// A named collection of tools available to agents.
///
/// In v1, tools must be registered programmatically via the compilation
/// API; the manifest declares the registry name and (optionally) which
/// MCP servers to include. Statically-linked Rust tools are added in
/// code, not in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRegistrySpec {
    /// MCP server endpoints to include as tool sources.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerSpec>,
}

/// An MCP server to mount as a tool source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerSpec {
    /// Connection URL (e.g., "http://localhost:3001").
    pub url: String,
    /// Optional human-readable label.
    #[serde(default)]
    pub label: Option<String>,
}

/// A named state/memory backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum MemorySpec {
    /// In-process memory store (`skg-state-memory`). Not durable.
    Memory,
    /// Filesystem-backed store (`skg-state-fs`).
    Fs {
        /// Root directory for persisted state.
        root: String,
    },
    /// Remote proxy (`skg-state-proxy`).
    Proxy {
        /// Base URL of the remote state service.
        url: String,
        /// Credential name for authentication (optional).
        #[serde(default)]
        credential: Option<String>,
    },
}

/// A prompt/system-prompt asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptSpec {
    /// Inline prompt text.
    Inline {
        text: String,
    },
    /// Load from a file path (resolved relative to the manifest).
    File {
        path: String,
    },
}

/// An agent role declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// References a key in `SystemSpec::models`.
    pub model: String,
    /// References a key in `SystemSpec::prompts`. Optional; if absent,
    /// the agent has no system prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// References a key in `SystemSpec::tools`. Optional.
    #[serde(default)]
    pub tools: Option<String>,
    /// References a key in `SystemSpec::memory` to use as this agent's
    /// state store. Optional.
    #[serde(default)]
    pub memory: Option<String>,
    /// Maximum inference turns per invocation.
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Maximum output tokens per turn.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Per-agent environment overrides. Merged with system-level
    /// `EnvironmentSpec`; agent fields take precedence.
    #[serde(default)]
    pub environment: Option<EnvironmentSpec>,
}

/// A workflow topology declaration.
///
/// Compiles to a `WorkflowBuilder`-produced `Arc<dyn Operator>` registered
/// under the workflow's name in the system dispatcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    /// Ordered list of workflow steps.
    pub steps: Vec<WorkflowStep>,
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowStep {
    /// Sequential dispatch to a named agent.
    Agent {
        /// References a key in `SystemSpec::agents`.
        agent: String,
    },
    /// Fan-out to multiple agents in parallel, outputs merged by the
    /// default reducer.
    Parallel {
        /// Agent names to execute in parallel.
        agents: Vec<String>,
    },
}
```

---

## Compilation Pipeline

The compiler lives in `op/skg-system` as a `SystemCompiler` struct. It consumes
a `SystemSpec` and produces a `CompiledSystem` that holds all the wired-up
runtime primitives ready for use.

### Step 1: Validate `SystemSpec`

Before any allocation:

1. Every `AgentSpec::model` reference exists in `SystemSpec::models`.
2. Every `AgentSpec::system_prompt` reference exists in `SystemSpec::prompts`.
3. Every `AgentSpec::tools` reference exists in `SystemSpec::tools`.
4. Every `AgentSpec::memory` reference exists in `SystemSpec::memory`.
5. Every `WorkflowStep::Agent::agent` reference exists in `SystemSpec::agents`.
6. Every `WorkflowStep::Parallel::agents` entry exists in `SystemSpec::agents`.
7. No circular workflow references (linear only in v1; cycles are an error).
8. Credential names referenced in `ModelSpec::credential`,
   `MemorySpec::Proxy::credential`, and `AgentSpec::environment.credentials`
   exist in `SystemSpec::environment.credentials`.

Validation returns `SystemValidationError` with the full set of violations —
not just the first. This is intentional: a manifest with five broken references
should report all five.

### Step 2: Resolve prompts

For each `PromptSpec::File`, load the file relative to the manifest path. Fail
fast if any file is missing. The loaded text replaces the file reference in an
internal `ResolvedSystemSpec` type. `PromptSpec::Inline` is passed through
unchanged.

### Step 3: Build model/provider registry

For each `ModelSpec`:

1. Look up a registered `ProviderFactory` by the `provider` tag.
2. If `credential` is set, resolve the credential name to a `CredentialRef`
   in `EnvironmentSpec::credentials` and pass the resolved `SecretValue` to
   the factory. The `SecretValue` is scoped via `with_bytes` — the raw bytes
   do not escape the factory call.
3. If `routes` is non-empty, construct a `ModelMapPolicy` and wrap the
   primary provider in a `RoutingProvider` with routes added in order.
4. Store the resulting `Arc<dyn Provider>` under the logical model name.

### Step 4: Build state backends

For each `MemorySpec`:

1. `MemorySpec::Memory` → `Arc::new(InMemoryStateStore::new())`.
2. `MemorySpec::Fs { root }` → `Arc::new(FsStateStore::new(root))`.
3. `MemorySpec::Proxy { url, credential }` → resolve credential if present,
   construct `Arc::new(ProxyStateStore::new(url, ...))`.

### Step 5: Build tool registries

For each `ToolRegistrySpec`:

1. Construct a `ToolRegistry`.
2. For each `McpServerSpec`, connect to the MCP server and project its
   capability list into `Arc<dyn ToolDyn>` wrappers via the `skg-mcp` client.
   Fail if the server is unreachable.
3. Return the populated `ToolRegistry`.

Statically-linked tools are added after this step via the `SystemCompiler`
callback API (see extension path).

### Step 6: Build agent operators

For each `AgentSpec`, using the resolved providers, state backends, and tool
registries:

1. Select the `Arc<dyn Provider>` compiled in step 3 under `AgentSpec::model`.
2. Load the system prompt string (from resolved prompts in step 2, or empty).
3. Select the `ToolRegistry` (step 5) or an empty registry if `tools` is `None`.
4. Build a `ReactLoopConfig` from the agent's settings.
5. Construct a `CognitiveBuilder`, attach config, provider, tools.
6. If `memory` is set, attach the `Arc<dyn StateStore>` from step 4 via the
   appropriate context wiring (exact attachment point TBD — see open
   questions).
7. Merge the agent's `environment` override with the system-level
   `EnvironmentSpec`. Agent fields win on conflict.
8. Call `.build()` to produce `Box<dyn Operator>`.
9. Wrap in `Arc` and register in a `OperatorRegistry` under the agent's name.

### Step 7: Build workflows

For each `WorkflowSpec`:

1. Construct a `WorkflowBuilder` backed by the system's `Arc<dyn Dispatcher>`.
2. For each `WorkflowStep::Agent`, call `.step(OperatorId::new(agent))`.
3. For each `WorkflowStep::Parallel`, call `.parallel(operator_ids)`.
4. Call `.build()` to produce `Arc<dyn Operator>`.
5. Register the workflow operator in the `OperatorRegistry` under the
   workflow's name.

### Step 8: Assemble `CompiledSystem`

```rust
pub struct CompiledSystem {
    /// All named operators (agents + workflows) ready to dispatch.
    pub registry: Arc<OperatorRegistry>,
    /// The dispatcher that backs all operators.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// The effects handler wired to this system.
    pub effects: Arc<dyn EffectHandler>,
    /// An `OrchestratedRunner` pre-wired with `dispatcher` and `effects`.
    pub runner: OrchestratedRunner,
}
```

The caller can use `CompiledSystem::runner` directly, extract operators from
`registry`, or pass the `dispatcher` to their own orchestration layer.

---

## Crate Placement

**Crate path**: `op/skg-system`

**Crate name**: `skg-system`

**Why `op/`**: The `op/` directory holds operator-layer composition crates
(`skg-context-engine`, `skg-op-single-shot`). A system compiler that assembles
operators from a manifest belongs there. It is above Layer 0 and below the
umbrella `skelegent` crate.

**Not in `skelegent/` workspace defaults**: The `skelegent` umbrella crate
does not add `skg-system` as a default dependency. Users add it explicitly:

```toml
[dependencies]
skg-system = { path = "../op/skg-system", features = ["toml"] }
```

**Feature flags**:

| Feature | Effect |
|---|---|
| `toml` | Enables TOML manifest loading via `toml` crate |
| `yaml` | Enables YAML manifest loading via `serde_yaml` |
| `json` | Always on (serde_json is already a workspace dependency) |

**Direct dependencies** (no new workspace-external crates required for the
core):

- `layer0` (workspace)
- `skg-context-engine` (workspace)
- `skg-tool` (workspace)
- `skg-orch-kit` (workspace)
- `skg-orch-patterns` (workspace)
- `skg-orch-local` (workspace)
- `skg-provider-router` (workspace)
- `skg-secret` (workspace)
- `skg-state-memory` (workspace)
- `skg-state-fs` (workspace)
- `skg-state-proxy` (workspace)
- `serde` + `serde_json` (workspace)
- `toml` (optional, for `toml` feature)
- `serde_yaml` (optional, for `yaml` feature)
- `thiserror` (workspace)
- `async-trait` (workspace)

**Cargo.toml skeleton**:

```toml
[package]
name = "skg-system"
edition.workspace = true
license.workspace = true
description = "Declarative agent system manifest compiler for Skelegent."

[features]
default = ["json"]
json = []
toml = ["dep:toml"]
yaml = ["dep:serde_yaml"]

[dependencies]
layer0 = { workspace = true }
skg-context-engine = { workspace = true }
skg-tool = { workspace = true }
skg-orch-kit = { workspace = true }
skg-orch-patterns = { workspace = true }
skg-orch-local = { workspace = true }
skg-provider-router = { workspace = true }
skg-secret = { workspace = true }
skg-state-memory = { workspace = true }
skg-state-fs = { workspace = true }
skg-state-proxy = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
async-trait = { workspace = true }
toml = { version = "0.8", optional = true }
serde_yaml = { version = "0.9", optional = true }
```

---

## v1 Scope and Non-Goals

### Ships in v1

- `SystemSpec` struct hierarchy as sketched in the IR schema section.
- `SystemCompiler` with the eight-step pipeline above.
- `CompiledSystem` output struct.
- `SystemValidationError` with all-violations collection.
- TOML and JSON manifest loading.
- Local in-process compilation only.
- Support for: models, tool registries (MCP only in manifest; static tools via
  callback), memory backends, agent roles, linear and parallel workflow steps,
  environment policy, credential binding.

### Does not ship in v1

- No Layer 0 workflow trait. `WorkflowSpec` compiles to `WorkflowBuilder`, which
  uses the existing `Operator` trait. No new protocol surface.
- No universal deployment DSL. `SystemSpec` is a local config file, not a
  cross-environment deployment manifest.
- No scheduling assumptions. `OrchestratedRunner` handles execution; there is no
  scheduler, queue, or control plane in this crate.
- No Nix module. Nix is one deployment frontend and can generate or load a
  `SystemSpec` independently. This crate does not know about Nix.
- No live secrets in manifests. `CredentialRef` references where secrets live.
  The actual resolution happens at compilation time via `SecretRegistry`.
- No remote operator dispatch. `WorkflowSpec` workflows only reference agents
  in the same manifest. Cross-system agent references are post-v1.
- No hot-reload. The manifest is loaded once at startup and compiled to a
  `CompiledSystem`. A changed manifest requires a restart.
- No YAML in v1 (defer until there is a concrete user need). TOML and JSON cover
  the initial use case.
- No `skg-runner` integration in the `SystemCompiler` itself. A compiled
  `OperatorRegistry` can be passed directly to `RunnerServiceImpl`, but
  `skg-system` does not depend on `skg-runner`. That wiring lives at the
  application layer.

---

## Extension Path

The v1 design has explicit seams for the following post-v1 work:

**Remote deployment manifest.** `SystemSpec` can grow a `deployment` section
describing target environments (k8s namespace, EC2 region, etc.) without
changing the compilation pipeline. The compiler delegates the deployment section
to a separate `DeploymentDriver` trait, not wired in v1.

**Nix module.** A Nix module can consume `system.toml` directly (it is a data
file) or generate it from Nix attribute sets. No code changes needed in
`skg-system` to support this — the module ships separately.

**Hot-reload.** A `SystemWatcher` wrapper around `SystemCompiler` can watch the
manifest file for changes, recompile, and swap the `Arc<OperatorRegistry>` under
a `RwLock`. The `CompiledSystem` struct already holds everything needed.

**Scheduling / control plane.** If a scheduler needs to route work to agents, it
can read from `CompiledSystem::registry`. `skg-system` does not need to change.

**Cross-system agent references.** `WorkflowStep::Agent` currently resolves
only to agents in the same manifest. A `WorkflowStep::Remote` variant could
reference an operator by URL, delegating to an `skg-orch-env` dispatcher.

**Static tool registration callback.** The v1 compiler accepts a
`FnOnce(&mut ToolRegistry, &str)` callback per named tool registry, called
after MCP tools are loaded. This is the primary extension point for adding
compiled-in tools to declarative registries. Formalize the callback signature
before v1 ships.

---

## Open Questions

**1. Memory attachment to `CognitiveBuilder`**

`CognitiveBuilder` / `ReactLoopConfig` do not currently have a field for an
`Arc<dyn StateStore>`. Memory is injected via context-engine internals that are
not exposed at the `AgentBuilder` level. Before v1 can compile `AgentSpec::memory`,
the context-engine API must expose a stable setter (e.g.,
`CognitiveBuilder::state_store(Arc<dyn StateStore>)`). Decision needed: does
this land in `skg-context-engine` as part of this work, or is `memory` deferred
to v2 of this layer?

**2. `ProviderFactory` registry**

Step 3 of the compilation pipeline requires a `ProviderFactory` lookup by tag
string. No such registry exists today. Options:
- (a) Compile-time match on known provider tags (`"anthropic"`, `"openai"`,
  `"ollama"`) with `#[cfg(feature = "...")]` guards — matches how
  `skelegent::agent()` resolves models today. Simple, but not extensible.
- (b) A `ProviderRegistry` that maps tag → `Arc<dyn ProviderFactory>`, populated
  by the caller before compilation. Extensible, but adds indirection.

Option (a) covers all v1 use cases. Decision: use (a) for v1, design (b) as the
extension path?

**3. Credential resolution timing**

The compilation pipeline resolves credentials in step 3 (model providers) and
step 4 (state backends). This means `SecretRegistry` must be provided to
`SystemCompiler::compile()` at call time — the compiler is async. Is it
acceptable for compilation to be an `async fn`? The alternative is
lazy resolution (credentials are resolved at first use), but that defers errors
out of the compilation step, which is worse. Confirm: compilation is async and
calls `SecretRegistry::resolve()` eagerly.

**4. `EnvironmentSpec` merge semantics**

Step 6 says "agent fields win on conflict" when merging system-level and
per-agent `EnvironmentSpec`. The exact merge rules need to be nailed down before
implementation:

- `isolation`: concatenate or replace? (Suggestion: replace, since isolation
  layers are ordered and the agent should own the full stack.)
- `credentials`: union or agent-only? (Suggestion: union — the agent inherits
  system credentials and can add its own.)
- `resources`: agent override wins entirely or per-field? (Suggestion: per-field,
  so an agent can increase `memory` without losing the system `cpu` limit.)
- `network`: replace entirely. Network policy is security-sensitive; partial
  merges produce hard-to-reason-about results.

**5. Manifest file location and relative path resolution**

`PromptSpec::File` paths and `MemorySpec::Fs::root` are resolved relative to
the manifest file. The API must communicate the manifest's parent directory to
the compiler. Two options:
- (a) `SystemCompiler::from_file(path)` — loads the manifest and records the
  base directory. The base directory is part of compiler state.
- (b) `SystemCompiler::compile(spec, base_dir)` — caller supplies the base dir.
  Allows loading the manifest independently of compilation.

Option (b) is more composable (the caller can deserialize the manifest from any
source — env var, remote config service, test fixture — and supply the base
path separately). Decision: go with (b)?
