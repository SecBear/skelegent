# Orchestration

Orchestration is how multiple agents compose and how execution survives failures. The `Orchestrator` trait provides dispatch (send work to agents), signaling (inter-workflow communication), and queries (read-only state inspection).

## The Orchestrator trait

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

## LocalOrchestrator (`neuron-orch-local`)

The local orchestrator dispatches operator invocations in-process using tokio. It maps `AgentId` values to `Arc<dyn Operator>` references and calls `execute()` directly.

```rust,no_run
use neuron_orch_local::LocalOrchestrator;
use layer0::operator::Operator;
use layer0::id::AgentId;
use std::sync::Arc;

// Assume `coder` and `reviewer` are constructed operators
let coder: Arc<dyn Operator> = /* ... */;
let reviewer: Arc<dyn Operator> = /* ... */;

let mut orchestrator = LocalOrchestrator::new();
orchestrator.register(AgentId("coder".into()), coder);
orchestrator.register(AgentId("reviewer".into()), reviewer);
```

### Dispatching

Single dispatch sends work to one agent:

```rust,no_run
use layer0::orchestrator::Orchestrator;
use layer0::operator::{OperatorInput, TriggerType};
use layer0::content::Content;
use layer0::id::AgentId;

# async fn example(orchestrator: &dyn Orchestrator) -> Result<(), Box<dyn std::error::Error>> {
let input = OperatorInput::new(
    Content::text("Implement the authentication module"),
    TriggerType::Task,
);

let output = orchestrator
    .dispatch(&AgentId("coder".into()), input)
    .await?;

println!("Agent response: {:?}", output.message);
# Ok(())
# }
```

### Parallel dispatch

`dispatch_many` sends work to multiple agents concurrently. The local orchestrator uses `tokio::spawn` for parallelism:

```rust,no_run
use layer0::orchestrator::Orchestrator;
use layer0::operator::{OperatorInput, TriggerType};
use layer0::content::Content;
use layer0::id::AgentId;

# async fn example(orchestrator: &dyn Orchestrator) -> Result<(), Box<dyn std::error::Error>> {
let tasks = vec![
    (
        AgentId("analyzer".into()),
        OperatorInput::new(Content::text("Analyze security risks"), TriggerType::Task),
    ),
    (
        AgentId("reviewer".into()),
        OperatorInput::new(Content::text("Review code quality"), TriggerType::Task),
    ),
];

let results = orchestrator.dispatch_many(tasks).await;
for result in results {
    match result {
        Ok(output) => println!("Success: {:?}", output.exit_reason),
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
use layer0::orchestrator::Orchestrator;
use layer0::effect::SignalPayload;
use layer0::id::WorkflowId;

# async fn example(orchestrator: &dyn Orchestrator) -> Result<(), Box<dyn std::error::Error>> {
let signal = SignalPayload {
    signal_type: "cancel".into(),
    data: serde_json::json!({"reason": "user requested"}),
};

orchestrator
    .signal(&WorkflowId("wf-001".into()), signal)
    .await?;
# Ok(())
# }
```

`signal()` returns `Ok(())` when the signal is accepted, not when it is processed.

### Queries

Queries provide read-only inspection of workflow state:

```rust,no_run
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::id::WorkflowId;

# async fn example(orchestrator: &dyn Orchestrator) -> Result<(), Box<dyn std::error::Error>> {
let query = QueryPayload::new("status", serde_json::json!({}));
let result = orchestrator
    .query(&WorkflowId("wf-001".into()), query)
    .await?;
println!("Workflow status: {}", result);
# Ok(())
# }
```

## OrchKit (`neuron-orch-kit`)

The `neuron-orch-kit` crate provides shared utilities for orchestrator implementations. These are building blocks that any orchestrator (local, Temporal, Restate) can reuse.

## Error handling

```rust
pub enum OrchError {
    AgentNotFound(String),    // No agent registered with that ID
    WorkflowNotFound(String), // No workflow with that ID
    DispatchFailed(String),   // Dispatch failed for other reasons
    SignalFailed(String),     // Signal delivery failed
    OperatorError(OperatorError), // Propagated from the operator
    Other(Box<dyn Error>),    // Catch-all
}
```

`OperatorError` propagates through `OrchError` via `From`. If an operator fails during dispatch, the error is wrapped as `OrchError::OperatorError`.

## Future orchestrators

The `Orchestrator` trait is designed to support orchestrators beyond in-process dispatch:

- **Temporal** -- Durable execution with automatic replay and fault tolerance. `dispatch` becomes a Temporal activity. `signal` maps to Temporal signals. `query` maps to Temporal queries.
- **Restate** -- Durable execution with virtual objects. Similar to Temporal but with a different programming model.
- **HTTP** -- Dispatch over HTTP for microservice architectures. `dispatch` sends a serialized `OperatorInput` over the network.

The trait is transport-agnostic by design. All protocol types (`OperatorInput`, `OperatorOutput`, `SignalPayload`, `QueryPayload`) implement `Serialize + Deserialize`, so they can cross any boundary.


## Effects, signals, and custom operators

Neuron draws a hard boundary: operators declare `effects`; orchestrators execute them. This separation lets you reuse the same operator across transports (in-process, Temporal, Restate) without leaking execution mechanics.

Custom operators (e.g., barrier-scheduled loops) can freely declare effects like `Effect::Log`, `Effect::Delegate`, or `Effect::Signal`. The orchestrator decides when to execute them relative to dispatch lifecycles, and exposes `signal()`/`query()` for out-of-band communication.

Defaults stay slim: if you do nothing, use `ReactOperator` or `SingleShotOperator`. If you need Rho-like control (barriers and steering), implement a custom operator and keep effects at the boundary. See `examples/custom_operator_barrier`.