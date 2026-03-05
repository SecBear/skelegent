# Orchestrator Composition Guide

## The confusion this doc prevents

People building on neuron hit the same confusion: "I need to chain multiple LLM
calls. Where does that code go? Is it an operator? A new trait? Something
between operator and orchestrator?"

The answer: **it goes in the orchestrator**. There is no missing layer. This doc
explains why and how.

## The two roles

### Operator: one atomic unit of LLM work

An operator receives input, does one reasoning cycle (which may involve multiple
LLM calls and tool uses internally via a ReAct loop), and returns output +
declared effects.

```rust
// Operator is intentionally one method.
// Everything inside is the implementation's concern.
async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError>;
```

What operators do:

- Assemble context (identity + history + memory + tools)
- Run a reasoning loop (reason, act, observe, repeat)
- Declare effects (WriteMemory, Delegate, Signal, etc.)

What operators do NOT do:

- Execute effects (that's the orchestrator's job)
- Sequence other operators (that's orchestration)
- Handle durability (that's orchestration)
- Inject credentials (that's the environment, mediated by the orchestrator)

### Orchestrator: everything else

The orchestrator composes operators into workflows, handles durability, injects
credentials, coordinates communication, and executes effects. It is
intentionally the vaguest protocol in neuron because composition semantics vary
wildly by deployment.

```rust
// dispatch() RETURNS the result. This is synchronous from the caller's perspective.
async fn dispatch(&self, agent: &AgentId, input: OperatorInput) -> Result<OperatorOutput, OrchError>;
```

The orchestrator implementation decides:

- Whether dispatch is a function call or a Temporal activity
- Whether there's checkpointing between steps
- How credentials get injected per-operator
- Whether delegation is sync (await result) or async (fire-and-forget)
- How results flow between operators
- What happens on crash (nothing, replay, checkpoint restore)

## Multi-step workflows are just sequential dispatches

Pipeline composition doesn't need a framework — it's just code:

```rust
// A sweep workflow: three operators, sequenced, with data flow.
// This is application code, not a framework feature.
async fn run_sweep(
    orch: &dyn Orchestrator,
    decision_id: &str,
) -> Result<SweepVerdict, OrchError> {
    // Step 1: Research
    let research_input = OperatorInput::new(
        Content::text(format!("Research current state of: {}", decision_id)),
        TriggerType::Schedule,
    );
    let research_output = orch.dispatch(&"research".into(), research_input).await?;

    // Step 2: Compare (receives research findings via input)
    let mut compare_input = OperatorInput::new(
        research_output.message.clone(),
        TriggerType::Task,
    );
    compare_input.metadata = json!({ "decision_id": decision_id });
    let compare_output = orch.dispatch(&"compare".into(), compare_input).await?;

    // Step 3: Plan (receives comparison verdict via input)
    let plan_input = OperatorInput::new(
        compare_output.message.clone(),
        TriggerType::Task,
    );
    let plan_output = orch.dispatch(&"plan".into(), plan_input).await?;

    // Collect effects from all steps for execution
    let all_effects: Vec<Effect> = [research_output, compare_output, plan_output]
        .iter()
        .flat_map(|o| o.effects.clone())
        .collect();

    // ... execute effects, build verdict
    Ok(verdict)
}
```

This code works unchanged with any orchestrator:

- `LocalOrch`: dispatches are function calls, no durability
- `TemporalOrch`: each dispatch is a Temporal activity, workflow replays on
  crash
- `RestateOrch`: each dispatch is a Restate handler, automatic checkpointing

**Same operators. Same workflow code. Different durability guarantees.** That's
the architectural position: "deployment choice, not code change."

## Where each concern lives

| Concern                     | Owner                                           | NOT          |
| --------------------------- | ----------------------------------------------- | ------------ |
| LLM reasoning               | Operator                                        | Orchestrator |
| Tool execution              | Operator (within turn)                          | Orchestrator |
| Context assembly            | Operator (via ContextStrategy)                  | Orchestrator |
| Effect declaration          | Operator                                        | Orchestrator |
| Effect execution            | Orchestrator (via EffectInterpreter)            | Operator     |
| Operator sequencing         | Orchestrator / application code                 | Operator     |
| Data flow between operators | Orchestrator / application code                 | Operator     |
| Credential injection        | Environment (mediated by Orchestrator)          | Operator     |
| Durability / checkpointing  | Orchestrator implementation                     | Operator     |
| Crash recovery              | Orchestrator implementation                     | Operator     |
| Retry policy                | Orchestrator implementation                     | Operator     |
| Budget enforcement          | Operator (per-turn) + Orchestrator (cross-turn) | Either alone |
| Communication               | Orchestrator implementation                     | Operator     |
| Result return               | Orchestrator (dispatch returns OperatorOutput)  | Operator     |

## Anti-patterns

### Orchestration inside an operator

**Wrong**: A single operator that sequences multiple LLM calls, reads/writes
state directly, and handles its own retries.

```rust
// BAD: This is orchestration masquerading as an operator
pub async fn run(&self, store: &dyn StateStore) -> Result<Verdict, Error> {
    let research = self.provider.search(query).await?;  // LLM call 1
    let comparison = self.provider.compare(research).await?;  // LLM call 2
    let plan = self.provider.plan(comparison).await?;  // LLM call 3
    store.write_hinted(&scope, &key, value, &opts).await?;  // Direct state write
    Ok(verdict)
}
```

Problems:

- Direct state writes bypass the effect pipeline (hooks don't fire)
- No checkpointing between steps (crash loses all work)
- Not composable (can't swap in durable execution)
- Not observable (no ExecutionTrace, no events)

**Right**: Three operators + workflow code using Orchestrator::dispatch().

### A "workflow trait" or "pipeline trait"

**Wrong**: Adding a new protocol trait for multi-step workflows.

The Orchestrator trait already handles this. `dispatch()` returns results.
Sequential calls with data transformation between them IS a pipeline. No new
abstraction needed.

### Separate "client" crates for HTTP APIs

**Wrong**: Creating `neuron-client-parallel-ai` or `neuron-client-github`.

Per the architecture: HTTP clients are implementation details inside
provider/effect crates. They embed inside the operator or effect executor that
uses them. The Orchestrator doesn't need to know about HTTP.

## When to create a new orchestrator implementation

Create a new orchestrator implementation when you need different composition
semantics:

- **LocalOrch**: In-process, no durability. For dev/test.
- **TemporalOrch**: Temporal workflows. For production durable execution.
- **RestateOrch**: Restate handlers. Alternative durable execution.
- **HttpOrch**: Dispatch over HTTP. For microservice deployments.

Each implementation gives the same operators different durability, retry, and
communication guarantees. The operators don't change.

## Composition and lifecycle implementation

The orchestrator is where most composition and lifecycle decisions get
implemented:

- **Child context assembly**: The orchestrator builds OperatorInput for child
  dispatches
- **Result return**: `dispatch()` returns OperatorOutput synchronously
- **Lifecycle management**: The orchestrator manages agent lifetimes
- **Communication routing**: The orchestrator routes signals, queries, and
  events
- **Durability**: The orchestrator implementation provides durability
- **Retry policy**: The orchestrator implementation decides retry policy
- **Crash recovery**: The orchestrator implementation handles recovery

Single-turn decisions (trigger, agent internals, exit conditions, memory writes,
compaction, budget, observability) live in the operator and its support crates.
