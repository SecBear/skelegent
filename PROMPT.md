# Agent Task (Ralph Loop)

You are an autonomous coding agent operating in this repository.

## Load Stack (Do This First, Every Loop)

1. Read `AGENTS.md` (operating instructions).
2. Read `SPECS.md` (spec index).
3. Read the spec(s) relevant to the single task you will choose from `ralph_queue.md`.
4. Read any relevant files in `rules/` (process constraints).

## Context Hygiene

1. One task per context window.
2. Choose exactly one item from `ralph_queue.md`. Only one.
3. If you drift (multiple tasks, inventing APIs, repeating mistakes), stop and restart a fresh loop.

## Execution (Single Item)

1. Search the codebase before assuming something is missing.
2. Choose the single highest-priority unimplemented item from `ralph_queue.md`.
3. Implement it with TDD when behavior is changing:
   - write a failing test
   - implement minimal code
   - refactor after green
4. Run verification:
   - `nix develop -c cargo test --workspace --all-targets`
   - (when relevant) `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
5. If verification fails, fix it before moving on.
6. Commit only when green. Use a conventional commit title.
7. Update `ralph_queue.md` to reflect what changed and what is next.

## Do Not

1. Do not start multiple work items in the same loop.
2. Do not introduce opinionated workflow DSLs.
3. Do not add new protocol surface area without updating specs.

## Invocation (Human Runs This)

Default (Claude Code):

```bash
cat PROMPT.md | claude-code
```

Codex:

```bash
CODEX=1 ./scripts/ralph-once.sh
```

Pure loop (supervised):

```bash
while :; do cat PROMPT.md | claude-code; done
```

Codex loop (supervised):

```bash
CODEX=1 ./scripts/ralph.sh
```

Auto-create worktree + run:

```bash
./scripts/ralph-worktree.sh orch-temporal redesign/v2
```

Codex model override:

```bash
CODEX=1 CODEX_MODEL=gpt-5.3-codex ./scripts/ralph-once.sh
```
