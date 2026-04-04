# Layer0 Invocation / Outcome Migration Annex

## Purpose

Define the first implementation-grade `v2` migration slice for `layer0`:

- replace `ExitReason` with `Outcome`
- replace `DispatchHandle` with `InvocationHandle`
- move shared wait nouns into `layer0`
- replace protocol-visible stringified failures with `ProtocolError`
- define the exact replacement rules for current `layer0` and `skg-run-core`
  surfaces

This annex is implementation-authoritative for the first `v2` cutover slice
once a future change adopts the `v2` track.

## Migration Posture

This slice assumes a deliberate breaking cutover.

Rules:

- the merged public kernel surface MUST expose the `v2` names and wire shapes
  only
- superseded `v1` names are removed rather than kept as deprecated public
  adapters
- canonical `v2` serde behavior does not have to accept `v1` field names or
  old string failure payloads
- private one-off refactor helpers are allowed while the branch is in flight,
  but they are migration scaffolding only and MUST be removed before merge

This repo currently has no downstream compatibility obligation strong enough to
justify carrying public adapter debt into the `v2` kernel.

## Scope

This slice is intentionally narrow.

It covers:

- immediate invocation result typing
- immediate handle naming and collection semantics
- shared wait nouns across immediate and durable paths
- structured protocol-visible failures for invocation surfaces
- durable read-model alignment needed to reuse the shared wait and error nouns

It does not cover:

- the full `ExecutionEvent` replacement for `DispatchEvent`
- the `Effect` -> `Intent` / observation split from `v2/03`
- durable backend internals such as checkpoints, leases, timers, or replay
- state/environment trait-family redesign
- a full replacement of durable run read models with one shared `Outcome`
  wrapper

## Ownership

| Crate | Module | Owns in this slice |
|---|---|---|
| `layer0` | `src/operator.rs` | `Outcome`, `TerminalOutcome`, `TransferOutcome`, `LimitReason`, `InterceptionKind`, `OperatorOutput.outcome`, removal of `ExitReason` from the public surface |
| `layer0` | `src/dispatch.rs` | `Dispatcher` return type update, `InvocationHandle`, `CollectedInvocation`, temporary `DispatchEvent` transition stream, removal of `DispatchHandle` / `CollectedDispatch` |
| `layer0` | `src/wait.rs` | `WaitReason`, `WaitState`, `ResumeInput` |
| `layer0` | `src/error.rs` | `ProtocolError`, `ErrorCode`, replacement of public invocation-facing `OperatorError` / `OrchError` usage |
| `orch/skg-run-core` | `src/wait.rs` | re-export of `layer0::wait::{ResumeInput, WaitReason}`; local duplicate definitions removed |
| `orch/skg-run-core` | `src/model.rs` | durable read-model failure fields upgraded to `ProtocolError`; waiting fields use shared `layer0::wait::WaitReason` |
| `orch/skg-run-core` | `src/kernel.rs` and `src/command.rs` | failure payloads upgraded to `ProtocolError`; no new lifecycle substrate added |

No new wait or outcome nouns may be duplicated in `skg-run-core` after this
slice lands.

## Canonical Public Types

### Shared Wait Vocabulary

