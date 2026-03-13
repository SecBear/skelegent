# Durable Orchestration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a backend-pluggable durable run/control substrate above Layer 0, then prove it with a SQLite composition and a Temporal adapter path.

**Architecture:** Introduce a new orchestration-local core crate for portable durable run/control primitives (`skg-run-core`), keep replay/checkpoint internals backend-specific, and compose DIY durable backends from smaller persistence/driver seams instead of centering the design on `StateStore`.

**Tech Stack:** Rust workspace crates, Nix tooling, existing `layer0` / `skg-orch-kit` / `skg-orch-local` / `skg-orch-temporal`, SQLite backend in `extras`, async traits, serde, Tokio.

---

### Task 1: Codify architecture/spec positions for durable orchestration

**Files:**
- Modify: `ARCHITECTURE.md`
- Modify: `specs/01-architecture-and-layering.md`
- Modify: `specs/05-orchestration-core.md`
- Modify: `specs/09-hooks-lifecycle-and-governance.md`
- Modify: `SPECS.md` (if a new spec is added)
- Create: `specs/14-durable-orchestration-core.md` (if needed for clarity)
- Reference: `docs/plans/2026-03-12-durable-orchestration-design.md`

**Step 1: Write the failing doc contract review checklist**
Create an explicit checklist in your notes:
- durable run/control is above Layer 0
- `Dispatcher` remains immediate invocation
- signals and resume are distinct
- `StateStore` is not the durable run contract
- replay substrate is backend-specific

**Step 2: Edit the architecture/spec files to match the approved design**
Add durable run/control language and remove any implication that durable orchestration is just state persistence or a Layer 0 concern.

**Step 3: Verify doc coherence with targeted searches**
Run: `grep` searches for contradictory phrases such as `checkpoint`/`history`/`StateStore` in durable sections.
Expected: docs describe the new split consistently.

**Step 4: Commit**
`git commit -m "docs: specify durable orchestration core"`

---

### Task 2: Add `skg-run-core` crate with public durable run/control primitives

**Files:**
- Create: `orch/skg-run-core/Cargo.toml`
- Create: `orch/skg-run-core/src/lib.rs`
- Create: `orch/skg-run-core/src/id.rs`
- Create: `orch/skg-run-core/src/model.rs`
- Create: `orch/skg-run-core/src/control.rs`
- Create: `orch/skg-run-core/src/wait.rs`
- Create: `orch/skg-run-core/tests/public_surface.rs`
- Modify: workspace `Cargo.toml`

**Step 1: Write failing public-surface tests**
Cover:
- serde round-trip for `RunId`, `RunStatus`, `WaitReason`, `ResumeInput`
- compile-time/basic usage test for `RunController` trait objectability
- ensure signal and resume are separate operations

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-run-core --all-targets`
Expected: fail because crate/types don’t exist.

**Step 3: Implement minimal public surface**
Add only the portable nouns and traits:
- `RunId`
- `RunStatus`
- `RunView` / `RunOutcome`
- `WaitPointId`
- `WaitReason`
- `ResumeInput`
- `RunStarter`
- `RunController`

Do **not** add checkpoint/history internals yet.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-run-core --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: add durable run core primitives"`

---

### Task 3: Add pure durable kernel and orchestration command model

**Files:**
- Create: `orch/skg-run-core/src/kernel.rs`
- Create: `orch/skg-run-core/src/command.rs`
- Create: `orch/skg-run-core/tests/kernel_transitions.rs`
- Modify: `orch/skg-run-core/src/lib.rs`

**Step 1: Write failing kernel transition tests**
Cover minimal transitions:
- start -> running
- running -> waiting on explicit waitpoint
- waiting + matching resume -> running/completed
- cancel from running/waiting -> cancelled
- invalid resume token rejected

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-run-core kernel_transitions -- --nocapture`
Expected: fail because kernel does not exist.

**Step 3: Implement minimal pure kernel**
Represent transitions as pure state updates plus emitted orchestration commands such as:
- dispatch operator
- schedule wake
- enter waitpoint
- complete run
- fail run

Keep storage/execution out of the kernel.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-run-core --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: add durable run kernel"`

---

### Task 4: Add lower pluggable DIY seams for durable backends

**Files:**
- Create: `orch/skg-run-core/src/store.rs`
- Create: `orch/skg-run-core/src/timer.rs`
- Create: `orch/skg-run-core/src/lease.rs`
- Create: `orch/skg-run-core/src/driver.rs`
- Create: `orch/skg-run-core/tests/trait_shapes.rs`
- Modify: `orch/skg-run-core/src/lib.rs`

**Step 1: Write failing trait-shape tests**
Cover compile/use of:
- `RunStore`
- `WaitStore`
- `TimerStore`
- optional `LeaseStore`
- `RunDriver`

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-run-core trait_shapes -- --nocapture`
Expected: fail.

**Step 3: Implement narrow lower-level traits**
Keep these small and backend-oriented. Avoid public checkpoint/history commitments beyond opaque payloads or generic references.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-run-core --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: add durable backend seam traits"`

