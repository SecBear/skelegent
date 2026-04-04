# Layer0 Intent / Event Migration Annex

## Purpose

Define the second implementation-grade `v2` migration slice for `layer0`:

- replace `Effect` with executable `Intent`
- replace `DispatchEvent` with semantic `ExecutionEvent`
- replace `OperatorOutput.effects` with `OperatorOutput.intents`
- finish the immediate-path cutover started in
  `specs/v2/15-layer0-invocation-outcome-migration-annex.md`
- lock the exact temporary adapters allowed while current effect-based callers
  are being removed

This annex is implementation-authoritative for the `v2` intent/event cutover
slice once a future change explicitly adopts the `v2` track.

## Migration Posture

This slice assumes a deliberate breaking cutover.

Rules:

- the merged public kernel surface MUST expose `Intent` and `ExecutionEvent`
  only
- superseded `v1` nouns (`Effect`, `DispatchEvent`, `EffectEmitter`) are
  removed from the merged public kernel surface
- canonical `v2` serde behavior does not have to accept old `effect` or
  `dispatch_event` wire forms
- branch-only migration adapters are allowed while the cutover PR is in flight,
  but they MUST remain crate-private and MUST be removed before merge unless
  this annex explicitly keeps them

This repo currently has no downstream compatibility obligation strong enough to
justify carrying public adapter debt into the merged `v2` kernel.

## Scope

This slice is intentionally narrow.

It covers:

- executable intent type ownership and declaration semantics
- semantic event envelope and stream terminal semantics
- immediate invocation stream replacement from `DispatchEvent` to
  `ExecutionEvent`
- `OperatorOutput.intents` as the only executable declaration field
- exact replacement rules from the current effect/event surfaces
- exact branch-only migration adapters allowed during the refactor
- provider-to-semantic projection rules needed to keep the event plane
  replay-meaningful

It does not cover:

- capability discovery migration (`specs/v2/17-capability-discovery-migration-annex.md`)
- durable lifecycle substrate redesign
- durable run read-model redesign
- generic query-service design
- `orch/skg-orch-kit`'s local `ExecutionEvent` trace enum
- provider chunk or transport protocol passthrough as kernel events

## Ownership

| Crate | Module | Owns in this slice |
|---|---|---|
| `layer0` | `src/intent.rs` | `Intent`, `IntentMeta`, `IntentKind`, and the executable helper nouns `Scope`, `MemoryScope`, `SignalPayload`, `HandoffContext` |
| `layer0` | `src/event.rs` | `ExecutionEvent`, `EventMeta`, `EventSource`, `EventKind` |
| `layer0` | `src/operator.rs` | `OperatorOutput.intents`; removal of `OperatorOutput.effects` and effect-oriented helpers from the public surface |
| `layer0` | `src/dispatch.rs` | `InvocationHandle` event item changes to `ExecutionEvent`; `InvocationSender::send(ExecutionEvent)`; `CollectedInvocation.events: Vec<ExecutionEvent>`; removal of `DispatchEvent` and `EffectEmitter` from the public surface |
| `layer0` | `src/effect.rs` | branch-only private compatibility helpers while downstream effect callers are being removed; no public ownership after merge |
| `layer0` | `src/lib.rs` | public re-exports switched to `intent`/`event` nouns only |
| `op/skg-context-engine` | `src/context.rs` | `push_intent`, `extend_intents`, `intents`, `drain_intents`; no effect-declaration API |
| `op/skg-context-engine` | `src/stream.rs` | `ContextMutation::IntentDeclared`; `InferenceDelta` stays runtime-local and is not a kernel semantic event |
| `op/skg-context-engine` | `src/react.rs` and `src/stream_react.rs` | runtime emits `Intent` and `ExecutionEvent`; removes dual-write effect projection before merge |
| `orch/skg-run-core` | `src/observe.rs` | `RunUpdate` stays durable-specific; only invocation-projection surfaces reuse `ExecutionEvent` vocabulary where applicable |

No executable helper noun may remain duplicated between `layer0::intent` and
`layer0::effect` after the merged cutover.

## Canonical Public Types

### Shared Executable Helper Nouns

The following helper nouns remain public and are owned only by
`layer0/src/intent.rs` in the merged surface:

