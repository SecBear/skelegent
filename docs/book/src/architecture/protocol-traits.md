# Protocol Traits

Layer 0 defines four protocol traits and two cross-cutting interfaces. Every trait is object-safe (`Box<dyn Trait>` is `Send + Sync`), uses `#[async_trait]`, and is designed to be operation-defined rather than mechanism-defined.

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
    ObserverHalt { reason },    // Hook halted execution
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

## Protocol 2: Orchestrator

**Crate:** `layer0::orchestrator`

How operators from different agents compose, and how execution survives failures.

```rust
#[async_trait]
pub trait Orchestrator: Send + Sync {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError>;

    async fn dispatch_many(
        &self,
        tasks: Vec<(AgentId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>>;

    async fn signal(
        &self,
        target: &WorkflowId,
        signal: SignalPayload,
    ) -> Result<(), OrchError>;

    async fn query(
        &self,
        target: &WorkflowId,
        query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError>;
}
```

- **`dispatch`** -- Send an operator invocation to a specific agent. May be in-process or remote.
- **`dispatch_many`** -- Parallel dispatch. Results returned in input order. Individual tasks may fail independently.
- **`signal`** -- Fire-and-forget message to a running workflow. Returns when accepted, not when processed.
- **`query`** -- Read-only query of a workflow's state. Returns `serde_json::Value` (schema depends on the workflow).

The key property: calling code does not know which implementation is behind the trait. `dispatch()` might be a function call or a network hop.

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

## Interface 5: Hook

**Crate:** `layer0::hook`

Observation and intervention in the operator's inner loop.

```rust
#[async_trait]
pub trait Hook: Send + Sync {
    fn points(&self) -> &[HookPoint];
    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError>;
}
```

Hooks fire at five defined points:

| HookPoint | When |
|-----------|------|
| `PreInference` | Before each model call |
| `PostInference` | After model responds, before tool execution |
| `PreSubDispatch` | Before each tool is executed |
| `PostSubDispatch` | After each tool completes |
| `ExitCheck` | At each exit-condition check |

`HookContext` provides read-only access to the current state: tool name/input/result, model output, running token count, running cost, turns completed, elapsed time.

`HookAction` determines what happens next:

| Action | Effect |
|--------|--------|
| `Continue` | Proceed normally |
| `Halt { reason }` | Stop the operator with `ExitReason::ObserverHalt` |
| `SkipDispatch { reason }` | Skip this tool call (PreSubDispatch only) |
| `ModifyDispatchInput { new_input }` | Replace tool input before execution (PreSubDispatch only) |
| `ModifyDispatchOutput { new_output }` | Replace tool output (PostSubDispatch only) |

Hook errors are logged but do **not** halt execution. Use `HookAction::Halt` to halt.

## Interface 6: Lifecycle Events

**Crate:** `layer0::lifecycle`

Cross-layer coordination events:

- **`BudgetEvent`** -- Emitted when cost thresholds are crossed. A hook observes cost, emits a budget event, and the orchestrator can react (cancel the workflow, notify the user, adjust limits).
- **`CompactionEvent`** -- Coordinates context compaction between the operator and the state store.
- **`ObservableEvent`** -- General-purpose observable events for telemetry and monitoring.

These events are the glue between protocols. They carry information across boundaries that individual protocols cannot see.
