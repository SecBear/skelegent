# Durable Alignment and Control

## Purpose

Align immediate and durable control on shared value types while preserving
backend freedom over durable lifecycle internals.

## Shared Value Types

Immediate and durable flows must share:

- `Outcome`
- `WaitReason`
- resume payloads
- semantic execution events

This allows the same invocation result vocabulary to travel through local,
remote, and durable orchestration.

## Durable-Only Surfaces

Durable-only contracts remain above Layer 0:

- run starter/controller surfaces
- run views and terminal read models
- timer persistence
- leases and worker claims
- checkpoints, journals, and replay strategy

V2 must not collapse immediate invocation and durable lifecycle into one trait.

## Wait Semantics

Approval, timer, external input, and child-run waits must have:

- one shared `WaitReason`
- one shared resume payload model
- clear immediate behavior
- clear durable behavior

Immediate mode returns a suspended outcome to the caller.

Durable mode persists the wait and resumes through backend-specific control
surfaces.

## Relationship to Current Specs

This spec supersedes `specs/14-durable-orchestration-core.md` for the v2 track
while preserving the current rule that durable internals remain backend-specific.

## Minimum Proving Tests

- Immediate and durable paths serialize the same wait reasons and resume payloads.
- Durable backends remain free to choose checkpoint, replay, and timer storage internals.
- A bridge layer can map durable and immediate suspended outcomes without special-case value translation.
