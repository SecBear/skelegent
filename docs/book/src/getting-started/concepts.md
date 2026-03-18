# Core Concepts

skelegent's architecture is built on **four Layer 0 protocol traits** (plus `Signalable` and `Queryable` at Layer 2) and **two cross-cutting interfaces**, organized into **six layers**. This page explains each concept and how they compose.

## Protocol traits

Every agentic system must answer these questions. Each question maps to a protocol trait. The first four traits (`Operator`, `Dispatcher`, `StateStore`, `Environment`) live in `layer0`. `Signalable` and `Queryable` live in Layer 2 (`skg-effects-core`).

### Protocol 1: Operator -- "What does one agent do per cycle?"

The `Operator` trait defines the boundary around a single agent's execution cycle. Input goes in, the agent reasons (model calls) and acts (tool execution), and output comes out.

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError>;
}
```

The trait is intentionally one method. From the outside, an operator is atomic -- you do not care whether it made 1 model call or 20, whether it used tools or not, or what context strategy it used. Those are implementation details.

Implementations include a context engine (composable three-phase engine with assembly, inference, reaction) and `SingleShotOperator` (one model call, no tools).

The context engine's `Context` type is the conversation store. It holds the messages array sent to the model. Your application's domain data — shell history, file state, user preferences — feeds into `Context` via assembly operations (`inject_system`, `inject_message`) or the `system_addendum` field in `OperatorConfig`. Domain data and conversation state are separate concerns: your app owns the domain data, `Context` owns the conversation.

### Protocol 2: Dispatcher, Signalable, Queryable -- "How do agents compose?"



These three traits decompose the orchestration boundary:



**Dispatcher** defines how one agent invokes another:



```rust

#[async_trait]

pub trait Dispatcher: Send + Sync {

    async fn dispatch(&self, operator: &OperatorId, input: OperatorInput)

        -> Result<OperatorOutput, OrchError>;

}

```



`dispatch` might be a function call (in-process) or a network hop to another continent. The caller does not know and does not care.



**Signalable** provides fire-and-forget inter-workflow messaging:



```rust

#[async_trait]

pub trait Signalable: Send + Sync {

    async fn signal(&self, target: &WorkflowId, signal: SignalPayload)

        -> Result<(), OrchError>;

}

```



**Queryable** enables read-only inspection of workflow state:



```rust

#[async_trait]

pub trait Queryable: Send + Sync {

    async fn query(&self, target: &WorkflowId, query: QueryPayload)

        -> Result<serde_json::Value, OrchError>;

}

```



Related: `dispatch_many()` is a free function in `skg-orch-kit` that dispatches multiple tasks in parallel using `Dispatcher`.


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

## Middleware and runtime coordination

### Middleware -- Observation and intervention

Per-boundary middleware traits wrap each protocol's operations using the continuation pattern. Three traits — one per protocol boundary — live in `layer0::middleware`:

- **`DispatchMiddleware`** wraps `Dispatcher::dispatch`. Code before `next.dispatch()` = pre-processing; code after = post-processing; not calling `next` = short-circuit.
- **`StoreMiddleware`** wraps `StateStore` read/write. Use for encryption-at-rest, audit trails, caching, access control.
- **`ExecMiddleware`** wraps `Environment::run`. Use for resource metering, credential injection, sandboxing.

Middleware composes via `DispatchStack`, `StoreStack`, and `ExecStack` builders that organize layers into observer → transformer → guard ordering.

For operator-local interception (before/after inference, before/after tool use), the Rule system provides typed per-trigger-point rules with default no-op implementations. Rules fire via `Trigger`: `Before`, `After`, or `When`.
### Lifecycle coordination -- Above Layer 0

Budget/compaction coordination and observation/intervention mechanics span multiple protocols, but they currently live in runtime or orchestration code above Layer 0. Layer 0 keeps the middleware seams and message-level hints such as `CompactionPolicy`; higher layers decide when to halt, compact, observe, or intervene.

## How layers compose

The six layers form a strict dependency hierarchy:

```
Layer 5  Cross-Cutting    (middleware, runtime governance)
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
Operator   = ContextEngine<AnthropicProvider> + ToolRegistry
Middleware = DispatchStack { RedactionMiddleware, ExfilGuardMiddleware }
State      = FsStore (filesystem persistence)
Env        = LocalEnv (no isolation, dev mode)
Orchestr.  = LocalOrch { agent_a -> Operator, agent_b -> Operator }
```

Each component is constructed independently, then composed through trait objects. The orchestrator holds `Arc<dyn Operator>` references. The environment holds its own operator reference. Nothing knows about concrete types beyond its own construction site.


## Tools and agents

These terms name configuration patterns built on top of `Operator`, not separate types.

**Tool:** An operator registered with `ToolMetadata` (name, description, JSON input schema, concurrency hint). The metadata makes the operator callable from an LLM reasoning loop. The distinction between a tool and any other operator is configuration, not type — the `Operator` trait is the same.

**Agent:** A configured operator. Concretely: an `Operator` implementation (typically a context engine) wired with a provider, identity, tools, and optionally an `Arc<dyn Dispatcher>` for sub-dispatching to other agents. The term 'agent' has no corresponding trait; it describes how an operator is assembled and what capabilities it receives at construction time.

To create an agent, wrap `react_loop()` (from `skg-context-engine`) in a struct that implements `Operator`. The struct holds the provider, tools, and config. The `execute()` method creates a fresh `Context`, assembles domain context into it, and calls `react_loop()`. The provider's generic type parameter is erased at the `Operator` boundary — callers interact with `Arc<dyn Operator>` and never see the concrete provider type. See the [operators guide](../guides/operators.md) for a complete example.