- `Scope`
- `MemoryScope`
- `SignalPayload`
- `HandoffContext`

Rules:

- their public shapes remain the existing `layer0::intent` shapes
- `layer0::effect` duplicate definitions are removed from the merged public
  surface
- any branch-only projection from legacy `layer0::effect::*` helper types into
  `layer0::intent::*` helper types MUST be one-way and crate-private

### Intent

`Intent` is the executable declaration model.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub meta: IntentMeta,
    pub kind: IntentKind,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentMeta {
    pub intent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub seq: u64,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IntentKind {
    WriteMemory {
        scope: Scope,
        key: String,
        value: serde_json::Value,
        #[serde(default)]
        memory_scope: MemoryScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<MemoryTier>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<Lifetime>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_kind: Option<ContentKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        salience: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl: Option<DurationMs>,
    },
    DeleteMemory {
        scope: Scope,
        key: String,
    },
    LinkMemory {
        scope: Scope,
        link: MemoryLink,
    },
    UnlinkMemory {
        scope: Scope,
        from_key: String,
        to_key: String,
        relation: String,
    },
    Signal {
        target: WorkflowId,
        payload: SignalPayload,
    },
    Delegate {
        operator: OperatorId,
        input: Box<OperatorInput>,
    },
    Handoff {
        operator: OperatorId,
        context: HandoffContext,
    },
    RequestApproval {
        tool_name: String,
        call_id: String,
        input: serde_json::Value,
    },
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}
```

Rules:

- `IntentKind` is executable-only; observational payloads are forbidden by type
- `intent_id` is an opaque identifier; consumers MUST NOT parse it
- `IntentMeta.seq` is authoritative for in-invocation replay ordering;
  implementations MUST preserve declaration order within a single invocation,
  but the sequence does not need to be globally gap-free across concurrent
  invocations
- `RequestApproval` remains an executable declaration because it creates a wait
  requirement the outer executor must honor
- `Custom` is executable only; observational custom payloads belong in
  `EventKind::Observation`, `EventKind::Log`, or a future named event variant

### ExecutionEvent

`ExecutionEvent` is the semantic observation envelope.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub meta: EventMeta,
    pub kind: EventKind,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMeta {
    pub event_id: String,
    pub dispatch_id: DispatchId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_dispatch_id: Option<DispatchId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub seq: u64,
    pub timestamp_unix_ms: u64,
    pub source: EventSource,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Runtime,
    ProviderProjection,
    IntentExecutor,
    Orchestrator,
    Custom(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    InvocationStarted {
        operator: OperatorId,
    },
    InferenceStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    InferenceCompleted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ToolCallAssembled {
        call_id: String,
        capability_id: String,
        input: serde_json::Value,
    },
    ToolResultReceived {
        call_id: String,
        capability_id: String,
        output: serde_json::Value,
    },
    IntentDeclared {
        intent: Intent,
    },
    Progress {
        content: Content,
    },
    ArtifactProduced {
        artifact: Artifact,
    },
    Log {
        level: String,
        message: String,
    },
    Observation {
        key: String,
        value: serde_json::Value,
    },
    Metric {
        name: String,
        value: f64,
        #[serde(default)]
        tags: std::collections::HashMap<String, String>,
    },
    Suspended {
        wait: WaitState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_request: Option<ApprovalRequest>,
    },
    Completed {
        output: OperatorOutput,
    },
    Failed {
        error: ProtocolError,
    },
}
```

Rules:

- `ExecutionEvent` is observational; no `EventKind` variant performs execution
- `event_id` is an opaque identifier; consumers MUST NOT parse it
- `EventMeta.seq` is authoritative for in-stream ordering; `timestamp_unix_ms`
  is informational and MUST NOT be used as the primary ordering key
- provider token deltas, SSE frames, partial JSON argument fragments, and raw
  `StreamEvent` payloads are not kernel semantic events
- `Completed` and `Failed` are the only terminal invocation-stream events
- `Suspended` is non-terminal stream observation; the terminal stream event for
  a successfully suspended immediate invocation is still
  `Completed { output }`, where `output.outcome` is `Outcome::Suspended`

### Event Source Use Rules

The following source assignments are locked:

- `InvocationStarted`, `Progress`, `IntentDeclared`, `ArtifactProduced`,
  `Log`, `Observation`, `Metric`, `Suspended`, and `Completed` emitted by the
  turn runtime use `EventSource::Runtime`
- `InferenceStarted`, `InferenceCompleted`, and `ToolCallAssembled` emitted
  from provider-stream projection use `EventSource::ProviderProjection`
- `ToolResultReceived` emitted after the runtime materializes a tool result
  uses `EventSource::Runtime`
- semantic events emitted while an outer executor is carrying out intents use
  `EventSource::IntentExecutor`
- durable/control-plane emitters that project invocation state outward use
  `EventSource::Orchestrator`

### Operator Output

`OperatorOutput` keeps the role and field layout from `specs/v2/15` except the
declaration field.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorOutput {
    pub message: Content,
    pub outcome: Outcome,
    pub metadata: OperatorMetadata,
    #[serde(default)]
    pub intents: Vec<Intent>,
}
```

Rules:

- canonical writers MUST emit `intents`
- canonical `v2` surface MUST NOT expose `effects` as a public output field
- `OperatorOutput::new(...)` initializes `intents` to an empty vector
- a normal operator execution that ends in
  `Outcome::Terminal { terminal: Failed { error } }` still returns a valid
  `OperatorOutput` and therefore emits `EventKind::Completed { output }`, not
  `EventKind::Failed`

### Invocation Stream Item

`InvocationHandle` streams `ExecutionEvent`.

```rust
pub struct CollectedInvocation {
    pub output: OperatorOutput,
    pub events: Vec<ExecutionEvent>,
}
```

Required methods:

- `recv(&mut self) -> Option<ExecutionEvent>`
- `cancel(&self)`
- `cancel_rx(&self) -> watch::Receiver<bool>`
- `collect(self) -> Result<OperatorOutput, ProtocolError>`
- `collect_all(self) -> Result<CollectedInvocation, ProtocolError>`
- `intercept(F) -> InvocationHandle` remains available as a convenience helper
  over `ExecutionEvent`

Locked collection behavior:

1. `EventKind::Completed { output }` is the only successful terminal stream
   event.
2. `EventKind::Failed { error }` returns `Err(error)` directly.
3. `Completed.output` is authoritative for `message`, `outcome`, and
   `metadata`.
4. If `Completed.output.intents` is empty, `collect()` and `collect_all()`
   backfill it from prior streamed `IntentDeclared` events in stream order.
5. If `Completed.output.intents` is non-empty, collectors MUST preserve that
   list and MUST NOT append duplicate streamed intents.
6. `collect_all()` returns only non-terminal events in `CollectedInvocation.events`;
   the terminal `Completed` or `Failed` event is not included in that vector.
7. missing terminal event after requested cancellation returns
   `ProtocolError { code: Cancelled, retryable: false, ... }`.
8. missing terminal event without requested cancellation returns
   `ProtocolError { code: Internal, retryable: false, ... }`.

### Invocation Sender

`InvocationSender` remains the low-level public sender in this slice.

Rules:

- `InvocationSender::send(...)` changes to accept `ExecutionEvent`
- `EffectEmitter` is removed with no public replacement
- callers that need to emit stream items directly MUST either:
  - construct `ExecutionEvent` and send it via `InvocationSender`, or
  - use a crate-private helper local to the runtime/orchestrator

No public `EventEmitter` is introduced in this slice.

## Runtime and Projection Rules

### Context API

Runtime declaration APIs are locked as:

| Surface | Required shape |
|---|---|
| declaration append | `ctx.push_intent(intent)` |
| declaration append many | `ctx.extend_intents(intents)` |
| declaration inspection | `ctx.intents()` |
| declaration drain | `ctx.drain_intents()` |
| context observation | `ContextMutation::IntentDeclared(intent)` |

Rules:

- no public `push_effect`/`extend_effects`/`effects`/`drain_effects` API may
  remain on merged `Context`
- `ContextMutation::InferenceDelta(StreamEvent)` remains runtime-local and is
  not a kernel `ExecutionEvent`
- `ctx.push_intent(...)` MUST emit `ContextMutation::IntentDeclared(...)`
  immediately at declaration time

### Provider Projection Boundaries

Projection from provider transport to semantic events is locked:

1. the runtime emits `InvocationStarted` once, before any inference event
2. the runtime emits `InferenceStarted` once per provider inference call,
   before consuming provider deltas for that call
3. the runtime emits `ToolCallAssembled` only when a full call ID, capability
   ID, and complete JSON input are available
4. the runtime emits `ToolResultReceived` only after the tool result
   materializes and before it is injected back into context
5. the runtime emits `InferenceCompleted` once per provider inference call,
   after the final assistant response for that call is assembled
6. the runtime emits `IntentDeclared` at the point the executable intent is
   declared, not only at terminal collection time
7. the runtime emits `Completed` or `Failed` exactly once per invocation stream

No `ExecutionEvent` variant may encode raw provider chunk bytes, transport
frame boundaries, or partial argument fragments as the canonical payload.

## Exact Replacement Rules

### Removed Public Surfaces

The following public replacements are locked:

| Remove | Replace with |
|---|---|
| `Effect` | `Intent` |
| `EffectKind` | `IntentKind` for executable declarations, `EventKind` for observational payloads |
| `EffectMeta` | `IntentMeta` |
| `OperatorOutput.effects` | `OperatorOutput.intents` |
| `DispatchEvent` | `ExecutionEvent` |
| `DispatchEvent::EffectEmitted` | `EventKind::IntentDeclared` or an observational `EventKind` |
| `EffectEmitter` | no public replacement; use `InvocationSender<ExecutionEvent>` or crate-private helpers |
| public `layer0::effect::*` helper nouns | public `layer0::intent::*` helper nouns only |

### Current `EffectKind` Mapping

| Current `EffectKind` | Replacement |
|---|---|
| `WriteMemory` | `IntentKind::WriteMemory` |
| `DeleteMemory` | `IntentKind::DeleteMemory` |
| `LinkMemory` | `IntentKind::LinkMemory` |
| `UnlinkMemory` | `IntentKind::UnlinkMemory` |
| `Signal` | `IntentKind::Signal` |
| `Delegate` | `IntentKind::Delegate` |
| `Handoff` | `IntentKind::Handoff` |
| `ToolApprovalRequired` | `IntentKind::RequestApproval` |
| `Custom` | `IntentKind::Custom` |
| `Progress` | `EventKind::Progress` |
| `Artifact` | `EventKind::ArtifactProduced` |
| `Log` | `EventKind::Log` |
| `Observation` | `EventKind::Observation` |
| `Metric` | `EventKind::Metric` |

Rules:

- observational variants (`Progress`, `Artifact`, `Log`, `Observation`,
  `Metric`) MUST NOT be represented as intents in merged `v2` code
- `ToolApprovalRequired` maps only to `RequestApproval`; the corresponding
  wait observation is a separate `ExecutionEvent::Suspended`

### Current `DispatchEvent` Mapping

| Current `DispatchEvent` | Replacement `EventKind` |
|---|---|
| `Progress { content }` | `Progress { content }` |
| `ArtifactProduced { artifact }` | `ArtifactProduced { artifact }` |
| `EffectEmitted { effect }` executable effect | `IntentDeclared { intent }` |
| `EffectEmitted { effect }` observational effect | corresponding observational `EventKind` |
| `Completed { output }` | `Completed { output }` |
| `Failed { error }` | `Failed { error }` |
| `AwaitingApproval(request)` | `Suspended { wait: WaitState { reason: Approval }, approval_request: Some(request) }` |

### Failure vs Completed-Output Rule

The stream failure rule is locked:

- `EventKind::Failed { error }` is for invocation-protocol failure where no
  valid `OperatorOutput` is available at the dispatch boundary
- any operator execution that returns a valid `OperatorOutput`, including one
  whose `outcome` is `Outcome::Terminal { terminal: Failed { error } }`, MUST
  terminate the stream with `EventKind::Completed { output }`

This preserves the distinction established in
`specs/v2/15-layer0-invocation-outcome-migration-annex.md` between protocol
failure and a modeled terminal outcome.

## Migration Path and Private Shims

The implementation PR adopting this annex MUST follow this migration order.

1. Make `Intent` and `ExecutionEvent` the canonical public `layer0` surface.
2. Switch `InvocationHandle` and `InvocationSender` to `ExecutionEvent`.
3. Stop emitting `DispatchEvent` from public dispatch paths.
4. Stop serializing `OperatorOutput.effects`.
5. Remove public `Effect`, `DispatchEvent`, and `EffectEmitter` re-exports.
6. Remove any remaining branch-only adapters before merge unless this annex
   explicitly keeps them.

### Allowed Branch-Only Adapters

Only the following compatibility posture is allowed during the cutover branch:

- `op/skg-context-engine::react::project_intents_to_effects(...)` or an
  equivalent crate-private helper MAY temporarily exist while downstream
  orchestrators still consume effects
- `op/skg-context-engine::react::finalize_operator_output(...)` MAY
  temporarily dual-write `output.intents` and `output.effects` inside the
  branch while those downstream callers are being migrated
- `layer0::dispatch` MAY temporarily use a crate-private helper that projects
  `DispatchEvent` into `ExecutionEvent` while send sites are being updated
- `layer0::effect` MAY temporarily keep one-way private mapping helpers from
  legacy effect helper nouns into `layer0::intent` helper nouns

Rules for those adapters:

- they MUST remain `pub(crate)` or private
- they MUST be one-way projections from legacy to canonical nouns
- they MUST NOT be re-exported from `layer0::lib`
- they MUST NOT define public `From`/`Into` impls across legacy and canonical
  types
- they MUST be removed before merge of the cutover PR

### What Stays Temporarily vs What Is Replaced

| Surface | Branch-only shim allowed? | Merged `v2` result |
|---|---|---|
| `op/skg-context-engine` dual-write `intents` -> `effects` | Yes, private only | Removed |
| `layer0::effect` private mapping helpers | Yes, private only | Removed |
| `DispatchEvent` -> `ExecutionEvent` projection helper | Yes, private only | Removed |
| `RunUpdate` in `skg-run-core` | Yes, durable-specific type remains | Stays durable-specific |
| `ContextMutation::InferenceDelta` | Yes, runtime-local mutation remains | Stays runtime-local; not a kernel semantic event |
| public `Effect` / `DispatchEvent` / `EffectEmitter` | No | Replaced |

## Wire and Serde Rules

### Canonical v2 Shapes

Canonical writers MUST emit the following field names:

- `Intent.meta.intent_id`
- `Intent.kind`
- `ExecutionEvent.meta.event_id`
- `ExecutionEvent.meta.dispatch_id`
- `ExecutionEvent.meta.parent_dispatch_id`
- `ExecutionEvent.meta.correlation_id`
- `ExecutionEvent.meta.seq`
- `ExecutionEvent.meta.timestamp_unix_ms`
- `ExecutionEvent.meta.source`
- `ExecutionEvent.kind`
- `OperatorOutput.intents`
- `EventKind::Failed.error`

Examples:

```json
{
  "meta": {
    "intent_id": "intent-7",
    "causation_id": null,
    "correlation_id": "trace-123",
    "seq": 4
  },
  "kind": "request_approval",
  "tool_name": "shell.exec",
  "call_id": "call_9",
  "input": {
    "cmd": "rm -rf /tmp/example"
  }
}
```

```json
{
  "meta": {
    "event_id": "event-21",
    "dispatch_id": "disp_123",
    "parent_dispatch_id": null,
    "correlation_id": "trace-123",
    "seq": 9,
    "timestamp_unix_ms": 1760000000000,
    "source": "runtime"
  },
  "kind": "intent_declared",
  "intent": {
    "meta": {
      "intent_id": "intent-7",
      "causation_id": null,
      "correlation_id": "trace-123",
      "seq": 4
    },
    "kind": "request_approval",
    "tool_name": "shell.exec",
    "call_id": "call_9",
    "input": {
      "cmd": "rm -rf /tmp/example"
    }
  }
}
```

```json
{
  "meta": {
    "event_id": "event-22",
    "dispatch_id": "disp_123",
    "parent_dispatch_id": null,
    "correlation_id": "trace-123",
    "seq": 10,
    "timestamp_unix_ms": 1760000000001,
    "source": "runtime"
  },
  "kind": "suspended",
  "wait": {
    "reason": "approval"
  },
  "approval_request": {
    "reason": "destructive_action",
    "calls": []
  }
}
```

### No Legacy Shape Requirement

Because this slice is a breaking cutover:

- canonical `v2` deserializers are only required to accept canonical `v2`
  `Intent` and `ExecutionEvent` shapes
- legacy `effect`, `dispatch_event`, or stringified event-payload failures are
  not part of the merged `v2` contract
- if a backend or script must ingest pre-`v2` persisted data, that belongs in
  migration tooling or private bridge code outside the canonical `layer0`
  contract

## No Public Shim Rules

The merged `v2` surface MUST NOT include:

- public `Effect`, `EffectKind`, or `EffectMeta`
- public `DispatchEvent`
- public `EffectEmitter`
- public `OperatorOutput.effects`
- public effect-oriented `Context` APIs
- public compatibility aliases or public `From` impls between legacy and
  canonical event/intent surfaces

## Proving Tests

The implementation PR adopting this annex MUST add all of the following.

### `layer0`

- `layer0/tests/v2_intent_wire.rs`
  - each `IntentKind` variant round-trips with the locked wire shape
  - observational payloads are not representable as `IntentKind`
- `layer0/tests/v2_execution_event_wire.rs`
  - each `EventKind` variant round-trips with the locked wire shape
  - `InferenceCompleted` round-trips as a semantic event, not a transport chunk
- `layer0/tests/v2_invocation_event_collection.rs`
  - `collect()` backfills `OperatorOutput.intents` from `IntentDeclared` when
    the completed output omits them
  - `collect()` does not duplicate intents when `Completed.output.intents` is
    already populated
  - missing terminal event error mapping matches the locked rules
- `layer0/tests/v2_terminal_event_semantics.rs`
  - `EventKind::Failed` is used only for protocol failure
  - `Outcome::Terminal::Failed` still travels through
    `EventKind::Completed { output }`

### `op/skg-context-engine`

- `op/skg-context-engine/tests/v2_intent_context_api.rs`
  - `push_intent`/`extend_intents`/`drain_intents` behave as locked
  - no effect-declaration API remains on merged public `Context`
- `op/skg-context-engine/tests/v2_projection_semantics.rs`
  - provider deltas are projected only at the locked semantic boundaries
  - streaming and collected paths emit equivalent outcomes and semantic events
- branch-only adapter tests while the branch is in flight
  - the private `project_intents_to_effects(...)` bridge, if present, maps only
    executable intents and never observational payloads
  - those tests MUST be removed with the adapter before merge

## Golden Fixtures

The implementation PR adopting this annex MUST add JSON fixtures at these exact
paths:

- `layer0/tests/golden/v2/intent-write-memory.json`
- `layer0/tests/golden/v2/intent-request-approval.json`
- `layer0/tests/golden/v2/execution-event-intent-declared.json`
- `layer0/tests/golden/v2/execution-event-suspended-approval.json`
- `layer0/tests/golden/v2/execution-event-completed.json`
- `layer0/tests/golden/v2/execution-event-failed.json`

Fixture rules:

- fixtures MUST use canonical `v2` field names
- fixtures MUST NOT encode `effect` or `dispatch_event` legacy shapes
- fixtures MUST NOT encode raw provider chunk payloads

## Explicit Non-Goals

This annex does not authorize:

- raw provider chunk passthrough as kernel semantic events
- one universal observer runtime inside Layer 0
- durable run read-model replacement with `ExecutionEvent`
- moving durable wait-point identity or lifecycle storage into `layer0`
- capability-discovery redesign
- policy decisions about who may observe or intervene

## Relationship to Existing Specs

This annex refines:

- `specs/v2/03-intents-and-semantic-events.md`
- `specs/v2/05-streaming-runtime-and-provider-projection.md`
- `specs/v2/12-observation-intervention-and-queries.md`
- `specs/v2/13-composition-patterns-and-control-surfaces.md`

For the `v2` track it supersedes:

- the `Effect`-as-catch-all portions of `specs/03-effects-and-execution-semantics.md`
- the `DispatchEvent` transition stream retained temporarily by
  `specs/v2/15-layer0-invocation-outcome-migration-annex.md`
