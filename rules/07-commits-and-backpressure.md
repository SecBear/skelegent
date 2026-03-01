# Commits And Backpressure

Backpressure is the engine of agentic loops. Commits are the unit of progress.

## Rules

1. No commit without fresh verification evidence.
2. If pre-commit fails, fix it immediately. Do not bypass it.
3. Keep commits small and scoped to one work item.

## Conventional Commit Titles

Use:

1. `feat(<scope>): ...`
2. `fix(<scope>): ...`
3. `test(<scope>): ...`
4. `docs(<scope>): ...`
5. `chore(<scope>): ...`

Examples:

1. `feat(orch-local): add workflow cancellation support`
2. `test(hooks): cover SkipTool and ModifyToolInput`

## Suggested Workflow

1. Implement one item from `ralph_queue.md`.
2. Run `./scripts/verify.sh`.
3. Commit with `./scripts/agent-commit.sh "feat(scope): message"`.

## CI Enforcement

CI runs the same feedback loops:

1. pre-commit (treefmt)
2. `cargo test`
3. `cargo clippy -- -D warnings`

Note: use the workspace-wide forms from `rules/02-verification-and-nix.md`.
