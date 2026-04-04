# Intents and Semantic Events

## Purpose

Separate executable intent from observation and define the one semantic event plane.

## Intent Model

V2 replaces `Effect` with `Intent` for executable side effects only:

```rust
pub struct Intent {
    pub meta: IntentMeta,
    pub kind: IntentKind,
}
```

`IntentKind` covers executable declarations such as:

- state writes and deletes
- delegate
- handoff
- signal
- approval requests
- custom executable intents

An intent is something an outer executor may carry out or persist for replay.

## Event Model

Observations become semantic `ExecutionEvent`s:

```rust
pub struct ExecutionEvent {
    pub meta: EventMeta,
    pub kind: EventKind,
}
```

The event plane covers:

- status changes
- inference start and completion
- tool call assembly and result receipt
- intent declaration
- artifact production
- logs, metrics, and observations
- final outcome emission

## Explicit Context APIs

The runtime must expose separate APIs:

- `ctx.push_intent(...)`
- `ctx.extend_intents(...)`
- `ctx.emit(...)`

This makes the split visible at the call site and removes the current ambiguity
where observational data travels through the same path as executable effects.

## Semantic Envelope Boundary

Provider token deltas are not semantic execution events.

Token chunks, partial JSON deltas, or transport chunk boundaries remain
provider-wire data consumed internally by the runtime. The runtime projects them
into semantic events only at meaningful boundaries such as:

- inference started
- full tool call assembled
- inference completed
- structured response validated

The semantic event plane must be replay-meaningful. A replay consumer should be
able to reconstruct execution progress without depending on raw token chunks.

## Metadata Rules

`IntentMeta` keeps causal ordering and replay identity.

`EventMeta` keeps timestamp, correlation, dispatch/run identity, and source
information needed for observation, auditing, and bridges.

Only intents participate in intent replay ordering. Observations must remain
non-executable by type.

## Relationship to Current Specs

This spec supersedes `specs/03-effects-and-execution-semantics.md` for the v2
track and narrows `layer0` so executable intents and observations are no longer
modeled as one enum.

## Minimum Proving Tests

- Intent executors never receive observational events by type.
- Semantic event consumers can reconstruct execution progress without raw provider token chunks.
- Intent replay ordering remains deterministic under concurrent observation emission.
