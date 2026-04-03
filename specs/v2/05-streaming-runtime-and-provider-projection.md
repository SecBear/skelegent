# Streaming Runtime and Provider Projection

## Purpose

Make execution stream-first from provider through runtime to caller.

## Provider Contract

Providers in v2 are stream-first:

```rust
pub trait Provider: Send + Sync {
    async fn infer(&self, request: InferRequest) -> Result<InferStream, ProviderError>;
}
```

Convenience collectors may produce a final aggregated inference response, but
that is a helper built on top of the stream, not a distinct primary path.

## Runtime Projection

The turn runtime consumes provider streams and projects them into semantic events.

The runtime is responsible for:

- assembling complete tool calls from provider deltas
- assembling complete message content
- validating structured output if required
- emitting semantic execution events at meaningful boundaries

## No Behavioral Split

V2 forbids separate blocking and streaming execution paths with divergent
behavior. Rule firing, telemetry, budget checks, tool execution, and approvals
must behave the same regardless of whether the caller chooses to stream or
collect.

## Collection Helpers

Collection helpers may aggregate:

- the semantic event stream into a final `Outcome`
- the provider stream into a final inference response

These helpers must not introduce alternate execution semantics.

## Relationship to Current Specs

This spec supersedes the blocking-first provider/runtime assumptions in
`specs/04-operator-turn-runtime.md` for the v2 track while preserving provider
swap as a first-class goal.

## Minimum Proving Tests

- Streaming and collected execution of the same turn produce the same outcome and semantic events.
- Provider token deltas are consumed internally and projected only at semantic boundaries.
- Structured output validation behaves identically in streaming and collected modes.
