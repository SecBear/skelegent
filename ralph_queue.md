# ralph_queue.md

This file is the single "what next" queue used by the Ralph loop (`PROMPT.md`).

Rules:

1. Keep it short.
2. Each item must link to the governing spec(s).
3. Each item must have a concrete "done when" and a verification command.

- ReleasePrep: Prepare merge + publish
  - Done when: RELEASE_NOTES.md and MIGRATION.md present; crate versions bumped coherently; publish.yml dry-run succeeds
  - Verify: GH workflow `publish (manual)` with `dry-run: true` passes; CI green on PR
- RFC-ExecPrimitives: Factor execution primitives into dedicated crates
  - Specs: `specs/01-architecture-and-layering.md`, this RFC: `docs/plans/2026-03-02-execution-primitives-rfc.md`
  - Done when:
    - (A) `neuron-effects-core` crate created (trait + policy), `neuron-effects-local` moved from orch-kit; orch-kit re-exports for back-compat
    - (B) `neuron-turn-kit` crate created (planner/decider/batch-executor/steering traits + BarrierPlanner) and `neuron-op-react` switched to them without behavior change
  - Verify: `nix develop -c cargo build --workspace` and `nix develop -c cargo test --workspace --all-targets`
## Queue

- RHO-04: Reference local effect interpreter (execute Effects)
  - Specs: `specs/03-effects-and-execution-semantics.md`, `specs/05-orchestration-core.md`
  - Done when: add a minimal interpreter (in `neuron-orch-kit` or a new crate) that executes `WriteMemory/DeleteMemory` against selected `StateStore`, and maps `Delegate/Handoff/Signal` to orchestrator calls. Deterministic order and idempotence covered by tests.
  - Verify: `nix develop -c cargo test -p neuron-orch-kit -p neuron-orch-local -- --nocapture`
- RHO-05: LocalOrch signal/query minimal semantics
  - Specs: `specs/05-orchestration-core.md`
  - Done when: `signal` persists to a per-workflow journal and is observable by operator via `SteeringSource` (when wired). `query` returns minimal state shape. Tests verify accept, retrieval, and null behaviors.
  - Verify: `nix develop -c cargo test -p neuron-orch-local -- --nocapture`
- RHO-06: Provider credential story (CredentialRef path + no-leak tests)
  - Specs: `specs/08-environment-and-credentials.md`, `specs/10-secrets-auth-crypto.md`
  - Done when: providers accept credentials via `EnvironmentSpec`/`CredentialRef` wiring in local mode; secret resolution/injection tested with redaction. Error messages sanitized (no secret material).
  - Verify: `nix develop -c cargo test -p neuron-env-local -p neuron-provider-* -- --nocapture`
- RHO-07: Developer docs/examples for custom operator and migration
  - Specs: `specs/13-documentation-and-dx-parity.md`
  - Done when: add a short guide + example for implementing a custom operator (barrier scheduling + steering) and a Rho migration note; ensure README/book link paths.
  - Verify: `nix develop -c nix fmt && nix develop -c cargo test --workspace --all-targets`


## Completed

- 2026-02-28: Implement credential resolution + injection + audit story in local mode
  - Specs: `specs/08-environment-and-credentials.md`, `specs/10-secrets-auth-crypto.md`, `specs/09-hooks-lifecycle-and-governance.md`
  - Adds:
    - `neuron-env-local` now supports optional `SecretResolver` wiring and credential injection for `EnvVar`/`File`/`Sidecar` delivery modes
    - `LocalEnv` now emits both `SecretAccessEvent` (audit) and `ObservableEvent` (lifecycle) through a pluggable `EnvironmentEventSink`
    - Credential resolution/injection failures are sanitized to avoid secret-material leakage in `EnvError::CredentialFailed` messages
    - New integration coverage for end-to-end pipeline behavior and no-leak guarantees in `env/neuron-env-local/tests/env.rs`
  - Verify: `nix develop -c cargo test --workspace --all-targets` (pass)

- 2026-02-27: Make orchestration "core complete" for composed systems
  - Specs: `specs/03-effects-and-execution-semantics.md`, `specs/05-orchestration-core.md`, `specs/06-composition-factory-and-glue.md`, `specs/11-testing-examples-and-backpressure.md`
  - Adds:
    - `neuron-orch-kit` end-to-end effect pipeline integration test
    - `neuron-orch-local` in-memory workflow signal journal semantics
  - Verify: `nix develop -c cargo test --workspace --all-targets` (pass)

- 2026-02-27: CI hard enforcement (format, tests, clippy) is present
  - Spec: `specs/13-documentation-and-dx-parity.md`
  - Workflow: `.github/workflows/ci.yml`

- 2026-02-27: Root README added (crate map + quickstart)
  - Spec: `specs/13-documentation-and-dx-parity.md`
  - File: `README.md`

- 2026-02-27: Umbrella `neuron` crate added (features + prelude)
  - Spec: `specs/12-packaging-versioning-and-umbrella-crate.md`
  - Crate: `neuron/`

- 2026-03-02: RHO-01 ToolExecutionStrategy added (opt-in)
  - Specs: `specs/04-operator-turn-runtime.md`, `specs/01-architecture-and-layering.md`
  - Adds:
    - `ToolExecutionPlanner` + `BarrierPlanner` and `ConcurrencyDecider`
    - ReactOperator accepts planner/decider; default sequential keeps behavior
    - Shared-batch parallel execution; preserves order and hooks
  - Verify: `nix develop -c cargo test -p neuron-op-react --all-targets` (pass)
- 2026-03-02: RHO-02 SteeringSource (opt-in) and mid-loop injection
  - Specs: `specs/04-operator-turn-runtime.md`, `specs/05-orchestration-core.md`
  - Adds: `SteeringSource` trait; ReactOperator builder; boundary polls with skip semantics; placeholders for skipped tools
  - Verify: `nix develop -c cargo test -p neuron-op-react --all-targets` (pass)
- 2026-03-02: RHO-03 Streaming tool API + ToolExecutionUpdate hook
  - Specs: `specs/09-hooks-lifecycle-and-governance.md`, `specs/04-operator-turn-runtime.md`
  - Adds: `ToolDynStreaming` (optional); `HookPoint::ToolExecutionUpdate`; chunk forwarding in ReactOperator; tests
  - Verify: `nix develop -c cargo test -p neuron-tool -p neuron-op-react --all-targets` (pass)