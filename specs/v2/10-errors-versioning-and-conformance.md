# Errors, Versioning, and Conformance

## Purpose

Make v2 observable, serializable, and safe to evolve.

## Structured Errors

Protocol-visible failures must be structured and serializable:

```rust
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: serde_json::Value,
}
```

Display strings may be derived for logs, but they must not be the canonical wire
representation for dispatch or event failures.

## Cutover and Evolution Rules

- The initial adoption of v2 MAY be breaking relative to the current public
  kernel surfaces. It is a planned reset, not a compatibility promise.
- After the v2 cutover lands, Layer 0 changes within the v2 line should remain
  additive whenever possible.
- Extension traits are preferred over repeatedly expanding base traits.
- External protocol bridges must translate from structured errors, not parse
  human-readable strings.

## Conformance Requirements

V2 requires:

- golden semantic event traces
- wire round-trip coverage for all Layer 0 value types
- bridge conformance for capability and outcome projection
- backend capability manifests for state and environment implementations
- proof tests that stream-first collection helpers do not change behavior

## Async Trait Migration Note

V2 trait signatures should be written to the intended native-async shape.

If `async-trait` remains temporarily in implementation, it must be treated as a
compatibility polyfill with a clear future migration path, not the desired final
contract.

## Relationship to Current Specs

This spec supersedes the versioning and proof obligations currently spread across
`specs/11-testing-examples-and-backpressure.md`,
`specs/12-packaging-versioning-and-umbrella-crate.md`, and the Layer 0
compatibility notes in `specs/02-layer0-protocol-contract.md`.

## Minimum Proving Tests

- Structured protocol errors round-trip across dispatch handles and bridge adapters without display-string loss.
- Every public Layer 0 value type has a golden wire-format test.
- Semantic event traces are stable enough to use as conformance fixtures across at least two implementations.
