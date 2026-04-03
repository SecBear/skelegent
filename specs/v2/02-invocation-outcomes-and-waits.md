# Invocation, Outcomes, and Waits

## Purpose

Define the stream-first invocation model shared by immediate execution and durable
control.

## Invocation Primitive

`Dispatcher` remains the immediate invocation trait:

```rust
pub trait Dispatcher: Send + Sync {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<InvocationHandle, DispatchError>;
}
```

The handle is the primary result. Non-streaming callers consume it through
collection helpers. Blocking completion is a convenience, not the native path.

## InvocationHandle Contract

An `InvocationHandle` must support:

- receiving semantic execution events in order
- cancellation
- collection into a final outcome/result helper for convenience callers

The handle must not expose provider chunk internals directly. Its stream is the
semantic event plane defined in `v2/03`.

## Typed Outcome Model

V2 replaces flat exit enums with a classified outcome family:

```rust
pub enum Outcome {
    Terminal(TerminalOutcome),
    Suspended(WaitState),
    Transferred(TransferOutcome),
    Limited(LimitReason),
    Intercepted { reason: String },
    Custom(String),
}
```

This eliminates the need for every consumer to rediscover whether an invocation
ended because it completed, suspended for approval, handed off, or hit a budget.

## Shared Wait Vocabulary

Immediate and durable flows share value types for waits:

- `WaitReason`
- `ResumeInput`
- resume actions or decisions that satisfy a wait

Immediate dispatch returns a suspended outcome and lets the caller decide how to
re-enter. Durable control persists and resumes that same value-level wait through
backend-specific lifecycle machinery.

## Immediate vs Durable Boundary

Shared:

- `Outcome`
- `WaitReason`
- resume input payloads

Durable-only:

- run views
- checkpoint stores
- lease stores
- timer persistence
- replay internals

V2 standardizes the shared nouns, not a single lifecycle substrate.

## Relationship to Current Specs

This spec supersedes the exit and invocation semantics in `specs/04-operator-turn-runtime.md`,
`specs/05-orchestration-core.md`, and `specs/14-durable-orchestration-core.md`
for the v2 track.

## Minimum Proving Tests

- An immediate invocation that suspends for approval and later resumes with the shared wait vocabulary.
- A durable run that expresses the same wait through durable control surfaces without changing the shared value types.
- Collection helpers that produce the same final outcome as consuming the full semantic event stream.
