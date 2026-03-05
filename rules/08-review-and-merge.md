# Review And Merge

This repo uses worktree-based development for autonomous implementers.
This rule defines how to review worktree output before merging.

## What Review Must Prove

Before merge, reviewers must have fresh evidence that:

1. The change satisfies the work item's acceptance criteria.
2. The change matches the governing spec(s) and did not invent new behavior.
3. Deterministic backpressure is green.
4. The change is safe to merge (no scope creep, no accidental protocol expansion).

## Deterministic Gates (Required)

Run these in the worktree being reviewed:

- `./scripts/verify.sh`

If the worktree touched protocol boundaries (`layer0`) or cross-cutting governance, also run:

- `nix develop -c cargo test --workspace --all-targets`
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

Do not merge without green results.

## Context Hygiene For Review (New Context Window)

Do review in a fresh agent session (new context window) to prevent “implementation bias”.

Load order for review sessions:

1. `AGENTS.md`
2. `SPECS.md`
3. The spec(s) relevant to the completed work item
4. Relevant `rules/` entries (especially `rules/02-verification-and-nix.md` and this file)

Then:

1. Inspect the diff (`git diff <base>...HEAD`).
2. Map every meaningful behavior change back to a spec section or the work item's acceptance criteria.
3. Confirm there are tests proving the new behavior (or that the change is doc/config-only).

## LLM Review (Manual, Recommended)

LLM review is intentionally NOT a required CI gate:

- it is non-deterministic,
- it can be expensive,
- it can fail closed for reasons unrelated to correctness.

Instead, invoke it manually during review using Codex/Claude Code with a strict “reviewer” prompt.

### Reviewer Prompt Template

Use this as the system/user prompt for the reviewer model:

1. You are reviewing a worktree produced by an autonomous agent.
2. Your job is to judge spec compliance and merge safety, not to rewrite the implementation.
3. Load and cite the governing spec(s) and the work item.
4. Identify:
   - any behavior not justified by specs,
   - any missing tests vs “Done when”,
   - any protocol changes without spec edits,
   - any security concerns (secrets/logging, tool policy bypass),
   - any places verification is insufficient.
5. Output:
   - “Approve” or “Request changes”
   - a short list of required follow-ups (tests/docs/refactors)
   - the single biggest risk if merged as-is.

## Merge Guidance

- Prefer merge commits for integrating worktrees (see `rules/06-worktrees-and-parallelism.md`).
- Keep follow-up fixes in the same worktree if they are required to satisfy the chosen work item.
- If review discovers scope drift (multiple work items touched), stop and split the work into separate worktrees.
