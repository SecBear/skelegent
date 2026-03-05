# Effects and Execution Semantics

## Purpose

Effects are the boundary between "reasoning" and "side effects." Operators declare effects; outer layers decide how and when effects execute.

This is the key mechanism that makes Neuron composable.

## Effect Vocabulary

The `layer0::Effect` enum is the shared language for:

- state writes/deletes
- delegation and handoff to other agents
- signaling other workflows

## Effect Variants

### WriteMemory

Write a value to persistent state. The operator does not call the store directly — it
declares this effect and the executing layer handles delivery.

```rust
Effect::WriteMemory {
    scope: Scope,                      // where to write (session / workflow / agent / global)
    key: String,                       // storage key
    value: serde_json::Value,          // value to store
    tier: Option<MemoryTier>,          // advisory: hot / warm / cold
    lifetime: Option<Lifetime>,        // advisory: transient / session / durable
    content_kind: Option<ContentKind>, // advisory: episodic / semantic / procedural / structural
    salience: Option<f64>,             // advisory: importance hint 0.0–1.0
    ttl: Option<DurationMs>,           // advisory: auto-expire after this duration
}
```

**Core fields** (`scope`, `key`, `value`) are always required. They determine what is written and where.

**Advisory fields** (`tier`, `lifetime`, `content_kind`, `salience`, `ttl`) are always `Option` and default to `None`. Backends **MAY** ignore any or all of them. Operators set them as hints to influence storage routing, retention policy, or eviction priority — they carry no guarantee.

| Field | Type | Description |
|---|---|---|
| `scope` | `Scope` | Scope hierarchy: `Session`, `Workflow`, `Agent { workflow, agent }`, `Global`, or `Custom`. |
| `key` | `String` | Storage key within the scope. |
| `value` | `serde_json::Value` | Value to write. Any JSON-serializable data. |
| `tier` | `Option<MemoryTier>` | Storage tier hint. `Hot` (default) = low-latency; `Warm` = in-process/near cache; `Cold` = slow/cheap. |
| `lifetime` | `Option<Lifetime>` | Persistence policy. `Transient` = current turn only; `Session` = current session; `Durable` = persists across sessions. |
| `content_kind` | `Option<ContentKind>` | Cognitive category. `Episodic` / `Semantic` / `Procedural` / `Structural` / `Custom`. Backends may route storage differently per category. |
| `salience` | `Option<f64>` | Importance hint in range 0.0–1.0. Higher = more important to preserve under compaction pressure. |
| `ttl` | `Option<DurationMs>` | Auto-expire after this duration. Backends that do not support TTL ignore this field. |

### DeleteMemory

Delete a value from persistent state. Idempotent — missing key is not an error.

```rust
Effect::DeleteMemory {
    scope: Scope,
    key: String,
}
```

### Delegate

Request that the orchestrator dispatch another agent. The current operator does not call
the agent directly — it declares the intent.

```rust
Effect::Delegate {
    agent: AgentId,
    input: Box<OperatorInput>,
}
```

### Handoff

Transfer control to another agent. Unlike `Delegate`, the current operator is finished —
the next agent takes over entirely.

```rust
Effect::Handoff {
    agent: AgentId,
    state: serde_json::Value,  // context to pass to the next agent
}
```

### Signal

Fire-and-forget message to another workflow.

```rust
Effect::Signal {
    target: WorkflowId,
    payload: SignalPayload,
}
```

### Log

Emit an observable log/trace event. Consumed by observers and telemetry; not a state write.

```rust
Effect::Log {
    level: LogLevel,
    message: String,
    data: Option<serde_json::Value>,
}
```

### LinkMemory

Create a directed link between two memory entries within the same scope.

```rust
Effect::LinkMemory {
    scope: Scope,      // scope containing both entries
    link: MemoryLink,  // from_key, to_key, relation, optional metadata
}
```

### UnlinkMemory

Remove a directed link between two memory entries.

```rust
Effect::UnlinkMemory {
    scope: Scope,      // scope containing both entries
    from_key: String,  // source key
    to_key: String,    // target key
    relation: String,  // relationship type to remove
}
```

### Custom

Escape hatch for domain-specific effects that have not yet stabilized into a named variant.

```rust
Effect::Custom {
    effect_type: String,
    data: serde_json::Value,
}
```

When a custom effect type is used by three or more implementations, it graduates to a
named variant.

## Required Semantics (Core)

Neuron is "core complete" only when there is a clear, test-proven definition of how effects are handled.

At minimum:

1. **WriteMemory/DeleteMemory**: must be executed against a selected `StateStore` with deterministic key/scope semantics.
2. **Delegate**: represents "run another agent with an input and return results to the current control flow."
3. **Handoff**: represents "transfer responsibility to another agent," potentially with different lifecycle semantics.
4. **Signal**: represents "fire-and-forget asynchronous message to a workflow."

## Who Executes Effects?

Effects should be executed by orchestration/runtime glue, not by the operator itself.

- Local orchestration may execute them immediately.
- Durable orchestration may serialize and execute them inside workflow engines.

## Executor Guidance: Threading Advisory Fields

### Contract

Effect executors **MUST** package all five advisory fields from `WriteMemory` into a
`StoreOptions` struct and call `StateStore::write_hinted()`. They **MUST NOT** call
`StateStore::write()` directly for `WriteMemory` effects, because doing so silently
discards the advisory hints.

```rust
// Executors MUST do this:
let opts = StoreOptions {
    tier: *tier,
    lifetime: *lifetime,
    content_kind: content_kind.clone(),
    salience: *salience,
    ttl: *ttl,
};
state.write_hinted(scope, key, value, &opts).await?;
```

Backends **MAY** ignore any advisory field. The default `write_hinted()` implementation on
`StateStore` delegates to `write()`, discarding all options — this is correct behaviour
for backends that do not support hints.

### Reference Implementations

Two executors in this workspace demonstrate the pattern:

- **`LocalEffectExecutor`** (`neuron-effects-local`) — standalone executor implementing
  the `EffectExecutor` trait. Constructs `StoreOptions` from all five advisory fields,
  fires the optional `PreMemoryWrite` hook (which may halt or modify the value), then
  calls `write_hinted()`.

- **`LocalEffectInterpreter`** (`neuron-orch-kit`) — per-effect interpreter used by
  `OrchestratedRunner`. Identical advisory-field threading as `LocalEffectExecutor`.
  Additionally records a `MemoryWritten` event to the `ExecutionTrace`.

Both follow the same pattern. New executors for durable or remote backends must do the
same — construct `StoreOptions` and call `write_hinted()`, leaving backend-specific
interpretation of the hints to the backend.

### Hook Integration

Before calling `write_hinted()`, executors with a `HookRegistry` fire the `PreMemoryWrite` hook point. See `specs/09-hooks-lifecycle-and-governance.md` for hook dispatch semantics and actions.
