# Worktrees And Parallel Sessions

When work items are disjoint, isolate them using git worktrees and separate agent sessions.

## Rules

1. One task per worktree.
2. One agent session per worktree.
3. Keep worktrees small and scoped to a spec domain.
4. Prefer merges over rebases for integrating agent work (minimize history rewriting).

## Creating A Worktree

Use `scripts/new-worktree.sh`:

```bash
./scripts/new-worktree.sh orch-temporal redesign/v2
```

This creates:

1. A branch `feat/orch-temporal` off the base branch.
2. A sibling directory `../neuron-explore-orch-temporal/` checked out to that branch.

## Avoiding Context Mixing

Do not share "primary context windows" across worktrees.

If you must coordinate across multiple parallel worktrees:

1. Treat your main session as a scheduler only.
2. Delegate exploration to subagents.
3. Delegate build/test verification to exactly one session at a time.

