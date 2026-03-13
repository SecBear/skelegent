# Layer0 Slimming and 23-Decision Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce Layer 0 to protocol-only essentials, make pre-inference governance honest, move lifecycle coordination above Layer 0, and validate the architecture with a supervising multi-agent system under `golden/projects/skelegent/`.

**Architecture:** Layer 0 keeps only stable protocol traits and real cross-boundary wire types. Runtime-local governance (budget guards, compaction, observation/intervention) lives in `skg-context-engine`; orchestration-local policy and coordination lives in `skg-orch-kit`; reusable heavy implementations live in `extras`; a real validating system lives in `golden/projects/skelegent/`.

**Tech Stack:** Rust workspace crates in `skelegent/` and `extras/`, Nix flakes per repo (`skelegent/flake.nix`, `extras/flake.nix`, `golden/flake.nix`), Tokio, async-trait, serde, tracing.

---

## Repository-specific tooling

Use the repo-local environment for every command:

- **skelegent/**
  - Format: `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c nix fmt`
  - Test: `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test --workspace --all-targets`
  - Lint: `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- **extras/**
  - Test: `cd /Users/bear/dev/golden-neuron/extras && nix develop -c cargo test --workspace --all-targets`
  - Lint: `cd /Users/bear/dev/golden-neuron/extras && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- **golden/**
  - Enter tooling env: `cd /Users/bear/dev/golden-neuron/golden && nix develop -c bash -lc 'echo ready'`
  - If decision source files change: `cd /Users/bear/dev/golden-neuron/golden && nix develop -c bash scripts/build-decisions.sh`

Do not use global tooling when a repo flake exists.

---

## Task 1: Finish Layer 0 slimming

**Files:**
- Modify: `skelegent/layer0/src/lifecycle.rs`
- Modify: `skelegent/layer0/src/lib.rs`
- Modify: `skelegent/layer0/tests/phase1.rs`
- Modify: `skelegent/ARCHITECTURE.md`
- Modify: `skelegent/specs/02-layer0-protocol-contract.md`
- Modify: `skelegent/specs/04-operator-turn-runtime.md`
- Modify: `skelegent/specs/09-hooks-lifecycle-and-governance.md`
- Test: existing layer0 and workspace tests

**Step 1: Write the failing contract assertions**
- Add or update spec text so Layer 0 is explicitly limited to:
  - protocol traits
  - invocation/result wire types
  - cross-boundary nouns
  - message-level hints that travel with data
- Add a test or assertion in `layer0/tests/phase1.rs` that validates remaining Layer 0 exports still serde round-trip correctly after lifecycle vocab removal.

**Step 2: Remove speculative lifecycle vocab from Layer 0**
- Delete `BudgetEvent` and related decision types from `skelegent/layer0/src/lifecycle.rs` unless another live Layer 0 type still depends on them.
- Delete `CompactionEvent` from the same file.
- Keep `CompactionPolicy`.
- Update `lib.rs` re-exports accordingly.
- Remove obsolete lifecycle serde round-trip tests from `layer0/tests/phase1.rs`.

**Step 3: Make docs/specs honest**
- Update `ARCHITECTURE.md` and specs so they stop claiming these event vocabularies are current Layer 0 protocol.
- Replace those claims with the approved rule: lifecycle coordination lives above Layer 0 unless it is already a true cross-boundary contract.

**Step 4: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c nix fmt`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test --workspace --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

**Step 5: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add layer0/src/lifecycle.rs layer0/src/lib.rs layer0/tests/phase1.rs ARCHITECTURE.md specs/02-layer0-protocol-contract.md specs/04-operator-turn-runtime.md specs/09-hooks-lifecycle-and-governance.md
git commit -m "refactor(layer0): remove speculative lifecycle vocab"
```

---

## Task 2: Introduce a real pre-inference boundary in `skg-context-engine`

**Files:**
- Create: `skelegent/op/skg-context-engine/src/ops/infer.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/mod.rs`
- Modify: `skelegent/op/skg-context-engine/src/react.rs`
- Modify: `skelegent/op/skg-context-engine/src/stream_react.rs`
- Modify: `skelegent/op/skg-context-engine/src/compile.rs`
- Modify: `skelegent/op/skg-context-engine/src/lib.rs`
- Test: new tests in `skelegent/op/skg-context-engine/src/ops/infer.rs` or `react.rs`

**Step 1: Write the failing tests**
Add tests proving:
- a `Before<Infer>` rule fires before the provider call
- intervention is drained before inference when inference is run as a context op
- a halted pre-inference rule prevents the provider from being called
- streaming inference uses the same governed boundary

Use a recording/mock provider that increments a call counter; assert the counter stays at `0` when the pre-inference guard halts.

**Step 2: Add inference ops**
Create runtime ops for the actual boundary, for example:
- `Infer` for non-streaming
- `StreamInfer` for streaming

These should:
- accept the already-compiled request or the compile config needed to build it
- invoke the provider
- return `InferResponse`
- emit streaming deltas via `ctx.stream_sender()` for the streaming path

**Step 3: Refactor loops to use `ctx.run(...)` for inference**
In `react.rs` and `stream_react.rs`:
- replace direct `compiled.infer(provider).await?` / direct streaming calls with `ctx.run(Infer { ... }).await?` / `ctx.run(StreamInfer { ... }).await?`
- ensure all pre-inference rules and intervention drain now happen immediately before the model call

**Step 4: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test -p skg-context-engine --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy -p skg-context-engine --all-targets -- -D warnings`

**Step 5: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add op/skg-context-engine/src/ops/infer.rs op/skg-context-engine/src/ops/mod.rs op/skg-context-engine/src/react.rs op/skg-context-engine/src/stream_react.rs op/skg-context-engine/src/compile.rs op/skg-context-engine/src/lib.rs
git commit -m "feat(context-engine): make inference a first-class runtime boundary"
```

---

## Task 3: Make budget enforcement honest and explicit

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/rules/budget.rs`
- Modify: `skelegent/skelegent/src/agent.rs`
- Modify: `skelegent/op/skg-context-engine/src/react.rs`
- Modify: `skelegent/specs/04-operator-turn-runtime.md`
- Modify: `skelegent/ARCHITECTURE.md`
- Test: budget guard tests and agent-level tests

**Step 1: Write the failing tests**
Add tests proving:
- `BudgetGuard` attached as `Before<Infer>` blocks the provider call when turn limit is reached
- `BudgetGuard` blocks inference before cost/duration/tool-call breaches continue
- built-agent path returns a structured budget exit rather than collapsing everything into generic inference failure

**Step 2: Narrow the guard trigger**
- Stop using `Trigger::BeforeAny` for budget guard attachment in the high-level agent builder.
- Attach it to the actual inference op boundary introduced in Task 2.

**Step 3: Fix the exit mapping**
- Decide on one honest mapping for budget halt in the built-agent path.
- Recommended: budget-triggered halts become `OperatorOutput` with `ExitReason::BudgetExhausted`, not `OperatorError::InferenceError`.
- If necessary, introduce a typed internal halt reason in `skg-context-engine` so budget halts can be distinguished from generic halts.

**Step 4: Update docs/specs**
- Make `ARCHITECTURE.md` and `specs/04-operator-turn-runtime.md` match the actual behavior.
- Document the authority split:
  - turn-local guard = local boundary enforcement
  - orchestration policy = aggregate budget governance

**Step 5: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test -p skg-context-engine -p skelegent --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy -p skg-context-engine -p skelegent --all-targets -- -D warnings`

**Step 6: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add op/skg-context-engine/src/rules/budget.rs op/skg-context-engine/src/react.rs skelegent/src/agent.rs specs/04-operator-turn-runtime.md ARCHITECTURE.md
git commit -m "fix(runtime): enforce budget at the pre-inference boundary"
```

---

## Task 4: Move lifecycle coordination vocabulary above Layer 0 into `skg-orch-kit`

**Files:**
- Create: `skelegent/orch/skg-orch-kit/src/budget.rs`
- Create: `skelegent/orch/skg-orch-kit/src/compaction.rs`
- Modify: `skelegent/orch/skg-orch-kit/src/lib.rs`
- Modify: any existing orch-kit middleware/helper modules that should consume the new vocabulary
- Test: new orch-kit tests

**Step 1: Write the failing tests**
Add tests for orch-kit-local coordination types/functions such as:
- aggregate budget policy decisions
- compaction coordination policy selection
- pre-compaction flush decision points

Do not re-create speculative protocol. Keep these as orchestration-local types.

**Step 2: Add minimal orchestration-local coordination models**
Add only the vocabulary that is immediately useful for:
- workflow-level budget governance
- compaction coordination and reporting

Keep it small. No giant event taxonomy. Prefer explicit policy-return types over event enums unless multiple consumers already require event values.

**Step 3: Export only what is currently useful**
- Re-export from `skg-orch-kit` only the small set of types/functions actually used by later tasks.

**Step 4: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test -p skg-orch-kit --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy -p skg-orch-kit --all-targets -- -D warnings`

**Step 5: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add orch/skg-orch-kit/src/budget.rs orch/skg-orch-kit/src/compaction.rs orch/skg-orch-kit/src/lib.rs
git commit -m "feat(orch-kit): add orchestration-local lifecycle coordination"
```

---

## Task 5: Implement observation/intervention adapters in `skg-orch-kit`

**Files:**
- Create: `skelegent/orch/skg-orch-kit/src/observe.rs`
- Create: `skelegent/orch/skg-orch-kit/src/intervene.rs`
- Modify: `skelegent/orch/skg-orch-kit/src/lib.rs`
- Modify: `skelegent/orch/skg-orch-kit/Cargo.toml` if new deps are needed
- Test: `skelegent/orch/skg-orch-kit/tests/` or inline tests

**Step 1: Write the failing tests**
Add tests proving:
- an observer can subscribe to a worker context stream and receive `ContextEvent`s
- an intervention adapter can send an erased op into the worker channel
- a worker run processes the intervention before the next governed boundary

**Step 2: Implement `ObserveTool`-equivalent adapter**
Not necessarily as a Layer 0 tool type. The reusable primitive should:
- hold a `broadcast::Receiver<ContextEvent>` or a subscribe function
- expose a stable way to pull/transform stream events for supervisors/oracles

**Step 3: Implement `InterveneTool`-equivalent adapter**
- hold an `mpsc::Sender<Box<dyn ErasedOp>>`
- provide explicit APIs for sending interventions
- keep the intervention vocabulary open by accepting erased context ops or a thin typed wrapper around them

**Step 4: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test -p skg-orch-kit --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy -p skg-orch-kit --all-targets -- -D warnings`

**Step 5: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add orch/skg-orch-kit/src/observe.rs orch/skg-orch-kit/src/intervene.rs orch/skg-orch-kit/src/lib.rs orch/skg-orch-kit/Cargo.toml
git commit -m "feat(orch-kit): add observation and intervention adapters"
```

---

## Task 6: Turn compaction into a coordinated mechanism

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/rules/compaction.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/store.rs`
- Modify: `skelegent/orch/skg-orch-kit/src/compaction.rs`
- Modify: `skelegent/ARCHITECTURE.md`
- Modify: `skelegent/specs/04-operator-turn-runtime.md`
- Modify: `skelegent/specs/09-hooks-lifecycle-and-governance.md`
- Test: context-engine and orch-kit tests

**Step 1: Write the failing tests**
Add tests proving:
- a configured pre-compaction flush happens before destructive compaction
- flush failure is explicit and blocks or alters compaction according to policy
- compaction can still be opt-in, but when configured with pre-flush it is mandatory

**Step 2: Add a small compaction coordinator surface**
In `skg-orch-kit`, add policy primitives for:
- no compaction
- threshold trim
- summarize-and-replace
- pre-flush + compact

Do not over-generalize.

**Step 3: Wire flush + compaction composition**
Use existing `FlushToStore` and compaction primitives instead of inventing a second mechanism.

**Step 4: Update docs/specs**
Make the architecture honest:
- compaction coordination lives above Layer 0
- pre-compaction flush is a configured lifecycle policy, not an implied protocol guarantee

**Step 5: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test -p skg-context-engine -p skg-orch-kit --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy -p skg-context-engine -p skg-orch-kit --all-targets -- -D warnings`

**Step 6: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add op/skg-context-engine/src/rules/compaction.rs op/skg-context-engine/src/ops/store.rs orch/skg-orch-kit/src/compaction.rs ARCHITECTURE.md specs/04-operator-turn-runtime.md specs/09-hooks-lifecycle-and-governance.md
git commit -m "feat(compaction): add coordinated pre-flush and lifecycle policy"
```

---

## Task 7: Normalize the 23-decision configuration surface

**Files:**
- Modify: `skelegent/ARCHITECTURE.md`
- Create or modify: `skelegent/docs/design/` decision-surface doc
- Modify: relevant builder/config files in `skelegent/skelegent/src/agent.rs`, `skg-context-engine`, and `skg-orch-kit`
- Test: focused builder/config tests

**Step 1: Write the failing doc/tests**
Create a decision-surface matrix mapping each golden decision to one of:
- Layer 0 noun
- turn-local knob
- orchestration knob
- backend implementation point

Add focused tests for builder/config surfaces that are missing today.

**Step 2: Add missing knobs only where needed**
Likely areas:
- turn-local config in `agent.rs` / builder layer
- orchestration config in `skg-orch-kit`
- explicit child-context and result-routing strategy types above Layer 0

Keep protocol untouched unless a new boundary noun is truly required.

**Step 3: Verify skelegent**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test --workspace --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

**Step 4: Commit**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add ARCHITECTURE.md docs/design/ skelegent/src/agent.rs orch/skg-orch-kit/ op/skg-context-engine/
git commit -m "feat(architecture): normalize decision-surface knobs"
```

---

## Task 8: Build the supervising multi-agent validator in `golden/projects/skelegent/`

**Files:**
- Create/modify under `golden/projects/skelegent/`:
  - `systems/` or a new validator directory for the supervising system design/implementation
  - supporting scripts/configs/examples as needed
- Modify reusable supporting code in `skelegent/` / `extras/` only when the validator exposes a real missing primitive
- Test: validator-specific tests and smoke runs

**Step 1: Write the failing acceptance tests/spec**
Define the validator acceptance criteria in golden project code/docs:
- worker emits stream events
- supervisor observes them
- supervisor can intervene
- child context policy is explicit
- result routing is explicit
- model routing exercises at least two classes/tiers
- budget governance is split local/global
- compaction survives pressure with recoverable state

**Step 2: Build the minimal real system**
Implement a supervising multi-agent system with at least:
- worker operator
- supervisor operator
- coordinator/composition glue

Use existing skelegent primitives first. Do not paper over missing primitives with one-off hacks; if the validator exposes a real primitive gap, add the primitive in `skelegent` or reusable backend support in `extras`.

**Step 3: Add reusable pieces to `extras` only when they are reusable**
Examples that may belong in `extras` if needed:
- reusable provider routing policy impls
- reusable durable-ish orchestration helpers
- reusable state backend adapters

Do not put project-specific application logic into `extras`.

**Step 4: Verify all three repos as impacted**
Run the relevant repo-local checks with their own flakes.

**Step 5: Commit**
Use small commits by repo/logical milestone, e.g.:
```bash
cd /Users/bear/dev/golden-neuron/golden
git add projects/skelegent/...
git commit -m "feat(projects): add supervising multi-agent validator"
```

---

## Task 9: Final integration verification and review

**Files:**
- All touched files from prior tasks

**Step 1: Run full verification**
Run:
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c nix fmt`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo test --workspace --all-targets`
- `cd /Users/bear/dev/golden-neuron/skelegent && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/bear/dev/golden-neuron/extras && nix develop -c cargo test --workspace --all-targets`
- `cd /Users/bear/dev/golden-neuron/extras && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/bear/dev/golden-neuron/golden && nix develop -c bash -lc 'echo ready'`
- if golden decision source files changed: `cd /Users/bear/dev/golden-neuron/golden && nix develop -c bash scripts/build-decisions.sh`

**Step 2: Cross-check design vs implementation**
Confirm:
- Layer 0 contains only defended protocol essentials
- inference is a real governed runtime boundary
- lifecycle coordination is above Layer 0
- supervising validator exists in `golden/projects/skelegent/`
- `extras` contains only reusable implementations/patterns

**Step 3: Final review**
Run a final code review pass focused on:
- protocol honesty
- no duplicate mechanisms
- all 23 decisions still composable via knobs above Layer 0

**Step 4: Commit final cleanup if needed**
```bash
cd /Users/bear/dev/golden-neuron/skelegent
git add -A
git commit -m "chore: finalize layer0 slimming and validator integration"
```

---

## Execution order

Follow the tasks in order. Do not start the validator before Tasks 1-7 are complete enough that the validator can pressure-test the right architecture instead of relying on stopgap hacks.

## Review gates

After each task:
1. implementation self-review
2. spec-compliance review
3. code-quality review

Do not proceed with open review issues.

## Expected end state

At the end of this plan:
- Layer 0 is protocol-only and slimmer than today
- speculative lifecycle vocab is out of Layer 0
- budget guards act at the real inference boundary
- observation/intervention are reusable above Layer 0
- compaction is coordinated honestly above Layer 0
- a real supervising multi-agent system in `golden/projects/skelegent/` validates the architecture
- `extras` contains only reusable implementations and patterns
