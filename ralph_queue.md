# ralph_queue.md

This file is the single "what next" queue used by the Ralph loop (`PROMPT.md`).

Rules:

1. Keep it short.
2. Each item must link to the governing spec(s).
3. Each item must have a concrete "done when" and a verification command.

## Queue

- No open items. Add the next highest-priority spec-backed task.


## Completed

- 2026-02-28: Implement credential resolution + injection + audit story in local mode
  - Specs: `specs/08-environment-and-credentials.md`, `specs/10-secrets-auth-crypto.md`, `specs/09-hooks-lifecycle-and-governance.md`
  - Adds:
    - `neuron-env-local` now supports optional `SecretResolver` wiring and credential injection for `EnvVar`/`File`/`Sidecar` delivery modes
    - `LocalEnv` now emits both `SecretAccessEvent` (audit) and `ObservableEvent` (lifecycle) through a pluggable `EnvironmentEventSink`
    - Credential resolution/injection failures are sanitized to avoid secret-material leakage in `EnvError::CredentialFailed` messages
    - New integration coverage for end-to-end pipeline behavior and no-leak guarantees in `neuron-env-local/tests/env.rs`
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
