# Protocol Traits

Layer 0 defines six protocol traits and two cross-cutting interfaces. Every trait is object-safe (`Box<dyn Trait>` is `Send + Sync`), uses `#[async_trait]`, and is designed to be operation-defined rather than mechanism-defined.

"Operation-defined" means the trait says *what* happens, not *how*. `Operator::execute` means "cause this agent to process one cycle" -- not "make an API call" or "run a subprocess." This is what makes implementations swappable.

## Protocol 1: Operator

**Crate:** `layer0::operator`

The operator is what one agent does per cycle. It receives input, assembles context, reasons (model calls), acts (tool execution), and produces output.

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(
        &self,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OperatorError>;
}
```

The trait is one method. The operator is atomic from the outside.

### OperatorInput

```rust
pub struct OperatorInput {
    pub message: Content,              // The new message/task/signal
    pub trigger: TriggerType,          // What caused this invocation (User, Task, Signal, etc.)
    pub session: Option<SessionId>,    // Session for conversation continuity
    pub config: Option<OperatorConfig>,// Per-invocation config overrides
    pub metadata: serde_json::Value,   // Opaque passthrough (trace IDs, routing, etc.)
}
```

`OperatorInput` carries only what is *new*. It does not include conversation history or memory contents. The operator runtime reads those from a `StateStore` during context assembly. This keeps the protocol boundary clean.

### OperatorConfig

```rust
pub struct OperatorConfig {
    pub max_turns: Option<u32>,           // Max ReAct loop iterations
    pub max_cost: Option<Decimal>,        // Budget in USD
    pub max_duration: Option<DurationMs>, // Wall-clock timeout
    pub model: Option<String>,            // Model override
    pub allowed_operators: Option<Vec<String>>, // Operator restrictions
    pub system_addendum: Option<String>,  // Additional system prompt
}
```

Every field is optional. `None` means "use the implementation's default."

Tools are operators registered with `ToolMetadata`. The `allowed_operators` field restricts which operators can be sub-dispatched during a turn; tool names in this list are operator names.

### OperatorOutput

```rust
pub struct OperatorOutput {
    pub message: Content,              // The operator's response
    pub exit_reason: ExitReason,       // Why the loop stopped
    pub metadata: OperatorMetadata,    // Tokens, cost, timing, tool records
    pub effects: Vec<Effect>,          // Side-effects to execute
}
```

The `effects` field is a critical design decision. The operator *declares* effects but does not execute them. The calling layer (orchestrator, environment, lifecycle coordinator) decides when and how to execute them. This is what makes the same operator code work both in-process and in a durable workflow.

### ExitReason

```rust
pub enum ExitReason {
    Complete,                   // Natural completion
    MaxTurns,                   // Hit iteration limit
    BudgetExhausted,            // Hit cost budget
    CircuitBreaker,             // Consecutive failures
    Timeout,                    // Wall-clock timeout
    MiddlewareHalt { reason },    // Middleware halted execution
    Error,                      // Unrecoverable error
    Custom(String),             // Extension point
}
```

### OperatorMetadata

```rust
pub struct OperatorMetadata {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: Decimal,                    // USD, precise
    pub turns_used: u32,
    pub sub_dispatches: Vec<SubDispatchRecord>,
    pub duration: DurationMs,
}
```

Every field is concrete (not optional) because every operator produces this data. Implementations that cannot track a field (e.g., cost for a local model) use zero.

### SubDispatchRecord

`SubDispatchRecord` captures the result of a single sub-operator dispatch within a turn:

```rust
pub struct SubDispatchRecord {
    pub name: String,         // Operator name that was dispatched
    pub duration: DurationMs, // Wall-clock time for that dispatch
    pub success: bool,        // Whether the dispatch completed without error
}
```

## Protocol 2: Dispatcher



**Crate:** `layer0::dispatch`



The sole invocation primitive: how one agent's output becomes another agent's input.



```rust

#[async_trait]

pub trait Dispatcher: Send + Sync {

    async fn dispatch(

        &self,

        operator: &OperatorId,

        input: OperatorInput,

    ) -> Result<OperatorOutput, OrchError>;

}

```



- **`dispatch`** -- Send an operator invocation to a specific agent. May be in-process or remote. The key property: calling code does not know which implementation is behind the trait.



**Related:** `dispatch_many()` is a free function in `skg-orch-kit` that dispatches multiple tasks in parallel using `Dispatcher::dispatch`.



## Protocol 2b: Signalable



**Crate:** `layer0::signal`



Fire-and-forget inter-workflow messaging.



```rust

#[async_trait]

pub trait Signalable: Send + Sync {

    async fn signal(

        &self,

        target: &WorkflowId,

        signal: SignalPayload,

    ) -> Result<(), OrchError>;

}

```



- **`signal`** -- Fire-and-forget message to a running workflow. Returns when accepted, not when processed.



## Protocol 2c: Queryable



**Crate:** `layer0::query`



Read-only workflow state queries.



```rust

