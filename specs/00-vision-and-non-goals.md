> **RETIRED — superseded by specs/v2/. Do not use for new implementation work.**

# Vision and Non-Goals

## Vision

Skelegent is a Rust workspace for building agentic systems from composable primitives.

The redesign is centered on a protocol contract (`layer0`) that defines stable boundaries between concerns that *must* be separable if we want to build many different agentic systems without rewriting everything:

- operator/turn execution
- orchestration (composition + durability)
- state (persistence + retrieval)
- environment (isolation + credentials + resource/network policy)
- cross-cutting governance (hooks + lifecycle coordination vocabulary)

Skelegent is not “one agent product.” It is the foundation that many agentic products can be built on.

## Success Criteria

The redesign is “core complete” when:

1. The protocol contract is stable and documented.
2. There is a minimal local reference stack that demonstrates end-to-end composition (mock + real path).
3. Composition semantics are explicit and test-proven (delegate, handoff, signaling, state writes).
4. The remaining work to support a new execution technology (Temporal, Docker, Postgres, Vault) is limited to tech-specific implementations, not rewriting protocols or top-level architecture.

## Non-Goals (For Now)

- A fully distributed production orchestrator implementation (Temporal/Restate/etc.).
- Full isolation environments (Docker/K8s) beyond local passthrough.
- Completing every secret/auth/crypto backend implementation (stubs are acceptable).
- A “workflow DSL.” Skelegent favors protocol composition and factories.