---

### Task 5: Add SQLite-backed durable run components in `extras`

**Files:**
- Create: `extras/run/skg-run-sqlite/Cargo.toml`
- Create: `extras/run/skg-run-sqlite/src/lib.rs`
- Create: `extras/run/skg-run-sqlite/src/schema.rs`
- Create: `extras/run/skg-run-sqlite/src/store.rs`
- Create: `extras/run/skg-run-sqlite/src/timer.rs`
- Create: `extras/run/skg-run-sqlite/tests/sqlite_run_store.rs`
- Modify: `extras/Cargo.toml`

**Step 1: Write failing storage tests**
Cover:
- create/read/update run metadata
- persist waitpoint + resume input
- durable timer insertion/listing
- opaque continuation/checkpoint payload persistence
- status transition correctness

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-run-sqlite --all-targets`
Expected: fail because crate does not exist.

**Step 3: Implement SQLite components**
Back them with separate run tables/namespaces. Do not reuse `StateStore` tables or traits.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-run-sqlite --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: add sqlite durable run components"`

---

### Task 6: Assemble a SQLite durable orchestrator

**Files:**
- Create: `extras/orch/skg-orch-sqlite/Cargo.toml`
- Create: `extras/orch/skg-orch-sqlite/src/lib.rs`
- Create: `extras/orch/skg-orch-sqlite/src/driver.rs`
- Create: `extras/orch/skg-orch-sqlite/src/controller.rs`
- Create: `extras/orch/skg-orch-sqlite/tests/sqlite_orch_flow.rs`
- Reference: current `orch/skg-orch-local`, `orch/skg-orch-kit/src/runner.rs`

**Step 1: Write failing integration tests**
Cover:
- start durable run
- run reaches waitpoint
- resume with matching waitpoint token
- query current run state
- cancel waiting run
- prove effects and operator dispatch compose through existing machinery

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-orch-sqlite --all-targets`
Expected: fail.

**Step 3: Implement minimal durable orchestration composition**
Compose:
- `skg-run-core`
- `skg-run-sqlite`
- current dispatcher/effect/runtime machinery

Do not add product-specific policy.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-orch-sqlite --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: add sqlite durable orchestrator"`

---

### Task 7: Adapt Temporal backend to the public durable run/control contract

**Files:**
- Modify: `extras/orch/skg-orch-temporal/src/lib.rs`
- Modify: `extras/orch/skg-orch-temporal/src/config.rs`
- Add tests under: `extras/orch/skg-orch-temporal/tests/`

**Step 1: Write failing adapter tests**
Cover:
- expose `RunController`-like control surface
- distinguish `signal` from `resume`
- map Temporal query/signal/update semantics into the shared top-level contract without faking a SQL checkpoint model

**Step 2: Run targeted tests to verify red**
Run: `nix develop -c cargo test -p skg-orch-temporal --all-targets`
Expected: fail.

**Step 3: Implement adapter layer**
Use Temporal-native history/signals/queries internally. Do not force it through the same lower SQLite-oriented seams if that creates lies.

**Step 4: Re-run tests**
Run: `nix develop -c cargo test -p skg-orch-temporal --all-targets`
Expected: pass.

**Step 5: Commit**
`git commit -m "feat: adapt temporal orchestrator to durable run core"`

---

### Task 8: Add a real validator/proof project after primitives exist

**Files:**
- Modify or add under: `golden/projects/skelegent/`
- Likely create: `golden/projects/skelegent/durable-supervision-validator/`

**Step 1: Write failing end-to-end tests**
Cover:
- durable wait/resume
- supervisor pause/approval path
- crash-safe restart semantics for SQLite path
- durable signal/query control flow

**Step 2: Run targeted tests to verify red**
Run project-local test command under its own `flake.nix`.
Expected: fail.

**Step 3: Implement the validator**
Use only Skelegent + extras primitives. No product-specific shortcuts in core crates.

**Step 4: Re-run tests**
Expected: pass.

**Step 5: Commit**
`git commit -m "feat(projects): add durable orchestration validator"`

---

### Task 9: Full verification and final review

**Files:**
- Entire affected worktree and extras repo

**Step 1: Run full verification in skelegent worktree**
Run:
- `nix develop -c nix fmt`
- `nix develop -c cargo test --workspace --all-targets`
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

**Step 2: Run full verification in extras**
Run:
- `nix develop -c cargo test --workspace --all-targets`
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

**Step 3: Run project-local validator verification**
Run the validator project’s fmt/clippy/test/demo commands.

**Step 4: Final review**
Use spec review then code-quality review on the full delta.

**Step 5: Commit any final doc/test cleanup**
`git commit -m "docs: align durable orchestration surfaces"`
