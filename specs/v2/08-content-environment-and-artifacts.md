# Content, Environment, and Artifacts

## Purpose

Define the v2 content model, artifact semantics, and declarative environment boundary.

## Content

V2 keeps the existing universal content model and adds an efficient local binary path:

```rust
pub enum ContentSource {
    Url { url: String },
    Base64 { data: String },
    Bytes(bytes::Bytes),
}
```

`Bytes` is for efficient in-process transfer. Wire serialization may degrade it
to base64 when crossing process or protocol boundaries.

## Artifacts

Artifacts are semantic outputs reported through the event plane.

Artifacts must carry:

- stable artifact identity
- media type
- retrieval source or inline content source
- optional filename/title metadata

Artifacts are not executable intents.

## Environment

V2 keeps one public declarative environment contract. The spec continues to
describe isolation, credentials, resource limits, and network policy as fields
on one environment specification rather than splitting the public protocol too
early.

Backend implementations may decompose this internally into smaller substrate
concerns. That decomposition is not part of the Layer 0 public contract unless
proven necessary by a concrete implementation pressure.

## Relationship to Current Specs

This spec supersedes the content/artifact assumptions scattered across the
current Layer 0 and environment specs, and it refines
`specs/08-environment-and-credentials.md` for the v2 track without exploding
the public environment surface.

## Minimum Proving Tests

- Binary content round-trips locally with `Bytes` and degrades correctly to a wire-safe form when serialized.
- Artifacts appear as semantic events, not executable intents.
- A reference environment honors declarative isolation, credential, resource, and network fields without leaking platform-specific nouns into Layer 0.