#[async_trait]

pub trait Queryable: Send + Sync {

    async fn query(

        &self,

        target: &WorkflowId,

        query: QueryPayload,

    ) -> Result<serde_json::Value, OrchError>;

}

```



- **`query`** -- Read-only query of a workflow's state. Returns `serde_json::Value` (schema depends on the workflow).

## Protocol 3: StateStore / StateReader

**Crate:** `layer0::state`

How data persists and is retrieved across turns and sessions.

```rust
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn read(&self, scope: &Scope, key: &str)
        -> Result<Option<serde_json::Value>, StateError>;
    async fn write(&self, scope: &Scope, key: &str, value: serde_json::Value)
        -> Result<(), StateError>;
    async fn delete(&self, scope: &Scope, key: &str)
        -> Result<(), StateError>;
    async fn list(&self, scope: &Scope, prefix: &str)
        -> Result<Vec<String>, StateError>;
    async fn search(&self, scope: &Scope, query: &str, limit: usize)
        -> Result<Vec<SearchResult>, StateError>;
}
```

The trait is deliberately minimal: CRUD + list + search. Compaction is not part of this trait because it requires cross-protocol coordination (the lifecycle interface). Versioning is not part of this trait because not all backends support it.

**StateReader** is a read-only projection:

```rust
#[async_trait]
pub trait StateReader: Send + Sync {
    async fn read(&self, scope: &Scope, key: &str)
        -> Result<Option<serde_json::Value>, StateError>;
    async fn list(&self, scope: &Scope, prefix: &str)
        -> Result<Vec<String>, StateError>;
    async fn search(&self, scope: &Scope, query: &str, limit: usize)
        -> Result<Vec<SearchResult>, StateError>;
}
```

Every `StateStore` automatically implements `StateReader` via a blanket impl. Operators receive `&dyn StateReader` during context assembly -- they can read but cannot write directly. Writes go through `Effect`s in the `OperatorOutput`.

## Protocol 4: Environment

**Crate:** `layer0::environment`

How an operator executes within an isolated context.

```rust
#[async_trait]
pub trait Environment: Send + Sync {
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError>;
}
```

The `Environment` owns or has access to whatever it needs to execute an operator. `run()` takes only data (`OperatorInput` + `EnvironmentSpec`), not a function reference. For `LocalEnv`, the operator is an `Arc<dyn Operator>` stored at construction time. For a hypothetical `DockerEnvironment`, the input would be serialized, sent to a container, and the output deserialized.

### EnvironmentSpec

```rust
pub struct EnvironmentSpec {
    pub isolation: Vec<IsolationBoundary>,  // Process, Container, Gvisor, MicroVm, Wasm, etc.
    pub credentials: Vec<CredentialRef>,     // Secrets to inject
    pub resources: Option<ResourceLimits>,   // CPU, memory, disk, GPU limits
    pub network: Option<NetworkPolicy>,      // Allow/deny rules
}
```

## Interface 5: Per-Boundary Middleware

**Crate:** `layer0::middleware`

Observation and intervention at protocol boundaries. Three traits cover the three boundaries where cross-cutting logic is needed:

```rust
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    async fn on_dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: DispatchNext<'_>,
    ) -> Result<OperatorOutput, OrchError>;
}

#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    async fn on_read(
        &self,
        scope: &Scope,
        key: &str,
        next: StoreNext<'_>,
    ) -> Result<Option<serde_json::Value>, StateError>;

    async fn on_write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        next: StoreNext<'_>,
    ) -> Result<(), StateError>;
}

#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    async fn on_exec(
        &self,
        input: OperatorInput,
        next: ExecNext<'_>,
    ) -> Result<OperatorOutput, OperatorError>;
}
```

Each middleware wraps the next layer in the stack. The `next` parameter is a callback that invokes the rest of the middleware chain (and ultimately the real implementation). Middleware can inspect/modify inputs before calling `next`, inspect/modify outputs after, or short-circuit by returning early without calling `next`.

Middleware is composed into stacks:

- **`DispatchStack`** -- wraps Dispatcher::dispatch (budget enforcement, logging, routing)
- **`StoreStack`** -- wraps state store access (redaction, audit logging)
- **`ExecStack`** -- wraps operator execution (security guardrails, telemetry)

The Rule system provides typed interception within the context engine specifically, for use cases like tool-call filtering that are operator-internal rather than cross-cutting. Rules fire via Trigger enum: Before (pre-inference, pre-tool), After (post-inference, post-tool), or When (exit checks, steering).

## Message-Level Hints

**Crate:** `layer0::lifecycle`

This module now carries only message-level policy hints that travel with protocol data:

- **`CompactionPolicy`** -- An advisory per-message hint consumed by compaction code in the runtime or orchestration layers.

Lifecycle coordination, telemetry streams, and observation/intervention mechanics live above Layer 0 unless they are later promoted into a real cross-boundary contract.