`WaitReason` and `ResumeInput` move into `layer0`. `WaitState` is new and
intentionally small.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Approval,
    ExternalInput,
    Timer,
    ChildRun,
    Custom(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitState {
    pub reason: WaitReason,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeInput {
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}
```

Rules:

- `WaitState` is the shared value-level fact that an invocation suspended
- immediate approval details continue to travel through
  `ApprovalRequest` and `DispatchEvent::AwaitingApproval` in this slice
- durable wait-point identity remains durable-only and stays in
  `skg-run-core::WaitPointId`
- durable timer persistence stays above `layer0`; this slice does not move
  `PortableWakeDeadline`

### Outcome Family

`Outcome` replaces `ExitReason` as the canonical reason an invocation ended.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Terminal {
        terminal: TerminalOutcome,
    },
    Suspended {
        wait: WaitState,
    },
    Transferred {
        transfer: TransferOutcome,
    },
    Limited {
        limit: LimitReason,
    },
    Intercepted {
        interception: InterceptionKind,
        reason: String,
    },
    Custom {
        name: String,
        #[serde(default)]
        details: serde_json::Value,
    },
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalOutcome {
    Completed,
    Failed {
        error: ProtocolError,
    },
    Cancelled,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransferOutcome {
    Handoff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operator: Option<OperatorId>,
    },
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitReason {
    TurnLimit,
    Budget,
    Timeout,
    CircuitBreaker,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterceptionKind {
    Policy,
    Safety,
}
```

Variant semantics are locked:

- `Terminal.Completed`: the invocation completed naturally and produced its
  normal `OperatorOutput.message`
- `Terminal.Failed`: execution started, reached a terminal failure, and that
  failure is part of the final outcome model
- `Terminal.Cancelled`: cooperative cancellation won and produced an explicit
  terminal outcome
- `Suspended`: a shared wait reason is active
- `Transferred`: the current operator relinquished control; in this slice that
  means handoff only
- `Limited`: execution hit a bounded runtime limit, not a policy or safety stop
- `Intercepted`: execution was stopped by policy or safety, not by a resource
  limit and not by a wait
- `Custom`: explicit escape hatch for a named domain outcome

### Operator Output

`OperatorOutput` keeps its current role and field layout except for the outcome
field.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorOutput {
    pub message: Content,
    pub outcome: Outcome,
    pub metadata: OperatorMetadata,
    #[serde(default)]
    pub effects: Vec<Effect>,
}
```

Rules:

- `effects` remains the field name in this slice; the `Effect` -> `Intent`
  replacement is a later `v2/03` adoption step
- `OperatorOutput::new(...)` changes to take `Outcome`
- canonical writers MUST emit `outcome`, not `exit_reason`

### Protocol Error

`ProtocolError` is the canonical serializable failure surface for immediate
invocation APIs and event payloads.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub details: serde_json::Value,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    Cancelled,
    Timeout,
    Unavailable,
    ResourceExhausted,
    Unsupported,
    PermissionDenied,
    Internal,
}
```

Rules:

- `code` is the machine-readable classifier; no caller may parse `message`
- `retryable` is explicit; callers must not infer retryability from `code`
  alone
- canonical writers MUST emit `details` as a JSON object; deserializers MUST
  accept any JSON value for forward compatibility inside the `v2` line
- `Display` output remains log-oriented only and is not part of the wire
  contract

### Invocation Handle

`InvocationHandle` is the canonical immediate handle name in `layer0`.

```rust
#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<InvocationHandle, ProtocolError>;
}

pub struct InvocationHandle {
    pub id: DispatchId,
    /* private channel fields */
}

pub struct CollectedInvocation {
    pub output: OperatorOutput,
    pub events: Vec<DispatchEvent>,
}
```

In this slice `InvocationHandle` is stream-first by name and collection rules,
but its event item remains the temporary `DispatchEvent` transition stream until
the `v2/03` semantic-event slice lands.

Required methods:

- `recv(&mut self) -> Option<DispatchEvent>`
- `cancel(&self)`
- `cancel_rx(&self) -> watch::Receiver<bool>`
- `collect(self) -> Result<OperatorOutput, ProtocolError>`
- `collect_all(self) -> Result<CollectedInvocation, ProtocolError>`
- `intercept(F) -> InvocationHandle` remains available as a convenience helper
  over `DispatchEvent`

Locked collection behavior:

1. `collect()` merges any streamed `EffectEmitted` values into
   `OperatorOutput.effects`, exactly as the current `DispatchHandle` does.
2. `DispatchEvent::Completed { output }` is the only successful terminal event.
3. `DispatchEvent::Failed { error }` returns `Err(error)` directly.
4. If the stream closes without a terminal event after the caller requested
   cancellation, `collect()` returns
   `ProtocolError { code: Cancelled, retryable: false, ... }`.
5. If the stream closes without a terminal event and no cancellation was
   requested, `collect()` returns
   `ProtocolError { code: Internal, retryable: false, ... }`.

## Trait Signatures

The public invocation-facing trait signatures change in this slice.

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError>;
}

#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<InvocationHandle, ProtocolError>;
}
```

Rules:

- `ProtocolError` replaces `OperatorError` and `OrchError` at the public
  invocation boundary
- `OperatorError` and `OrchError` do not remain as public deprecated surface in
  this slice
- `StateError`, `EnvError`, and `RunControlError` remain in place for their own
  non-invocation traits in this slice; they must be converted to
  `ProtocolError` when they cross an invocation boundary

## Wire and Serde Rules

### Canonical v2 Shapes

Canonical writers MUST emit the following field names:

- `OperatorOutput.outcome`
- `Outcome.kind`
- `TerminalOutcome.kind`
- `TransferOutcome.kind`
- `ProtocolError.code`
- `ProtocolError.message`
- `ProtocolError.retryable`
- `ProtocolError.details`
- `DispatchEvent::Failed.error`

Examples:

```json
{
  "kind": "terminal",
  "terminal": {
    "kind": "failed",
    "error": {
      "code": "unavailable",
      "message": "provider unavailable",
      "retryable": true,
      "details": {
        "provider": "anthropic"
      }
    }
  }
}
```

```json
{
  "message": "waiting for approval",
  "outcome": {
    "kind": "suspended",
    "wait": {
      "reason": "approval"
    }
  },
  "metadata": {
    "tokens_in": 0,
    "tokens_out": 0,
    "cost": "0",
    "turns_used": 0,
    "sub_dispatches": [],
    "duration": 0
  },
  "effects": []
}
```

```json
{
  "type": "failed",
  "error": {
    "code": "internal",
    "message": "dispatch ended without terminal outcome",
    "retryable": false,
    "details": {
      "kind": "missing_terminal_event"
    }
  }
}
```

### No Legacy Shape Requirement

Because this slice is a breaking cutover:

- canonical `v2` deserializers are only required to accept the canonical `v2`
  shapes defined in this annex
- `OperatorOutput.exit_reason` is not part of the merged `v2` serde contract
- old string-based failure payloads are not part of the merged `v2` serde
  contract
- if a backend or script must ingest pre-`v2` persisted data, that belongs in
  migration tooling or private bridge code outside the canonical `layer0`
  contract

## Exact Replacement Rules

### Removed Public Surfaces

The following public replacements are locked:

| Remove | Replace with |
|---|---|
| `ExitReason` | `Outcome` |
| `OperatorOutput.exit_reason` | `OperatorOutput.outcome` |
| `DispatchHandle` | `InvocationHandle` |
| `CollectedDispatch` | `CollectedInvocation` |
| `Dispatcher::dispatch(...) -> Result<DispatchHandle, OrchError>` | `Dispatcher::dispatch(...) -> Result<InvocationHandle, ProtocolError>` |
| `Operator::execute(...) -> Result<OperatorOutput, OperatorError>` | `Operator::execute(...) -> Result<OperatorOutput, ProtocolError>` |
| `DispatchEvent::Failed { error: OrchError }` | `DispatchEvent::Failed { error: ProtocolError }` |
| local `skg-run-core::WaitReason` / `ResumeInput` definitions | `layer0::wait` source of truth plus `skg-run-core` re-export |
| durable string failure payloads | `ProtocolError` |
| public `layer0` re-exports of `OperatorError` / `OrchError` | no replacement; invocation-facing public API uses `ProtocolError` |

No public type alias may preserve the removed names in the merged `v2` surface.

### Current `ExitReason` Replacement Mapping

When refactoring current code paths that still branch on `ExitReason`, use the
following exact replacement mapping:

| Current `ExitReason` | Replacement `Outcome` |
|---|---|
| `Complete` | `Outcome::Terminal { terminal: Completed }` |
| `MaxTurns` | `Outcome::Limited { limit: TurnLimit }` |
| `BudgetExhausted` | `Outcome::Limited { limit: Budget }` |
| `CircuitBreaker` | `Outcome::Limited { limit: CircuitBreaker }` |
| `Timeout` | `Outcome::Limited { limit: Timeout }` |
| `InterceptorHalt { reason }` | `Outcome::Intercepted { interception: Policy, reason }` |
| `SafetyStop { reason }` | `Outcome::Intercepted { interception: Safety, reason }` |
| `AwaitingApproval` | `Outcome::Suspended { wait: WaitState { reason: Approval } }` |
| `HandedOff` | `Outcome::Transferred { transfer: Handoff { operator: first matching handoff effect target if present, otherwise None } }` |
| `Custom(name)` | `Outcome::Custom { name, details: {} }` |

Special rule for current `ExitReason::Error` sites:

- they MUST be replaced with
  `Outcome::Terminal { terminal: Failed { error: ProtocolError { ... } } }`
- merged `v2` code MUST carry a real `ProtocolError`
- placeholder "legacy error" payloads are prohibited in the merged `v2`
  surface

### Current Error Replacement Rules

When replacing current non-serializable invocation-facing errors with
`ProtocolError`, the implementation MUST use the following exact mapping.

If refactored code already holds a `ProtocolError`, pass it through unchanged.

#### Current `OperatorError` -> `ProtocolError`

| Current variant | `ErrorCode` | `retryable` | Required `details` keys |
|---|---|---|---|
| `Model { retryable: true, .. }` | `Unavailable` | `true` | `kind=operator_error`, `variant=model` |
| `Model { retryable: false, .. }` | `Internal` | `false` | `kind=operator_error`, `variant=model` |
| `SubDispatch { operator, .. }` | `Unavailable` | `false` | `kind=operator_error`, `variant=sub_dispatch`, `operator` |
| `ContextAssembly { .. }` | `InvalidInput` | `false` | `kind=operator_error`, `variant=context_assembly` |
| `Retryable { .. }` | `Unavailable` | `true` | `kind=operator_error`, `variant=retryable` |
| `NonRetryable { .. }` | `Internal` | `false` | `kind=operator_error`, `variant=non_retryable` |
| `Halted { .. }` | `Conflict` | `false` | `kind=operator_error`, `variant=halted` |
| `Other(..)` | `Internal` | `false` | `kind=operator_error`, `variant=other` |

#### Current `OrchError` -> `ProtocolError`

| Current variant | `ErrorCode` | `retryable` | Required `details` keys |
|---|---|---|---|
| `OperatorNotFound(name)` | `NotFound` | `false` | `kind=orch_error`, `variant=operator_not_found`, `name` |
| `WorkflowNotFound(name)` | `NotFound` | `false` | `kind=orch_error`, `variant=workflow_not_found`, `name` |
| `DispatchFailed(msg)` | `Unavailable` | `true` | `kind=orch_error`, `variant=dispatch_failed` |
| `SignalFailed(msg)` | `Unavailable` | `true` | `kind=orch_error`, `variant=signal_failed` |
| `OperatorError(inner)` | convert through the `OperatorError` table above | forwarded | preserve mapped details |
| `EnvironmentError(inner)` | convert through the `EnvError` table below | mapped | preserve mapped details |
| `Other(..)` | `Internal` | `false` | `kind=orch_error`, `variant=other` |

#### Current `EnvError` -> `ProtocolError`

| Current variant | `ErrorCode` | `retryable` | Required `details` keys |
|---|---|---|---|
| `ProvisionFailed(..)` | `Unavailable` | `true` | `kind=env_error`, `variant=provision_failed` |
| `IsolationViolation(..)` | `PermissionDenied` | `false` | `kind=env_error`, `variant=isolation_violation` |
| `CredentialFailed(..)` | `PermissionDenied` | `false` | `kind=env_error`, `variant=credential_failed` |
| `ResourceExceeded(..)` | `ResourceExhausted` | `false` | `kind=env_error`, `variant=resource_exceeded` |
| `OperatorError(inner)` | convert through the `OperatorError` table above | forwarded | preserve mapped details |
| `Other(..)` | `Internal` | `false` | `kind=env_error`, `variant=other` |

#### Current `StateError` -> `ProtocolError`

| Current variant | `ErrorCode` | `retryable` | Required `details` keys |
|---|---|---|---|
| `NotFound { scope, key }` | `NotFound` | `false` | `kind=state_error`, `variant=not_found`, `scope`, `key` |
| `WriteFailed(..)` | `Internal` | `false` | `kind=state_error`, `variant=write_failed` |
| `Serialization(..)` | `InvalidInput` | `false` | `kind=state_error`, `variant=serialization` |
| `Other(..)` | `Internal` | `false` | `kind=state_error`, `variant=other` |

## Immediate / Durable Alignment Rules

This slice locks the following immediate/durable alignment:

- immediate `Outcome::Suspended { wait }` and durable `RunView::Waiting` use
  the same `WaitReason`
- immediate resume payloads and durable resume payloads use the same
  `ResumeInput`
- immediate terminal failures and durable terminal failures use the same
  `ProtocolError`

This slice explicitly does not require:

- replacing `RunOutcome` with shared `Outcome`
- moving durable `RunStatus`, `RunView`, or `WaitPointId` into `layer0`
- introducing a single trait that merges immediate dispatch and durable control

## No Public Shim Rules

The merged `v2` surface MUST NOT include:

- public deprecated `ExitReason` or `DispatchHandle` aliases
- public `CollectedDispatch` alias
- public `layer0` invocation-facing `OperatorError` or `OrchError`
- `serde(alias)` support for `OperatorOutput.exit_reason` on canonical `v2`
  types
- canonical `DispatchEvent::Failed` serialization as a string payload
- canonical durable failure serialization as a string payload

`DispatchEvent` remains in this slice only because `v2/03` has not yet landed.
It is a temporary transition event stream, not a preserved `v1` compatibility
surface.

## Proving Tests

The implementation PR adopting this annex MUST add all of the following.

### `layer0`

- `layer0/tests/v2_outcome.rs`
  - canonical `Outcome` variants round-trip with the exact locked wire shape
  - handoff transfer derivation uses the first matching `EffectKind::Handoff`
    target when refactoring old handoff paths
  - `OperatorOutput` serializes `outcome` and does not serialize `exit_reason`
- `layer0/tests/v2_protocol_error.rs`
  - each current error replacement table entry produces the exact
    `ErrorCode`, `retryable`, and required `details` keys
- `layer0/tests/v2_invocation_handle.rs`
  - `collect()` merges streamed effects
  - missing terminal event after cancellation returns `ErrorCode::Cancelled`
  - missing terminal event without cancellation returns `ErrorCode::Internal`
- `layer0/tests/v2_dispatch_event_failed_wire.rs`
  - `DispatchEvent::Failed` round-trips with structured `ProtocolError`
  - serialized failure payload is an object, not a display string

### `orch/skg-run-core`

- `orch/skg-run-core/tests/v2_wait_alignment.rs`
  - `skg_run_core::WaitReason` and `skg_run_core::ResumeInput` are re-exports
    of the `layer0` types, not duplicate local definitions
  - waiting run views round-trip with the shared wait wire format
- `orch/skg-run-core/tests/v2_protocol_error_alignment.rs`
  - `RunOutcome::Failed`, `RunView::Failed`, `RunEvent::Fail`,
    `ResumeAction::Fail`, and `OrchestrationCommand::FailRun` round-trip with
    structured `ProtocolError`

No conformance test in this slice should require deserializing `v1` field names
or old string failure payloads.

## Golden Fixtures

The implementation PR adopting this annex MUST add JSON fixtures at these exact
paths:

- `layer0/tests/golden/v2/outcome-completed.json`
- `layer0/tests/golden/v2/outcome-approval-suspended.json`
- `layer0/tests/golden/v2/outcome-handoff-transferred.json`
- `layer0/tests/golden/v2/protocol-error-unavailable.json`
- `layer0/tests/golden/v2/dispatch-event-failed.json`
- `orch/skg-run-core/tests/golden/v2/run-view-waiting.json`
- `orch/skg-run-core/tests/golden/v2/run-outcome-failed.json`

Fixture rules:

- fixtures MUST use canonical `v2` field names
- no fixture in this slice may encode `v1` field names or string failure
  payloads
- no fixture in this slice may encode raw provider chunks or the future
  `ExecutionEvent` plane

## Explicit Non-Goals

This annex does not authorize any of the following:

- keeping `DispatchEvent` as the final `v2` event model
- treating observational `Effect` variants as blessed `v2` kernel semantics
- moving `RunControlError`, `RunStatus`, `RunView`, or `WaitPointId` into
  `layer0`
- adding approval-policy logic, resume-policy logic, or result-routing policy
  to `Outcome`
- standardizing durable wake storage or continue-as-new internals
- preserving `v1` public kernel names or wire forms beside the `v2` ones

## Relationship to Existing Specs

This annex refines `specs/v2/02-invocation-outcomes-and-waits.md`,
`specs/v2/09-durable-alignment-and-control.md`, and
`specs/v2/10-errors-versioning-and-conformance.md` for the first implementation
slice.

For the `v2` track it supersedes:

- the `ExitReason` portions of `specs/04-operator-turn-runtime.md`
- the immediate handle naming and stringified failure behavior in
  `layer0/src/dispatch.rs`
- the duplicated durable wait noun definitions in `orch/skg-run-core/src/wait.rs`
- the string failure payloads in `orch/skg-run-core` read models and durable
  kernel commands
