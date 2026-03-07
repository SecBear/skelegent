# Core Concepts

neuron's architecture is built on **four protocol traits** and **two cross-cutting interfaces**, organized into **six layers**. This page explains each concept and how they compose.

## The four protocols

Every agentic system must answer four questions. Each question maps to a protocol trait in Layer 0.

### Protocol 1: Operator -- "What does one agent do per cycle?"

The `Operator` trait defines the boundary around a single agent's execution cycle. Input goes in, the agent reasons (model calls) and acts (tool execution), and output comes out.

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError>;
}
```

The trait is intentionally one method. From the outside, an operator is atomic -- you do not care whether it made 1 model call or 20, whether it used tools or not, or what context strategy it used. Those are implementation details.

Implementations include `ReactOperator` (the ReAct reasoning loop with tools) and `SingleShotOperator` (one model call, no tools).

### Protocol 2: Orchestrator -- "How do agents compose?"

The `Orchestrator` trait defines how multiple agents work together and how execution survives failures.

```rust
#[async_trait]
pub trait Orchestrator: Send + Sync {
    async fn dispatch(&self, agent: &AgentId, input: OperatorInput)
        -> Result<OperatorOutput, OrchError>;
    async fn dispatch_many(&self, tasks: Vec<(AgentId, OperatorInput)>)
        -> Vec<Result<OperatorOutput, OrchError>>;
    async fn signal(&self, target: &WorkflowId, signal: SignalPayload)
        -> Result<(), OrchError>;
    async fn query(&self, target: &WorkflowId, query: QueryPayload)
        -> Result<serde_json::Value, OrchError>;
}
```

`dispatch` might be a function call (in-process) or a network hop to another continent. The caller does not know and does not care. `signal` provides fire-and-forget messaging to running workflows. `query` enables read-only inspection of workflow state.

### Protocol 3: StateStore -- "How does data persist?"

The `StateStore` trait provides scoped key-value persistence with optional semantic search.

```rust
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn read(&self, scope: &Scope, key: &str)
        -> Result<Option<serde_json::Value>, StateError>;
    async fn write(&self, scope: &Scope, key: &str, value: serde_json::Value)
        -> Result<(), StateError>;
    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError>;
    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError>;
    async fn search(&self, scope: &Scope, query: &str, limit: usize)
        -> Result<Vec<SearchResult>, StateError>;
}
```

Values are `serde_json::Value`, which provides schema flexibility without sacrificing serializability. Scopes partition data (per-agent, per-session, per-workflow). Implementations include `MemoryStore` (in-memory `HashMap`, good for tests) and `FsStore` (filesystem-backed, durable).

A read-only projection, `StateReader`, is provided to operators during context assembly. Operators can read state but must declare writes as effects -- they never write directly.

### Protocol 4: Environment -- "Where does the agent run?"

The `Environment` trait mediates execution within an isolation boundary.

```rust
#[async_trait]
pub trait Environment: Send + Sync {
    async fn run(&self, input: OperatorInput, spec: &EnvironmentSpec)
        -> Result<OperatorOutput, EnvError>;
}
```

The `EnvironmentSpec` declares isolation boundaries (process, container, VM, Wasm), credential injection, resource limits, and network policy. `LocalEnv` passes through with no isolation (for development). Future implementations could spin up containers or Kubernetes pods.

## The two interfaces

### Hook -- Observation and intervention

Hooks fire at defined points inside the operator's inner loop: before/after inference, before/after tool use, and at exit-condition checks.

```rust
#[async_trait]
pub trait Hook: Send + Sync {
    fn points(&self) -> &[HookPoint];
    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError>;
}
```

A hook can observe (logging, telemetry), intervene (halt execution, skip a sub-dispatch), or modify (sanitize dispatch input, redact dispatch output). Hook errors are logged but do not halt execution -- use `HookAction::Halt` to halt.

### Lifecycle -- Cross-layer coordination

Lifecycle events (`BudgetEvent`, `CompactionEvent`) coordinate concerns that span multiple protocols. A budget event might originate from a hook (observing cost) and propagate to the orchestrator (to cancel the workflow). A compaction event coordinates between the operator and the state store.

## How layers compose

The six layers form a strict dependency hierarchy:

```
Layer 5  Cross-Cutting    (hooks, lifecycle)
Layer 4  Environment      (isolation, credentials)
Layer 3  State            (persistence)
Layer 2  Orchestration    (multi-agent composition)
Layer 1  Operator impls   (providers, tools, operators, MCP)
Layer 0  Protocol traits  (the stability contract)
```

Higher layers depend on lower layers, never the reverse. Layer 0 has no knowledge of any implementation. A Layer 1 crate depends on Layer 0 for trait definitions but knows nothing about orchestration or state backends.

This means you can replace any layer's implementation without touching other layers. Swap `MemoryStore` for a hypothetical `PostgresStore` and nothing in your operator code changes. Swap `LocalOrch` for a Temporal-backed orchestrator and your operators, tools, and state stores remain identical.

## The composition pattern

A typical application composes the layers like this:

```
Operator   = ReactOperator<AnthropicProvider> + ToolRegistry + HookRegistry
State      = FsStore (filesystem persistence)
Env        = LocalEnv (no isolation, dev mode)
Orchestr.  = LocalOrch { agent_a -> Operator, agent_b -> Operator }
```

Each component is constructed independently, then composed through trait objects. The orchestrator holds `Arc<dyn Operator>` references. The environment holds its own operator reference. Nothing knows about concrete types beyond its own construction site.


## Tools and agents

These terms name configuration patterns built on top of `Operator`, not separate types.

**Tool:** An operator registered with `ToolMetadata` (name, description, JSON input schema, concurrency hint). The metadata makes the operator callable from an LLM reasoning loop. The distinction between a tool and any other operator is configuration, not type — the `Operator` trait is the same.

**Agent:** A configured operator. Concretely: an `Operator` implementation (typically `ReactOperator`) wired with a provider, identity, tools, and optionally an `Arc<dyn Orchestrator>` for sub-dispatching to other agents. The term 'agent' has no corresponding trait; it describes how an operator is assembled and what capabilities it receives at construction time.