# V2 Full Cutover Design

## Goal

Migrate the entire skelegent workspace from v1 to v2 in one pass. When
complete: zero v1 public surface, all tests assert v2 contracts, every crate
compiles against v2 kernel types only.

## Authority

This design is grounded in the three v2 migration annexes:

- `specs/v2/15-layer0-invocation-outcome-migration-annex.md` — Outcome, waits,
  ProtocolError, InvocationHandle
- `specs/v2/16-intent-event-migration-annex.md` — Intent, ExecutionEvent,
  OperatorOutput.intents
- `specs/v2/17-capability-discovery-migration-annex.md` — CapabilitySource,
  CapabilityDescriptor

The annexes define exact replacement tables, wire shapes, proving tests, and
golden fixtures. This design does not override them — it extends them to cover
the full workspace ripple.

## Migration Posture

Deliberate breaking cutover. No public compatibility shims, no deprecated
aliases, no dual-carry of v1 and v2 types in the final merged state. The repo
has no downstream compatibility obligation.

During the branch, layer0 may temporarily keep `#[deprecated]` re-exports of
superseded v1 types so downstream crates compile at each commit boundary. These
are removed in the final commit before merge. This is consistent with the
annex posture: "private one-off refactor helpers are allowed while the branch
is in flight, but they are migration scaffolding only and MUST be removed
before merge."

## Approach

Big-bang kernel, then ripple outward. All three annexes land in layer0
simultaneously. Then consumers update crate by crate. Each commit must compile
and pass tests. Deprecated re-exports in layer0 bridge the gap until
downstream crates are updated.

## Scope Boundaries

This design covers:

- v2 type cutover across all workspace crates
- CapabilityDescriptor projection from existing tool abstractions
- Annex-required proving tests and golden fixtures
- Spec retirement and doc updates

This design does NOT cover:

- ToolDyn/ToolRegistry deletion — annex 17 scopes its slice to projection into
  CapabilityDescriptor, not deletion of existing tool abstractions. ToolDyn
  removal is a separate follow-on decision.
- Orchestration redesign — mechanical type swap only
- New features enabled by v2 types
- A2A bridge implementation

## Commit Structure

### Commit 1: Layer0 kernel cutover

Wire in v2 modules. Add v2 types to public surface. Keep `#[deprecated]`
re-exports of superseded v1 types so downstream crates still compile.

**Wire in (new modules already on branch):**

- `src/intent.rs` — Intent, IntentMeta, IntentKind, Scope, MemoryScope,
  SignalPayload, HandoffContext
- `src/event.rs` — ExecutionEvent, EventMeta, EventSource, EventKind
- `src/capability.rs` — CapabilitySource, CapabilityDescriptor,
  CapabilityFilter, supporting types
- `src/wait.rs` — WaitReason, WaitState, ResumeInput

**Modify `src/operator.rs`:**

- Add Outcome family (already exists from stash)
- OperatorOutput: add `intents: Vec<Intent>` alongside existing `effects`
  (dual-carry temporarily; effects removed when downstream crates are updated)
- Operator::execute return: `Result<OperatorOutput, OperatorError>` →
  `Result<OperatorOutput, ProtocolError>`
- Mark ExitReason `#[deprecated]`

**Modify `src/dispatch.rs`:**

- Add InvocationHandle, CollectedInvocation as v2 names
- Dispatcher::dispatch return: → `Result<InvocationHandle, ProtocolError>`
- Stream item: → ExecutionEvent
- InvocationSender::send accepts ExecutionEvent
- Mark DispatchHandle, CollectedDispatch, DispatchEvent, EffectEmitter
  `#[deprecated]`

**Modify `src/error.rs`:**

- Add ProtocolError and ErrorCode as canonical invocation-facing error
- Add From impls per exact mapping tables in annex 15
- Mark OperatorError and OrchError `#[deprecated]`

**Modify `src/lib.rs` re-exports:**

- Add all v2 types to public surface
- Keep v1 types as `#[deprecated]` re-exports temporarily

**Remove modules:**

- `src/effect_middleware.rs` — removed entirely
- `src/effect_log.rs` — convert to intent log or remove

**Update:**

