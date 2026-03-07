> **SUPERSEDED:** The Hook system described here is being replaced by per-boundary
> continuation-based middleware. See `docs/plans/MIDDLEWARE-REDESIGN-BRIEFING.md`.
> This guide remains for reference until migration is complete.

# Hooks

> **Note:** The hook system's patterns are still evolving. This page provides a summary of the current design. For the full specification, see `specs/09-hooks-lifecycle-and-governance.md` in the repository.

Hooks provide observation and intervention at defined points inside the operator's inner loop. They fire before and after model inference, before and after tool execution, and at exit-condition checks.

## Overview

The `Hook` trait (defined in `layer0::hook`) declares which hook points an implementation listens to and what action to take when an event fires:

```rust
#[async_trait]
pub trait Hook: Send + Sync {
    fn points(&self) -> &[HookPoint];
    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError>;
}
```

The hook points are: `PreInference`, `PostInference`, `PreSubDispatch`, `PostSubDispatch`, `ExitCheck`, `SubDispatchUpdate`, `PreSteeringInject`, and `PostSteeringSkip`.

A hook can:
- **Observe** -- Log, emit telemetry, track metrics (return `HookAction::Continue`).
- **Halt** -- Stop execution with a reason (return `HookAction::Halt`).
- **Skip a sub-dispatch** -- Prevent a sub-dispatch (return `HookAction::SkipDispatch` at `PreSubDispatch`).
- **Modify input/output** -- Sanitize dispatch input or redact dispatch output (return `ModifyDispatchInput` or `ModifyDispatchOutput`).

Hook errors are logged but do not halt execution. Use `HookAction::Halt` to halt.

## HookRegistry (`neuron-hooks`)

The `HookRegistry` collects hooks into a kind-aware three-phase pipeline. At each hook point, hooks run in this order:

1. **Observers** — all run; returned actions and errors are discarded.
2. **Transformers** — each sees the context modified by the previous transformer; a `Halt` escalates immediately.
3. **Guardrails** — run against the original (pre-transformer) context; short-circuit on the first `Halt` or `SkipDispatch`.

```rust,no_run
use neuron_hooks::HookRegistry;
use std::sync::Arc;

let mut registry = HookRegistry::new();
registry.add_guardrail(Arc::new(budget_hook));
registry.add_transformer(Arc::new(sanitizer_hook));
registry.add_observer(Arc::new(logging_hook));
```

Both `ReactOperator` and `SingleShotOperator` accept a `HookRegistry` at construction time and dispatch events through it during execution.

## HookKind: composition rules

Every hook is registered with a `HookKind` that controls how its action composes with other hooks at the same point.

| Kind | When to use | On `Halt` | On error |
|------|-------------|-----------|----------|
| `Guardrail` | Policy enforcement — block or skip tools, halt the turn | Short-circuits; subsequent guardrails do not run | Logged via `tracing::warn`; pipeline continues |
| `Transformer` | Data rewriting — sanitize input, redact output | Escalates immediately (same as guardrail halt) | Logged via `tracing::warn`; pipeline continues |
| `Observer` | Telemetry, logging, metrics | Discarded; all observers run regardless | Logged via `tracing::warn`; all observers still run |

**Dispatch order within a single `dispatch` call:**
```
Observers (all run, actions discarded)
  → Transformers (chain in order; Halt escalates)
  → Guardrails (short-circuit on Halt or SkipDispatch)
```

Registration order within each phase matters. If two guardrails are registered, the first one to return `Halt` stops the second from running.
If you register a guardrail before an observer in the same `add` sequence, the observer still runs first because phases take precedence over registration order.

### Convenience registration methods

```rust,no_run
use neuron_hooks::{HookRegistry, HookKind};
use std::sync::Arc;

let mut registry = HookRegistry::new();

// Equivalent to registry.add(hook, HookKind::Guardrail)
registry.add_guardrail(Arc::new(my_policy_hook));

// Equivalent to registry.add(hook, HookKind::Transformer)
registry.add_transformer(Arc::new(my_sanitizer_hook));

// Equivalent to registry.add(hook, HookKind::Observer)
registry.add_observer(Arc::new(my_metrics_hook));

// Explicit kind — useful when kind is determined at runtime:
registry.add(Arc::new(my_hook), HookKind::Guardrail);
```

## Steering observability

`SteeringSource` and hooks are separate primitives with different control flows:

- **`SteeringSource`** is poll-driven: the operator calls `drain()` at batch boundaries and injects whatever messages it returns.
- **Hooks** are event-driven: the operator calls `on_event()` at defined `HookPoint`s during the loop.

Steering is observable *via* hooks, but it is not a `HookKind`. Four reasons:

1. **Different control flow** — steering is polling (`drain()` called repeatedly at boundaries); hooks are callbacks (`on_event()` fires once at a defined point).
2. **Different return types** — `drain()` returns `Vec<SteeringCommand>`; `on_event()` returns `HookAction`.
3. **Different composition** — steering messages are concatenated; hook actions short-circuit or chain.
4. **Different statefulness** — a steering source buffers messages between polls; hooks are stateless per call.

### Observing injection: `PreSteeringInject`

Fires after `drain()` returns a non-empty list, before the messages enter context. `ctx.steering_messages` holds the messages as debug-formatted strings. Guardrails can return `Halt` to block the injection entirely.

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct SteeringAuditHook;

#[async_trait]
impl Hook for SteeringAuditHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSteeringInject]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if let Some(msgs) = &ctx.steering_messages {
            for msg in msgs {
                tracing::info!(steering_message = %msg, "steering inject");
            }
        }
        Ok(HookAction::Continue) // return Halt to block injection
    }
}

// As an observer (logging only — cannot block injection):
// registry.add_observer(Arc::new(SteeringAuditHook));
//
// As a guardrail (can return Halt to block injection):
// registry.add_guardrail(Arc::new(SteeringAuditHook));
```

### Observing skipped tools: `PostSteeringSkip`

Fires after tools are skipped because steering messages were injected. `ctx.skipped_operators` holds the names of the operators that did not execute. This point is observation-only: `Halt` here halts the turn, but the skip already occurred.

```rust,no_run
use async_trait::async_trait;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::error::HookError;

struct SkipAuditHook;

#[async_trait]
impl Hook for SkipAuditHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PostSteeringSkip]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if let Some(skipped) = &ctx.skipped_operators {
            tracing::warn!(tools = ?skipped, "tools skipped by steering");
        }
        Ok(HookAction::Continue)
    }
}

// registry.add_observer(Arc::new(SkipAuditHook));
```

## Use cases

- **Budget enforcement** -- Track accumulated cost at `PostInference`, halt if over budget.
- **Guardrails** -- Validate sub-dispatches at `PreSubDispatch`, skip dangerous operations.
- **Telemetry** -- Emit OpenTelemetry spans at each hook point.
- **Heartbeat** -- Signal liveness to an orchestrator (e.g., Temporal heartbeat) at `PreInference`.
- **Secret redaction** -- Redact sensitive data from dispatch output at `PostSubDispatch`.

For security-focused hooks, see the `neuron-hook-security` crate.
