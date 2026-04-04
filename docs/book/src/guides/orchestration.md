# Orchestration

Orchestration is how multiple agents compose and how execution survives failures. The `Dispatcher` trait provides dispatch (send work to agents), `Signalable` provides signaling (inter-workflow communication), and `Queryable` provides queries (read-only state inspection).

## The Dispatcher, Signalable, and Queryable traits

```rust
#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, ProtocolError>;
}

#[async_trait]
pub trait Signalable: Send + Sync {
    async fn signal(
        &self,
        target: &WorkflowId,
        signal: SignalPayload,
    ) -> Result<(), ProtocolError>;
}

#[async_trait]
pub trait Queryable: Send + Sync {
    async fn query(
        &self,
        target: &WorkflowId,
        query: QueryPayload,
    ) -> Result<serde_json::Value, ProtocolError>;
}
```



Related: `dispatch_many()` is a free function in `skg-orch-kit` that dispatches multiple tasks in parallel using `Dispatcher`.


## LocalOrch (`skg-orch-local`)

The local orchestrator dispatches operator invocations in-process using tokio. It maps `OperatorId` values to `Arc<dyn Operator>` references and calls `execute()` directly.

```rust,no_run
use skg_orch_local::LocalOrch;
use layer0::operator::Operator;
use layer0::id::OperatorId;
use std::sync::Arc;

// Assume `coder` and `reviewer` are constructed operators
let coder: Arc<dyn Operator> = /* ... */;
let reviewer: Arc<dyn Operator> = /* ... */;

let mut orchestrator = LocalOrch::new();
orchestrator.register(OperatorId("coder".into()), coder);
orchestrator.register(OperatorId("reviewer".into()), reviewer);
```

### Dispatching

Single dispatch sends work to one agent:

```rust,no_run
use layer0::dispatch::Dispatcher;
use layer0::operator::{OperatorInput, TriggerType};
use layer0::content::Content;
use layer0::id::OperatorId;

# async fn example(dispatcher: &dyn Dispatcher) -> Result<(), Box<dyn std::error::Error>> {
let input = OperatorInput::new(
    Content::text("Implement the authentication module"),
    TriggerType::Task,
);

let output = dispatcher
    .dispatch(&OperatorId("coder".into()), input)
    .await?;

println!("Agent response: {:?}", output.message);
# Ok(())
# }
```

### Parallel dispatch

`dispatch_many` sends work to multiple agents concurrently. The local orchestrator uses `tokio::spawn` for parallelism:

```rust,no_run
use layer0::dispatch::Dispatcher;
use layer0::operator::{OperatorInput, TriggerType};
use layer0::content::Content;
use layer0::id::OperatorId;

# async fn example(dispatcher: &dyn Dispatcher) -> Result<(), Box<dyn std::error::Error>> {
let tasks = vec![
    (
        OperatorId("analyzer".into()),
        OperatorInput::new(Content::text("Analyze security risks"), TriggerType::Task),
    ),
    (
        OperatorId("reviewer".into()),
        OperatorInput::new(Content::text("Review code quality"), TriggerType::Task),
    ),
];

let results = dispatcher.dispatch_many(tasks).await;
for result in results {
    match result {
        Ok(output) => println!("Success: {:?}", output.outcome),
        Err(e) => println!("Failed: {}", e),
    }
}
# Ok(())
# }
```

Results are returned in the same order as the input tasks. Individual tasks may fail independently.

### Signals

Signals provide fire-and-forget messaging to running workflows:

```rust,no_run
use skg_effects_core::Signalable;
use layer0::effect::SignalPayload;
use layer0::id::WorkflowId;

# async fn example(signalable: &dyn Signalable) -> Result<(), Box<dyn std::error::Error>> {
let signal = SignalPayload {
    signal_type: "cancel".into(),
    data: serde_json::json!({"reason": "user requested"}),
};

signalable
    .signal(&WorkflowId("wf-001".into()), signal)
    .await?;
# Ok(())
# }
```

`signal()` returns `Ok(())` when the signal is accepted, not when it is processed.

### Queries

Queries provide read-only inspection of workflow state:

```rust,no_run
use skg_effects_core::{Queryable, QueryPayload};
use layer0::id::WorkflowId;

# async fn example(queryable: &dyn Queryable) -> Result<(), Box<dyn std::error::Error>> {
let query = QueryPayload::new("status", serde_json::json!({}));
let result = queryable
    .query(&WorkflowId("wf-001".into()), query)
    .await?;
println!("Workflow status: {}", result);
# Ok(())
# }
```

## OrchKit (`skg-orch-kit`)

The `skg-orch-kit` crate provides shared utilities for orchestrator implementations. These are building blocks that any orchestrator (local, Temporal, Restate) can reuse.

## Error handling

All dispatch, signal, and query operations return `ProtocolError`:

```rust
pub enum ProtocolError {
    NotFound { operator: OperatorId },   // No agent registered with that ID
    PolicyDenied { reason: String },     // Middleware/policy short-circuit
    Transient { message: String },       // Retryable failure
    Permanent { message: String },       // Non-retryable failure
    Internal { message: String },        // Unexpected runtime error
    Other(Box<dyn Error>),               // Catch-all
}
```

`ProtocolError::is_retryable()` lets retry middleware determine whether to attempt the dispatch again.

## Future orchestrators

The `Dispatcher`, `Signalable`, and `Queryable` traits are designed to support orchestrators beyond in-process dispatch:

- **Temporal** -- Durable execution with automatic replay and fault tolerance. `dispatch` becomes a Temporal activity. `signal` maps to Temporal signals. `query` maps to Temporal queries.
- **Restate** -- Durable execution with virtual objects. Similar to Temporal but with a different programming model.
- **HTTP** -- Dispatch over HTTP for microservice architectures. `dispatch` sends a serialized `OperatorInput` over the network.

The traits are transport-agnostic by design. All protocol types (`OperatorInput`, `OperatorOutput`, `SignalPayload`, `QueryPayload`) implement `Serialize + Deserialize`, so they can cross any boundary.


## Intents, signals, and custom operators

Skelegent draws a hard boundary: operators declare `intents`; orchestrators execute them. This separation lets you reuse the same operator across transports (in-process, Temporal, Restate) without leaking execution mechanics.

Custom operators (e.g., barrier-scheduled loops) can freely declare intents like `Intent::Delegate` or `Intent::Signal`. The orchestrator decides when to execute them relative to dispatch lifecycles, and exposes `signal()`/`query()` for out-of-band communication.

Defaults stay slim: if you do nothing, wrap `react_loop` in a simple operator or use `SingleShotOperator`. If you need custom control (barriers and steering), implement a custom operator and keep effects at the boundary. See `examples/custom_operator_barrier`.