- `src/approval.rs` — stays, referenced by EventKind::Suspended
- `src/middleware.rs` — use ProtocolError at boundaries
- `src/state.rs` — stays, StateError remains for non-invocation paths
- `src/test_utils/` — rewrite against v2 types

### Commit 2: skg-run-core alignment

- `src/wait.rs` — remove local WaitReason/ResumeInput, re-export from layer0
- `src/model.rs` — failure fields → ProtocolError, waiting fields → shared
  WaitReason
- `src/kernel.rs` and `src/command.rs` — failure payloads → ProtocolError
- Rewrite `tests/kernel_transitions.rs`

### Commit 3: skg-context-engine cutover

- `src/context.rs` — push_effect → push_intent, extend_effects →
  extend_intents, effects → intents, drain_effects → drain_intents,
  ContextMutation::EffectDeclared → IntentDeclared
- `src/react.rs` and `src/stream_react.rs` — emit Intent and ExecutionEvent,
  make_output drains intents, provider projection follows annex 16 boundary
  rules
- `src/cognitive_operator.rs` — return Result<OperatorOutput, ProtocolError>
- `src/builder.rs` — updated to v2 types
- `src/ops/tool.rs` — emit ExecutionEvent for tool calls, RequestApproval +
  Suspended for approval
- `src/ops/store.rs` and `src/ops/cognitive.rs` — Effect → Intent
- `src/rules/budget.rs` — Outcome::Limited { limit: Budget }
- `src/stream.rs` — ContextMutation::IntentDeclared, InferenceDelta stays
  runtime-local
- Rewrite `tests/acc_integration_test.rs`

Terminal-stream rule (from annex 16): `EventKind::Suspended` is an
observational non-terminal event. The terminal stream event for a successfully
suspended invocation is `EventKind::Completed { output }` where
`output.outcome` is `Outcome::Suspended { wait }`. `EventKind::Failed` is
only for protocol failure where no valid OperatorOutput exists.

### Commit 4: Turn crates

**skg-tool:**

- `src/adapter.rs` — projection from ToolDyn/ToolRegistry into
  CapabilityDescriptor per annex 17 mapping table
- `src/memory.rs` — Intent for memory operations
- ToolDyn, ToolRegistry, AliasedTool — remain as execution abstractions;
  annex 17 does not authorize deletion

**skg-tool-macro:**

- Update to v2 types at boundaries (ProtocolError, etc.)

**skg-tool-openapi:**

- Project OpenAPI specs into CapabilityDescriptor

**skg-mcp:**

- `src/client.rs` — MCP discovery → CapabilityDescriptor, extras in extensions
- `src/server.rs` — serve discovery through CapabilitySource
- MCP bridge becomes CapabilitySource + Dispatcher impl

**skg-context:**

- `src/context_assembly.rs` — update to v2 types

### Commit 5: Orchestration crates

Mechanical type swap — no redesign.

- `orch/skg-orch-local` — v2 Dispatcher signature, ExecutionEvent stream
- `orch/skg-orch-kit` — runner consumes InvocationHandle/ExecutionEvent/
  CollectedInvocation
- `orch/skg-orch-env` — v2 Dispatcher, ExecutionEvent forwarding
- `orch/skg-orch-patterns` — all patterns updated to Outcome, IntentKind for
  handoff, CapabilityDescriptor for scheduling facts
- `runner/skg-runner` — event handling and SSE streaming switch to
  ExecutionEvent

### Commit 6: Hooks and effects crates

- `hooks/skg-hook-recorder` — record ExecutionEvent and Intent
- `hooks/skg-hook-retry` — check Outcome and ProtocolError.retryable
- `hooks/skg-hook-security` — filter ExecutionEvent/Intent
- `effects/skg-effects-core` — handle Intent execution (keep crate name)
- `effects/skg-effects-local` — handler match arms switch to IntentKind,
  delete observational variant handling

### Commit 7: State, env, remaining crates

- `state/skg-state-memory`, `state/skg-state-fs`, `state/skg-state-proxy` —
  StateError internally, ProtocolError at boundaries
- `env/skg-env-local` — EnvError internally, ProtocolError at boundaries
- `op/skg-op-single-shot` — update to v2 types (Outcome, ProtocolError)
- `skelegent/` umbrella — re-export v2 surface only, update Cargo.toml and
  agent.rs

### Commit 8: Examples

Rewrite all examples to v2 types: echo-agent, react-chatbot, multi-agent,
workflow, middleware_echo, middleware_recorder, middleware_approval,
custom_operator_barrier.

### Commit 9: Tests

Delete all v1-only test files. Add all proving tests required by annexes:

**Annex 15 (layer0):**

- `layer0/tests/v2_outcome.rs`
- `layer0/tests/v2_protocol_error.rs`
- `layer0/tests/v2_invocation_handle.rs`
- `layer0/tests/v2_dispatch_event_failed_wire.rs`

**Annex 15 (skg-run-core):**

- `orch/skg-run-core/tests/v2_wait_alignment.rs`
- `orch/skg-run-core/tests/v2_protocol_error_alignment.rs`

**Annex 16 (layer0):**

- `layer0/tests/v2_intent_wire.rs`
- `layer0/tests/v2_execution_event_wire.rs`
- `layer0/tests/v2_invocation_event_collection.rs`
- `layer0/tests/v2_terminal_event_semantics.rs`

**Annex 16 (skg-context-engine):**

- `op/skg-context-engine/tests/v2_intent_context_api.rs`
- `op/skg-context-engine/tests/v2_projection_semantics.rs`

**Annex 17:**

- `layer0/tests/v2_capability_descriptor_wire.rs`
- `layer0/tests/v2_dispatcher_discovery_boundary.rs`
- `turn/skg-tool/tests/v2_tool_descriptor_projection.rs`
- `turn/skg-mcp/tests/v2_mcp_descriptor_projection.rs`

**Golden fixtures** at all paths defined in the three annexes.

### Commit 10: Strip deprecated v1 re-exports

Remove all `#[deprecated]` v1 type re-exports from layer0. Remove
`src/effect.rs` entirely. Verify zero deprecated warnings across workspace.

This is the commit that enforces the annex "no public shim" rules. After this
commit, the public layer0 surface exposes only v2 names.

### Commit 11: Docs, specs, and AGENTS.md

- Add retirement sentinel to superseded specs (see Spec Retirement below)
- `AGENTS.md` / `CLAUDE.md` — update Key Abstractions table to v2 types,
  remove v2-as-separate-track framing, update load order
- `ARCHITECTURE.md` — update to reflect v2 as current
- `SPECS.md` — update index, mark retired specs
- `docs/book/` — all guides and reference pages updated
- `docs/design/23-decision-surface.md` — update v1 noun references
- `llms.txt` — update crate map
- Remove `scripts/verify.sh` (direnv + bare cargo is the canonical workflow)
- Update `rules/02-verification-and-nix.md` to reflect direnv workflow

## Spec Retirement

Every superseded spec in `specs/` gets this exact sentinel as the first line:

```
> **RETIRED — superseded by specs/v2/. Do not use for new implementation work.**
```

Retired specs (per migration matrix in `specs/v2/00`):

- `specs/00-vision-and-non-goals.md`
- `specs/01-architecture-and-layering.md`
- `specs/02-layer0-protocol-contract.md`
- `specs/03-effects-and-execution-semantics.md`
- `specs/04-operator-turn-runtime.md`
- `specs/05-orchestration-core.md`
- `specs/07-state-core.md`
- `specs/08-environment-and-credentials.md`
- `specs/14-durable-orchestration-core.md`

Specs that stay active (no v2 successor yet):

- `specs/06-composition-factory-and-glue.md`
- `specs/09-hooks-lifecycle-and-governance.md`
- `specs/10-secrets-auth-crypto.md`
- `specs/11-testing-examples-and-backpressure.md`
- `specs/12-packaging-versioning-and-umbrella-crate.md`
- `specs/13-documentation-and-dx-parity.md`

Done in commit 11 alongside the docs update so authority docs and spec
retirement land together.

## Verification

Each commit must pass:

```
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

The repo uses direnv to populate Nix-provided Rust tooling into the shell.
Run `direnv allow` once per session; bare `cargo` commands work after that.

The final state must have zero deprecated warnings from v1 types.
