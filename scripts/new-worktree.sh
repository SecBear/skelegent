#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <topic-slug> [base-branch]"
  echo "example: $0 brain-v1 redesign/v2"
  exit 2
fi

topic="$1"
base="${2:-$(git rev-parse --abbrev-ref HEAD)}"

root="$(git rev-parse --show-toplevel)"
parent="$(cd "$root/.." && pwd)"
branch="feat/${topic}"
path="${parent}/skg-explore-${topic}"

echo "[worktree] base:   ${base}"
echo "[worktree] branch: ${branch}"
echo "[worktree] path:   ${path}"

git fetch --all --prune >/dev/null 2>&1 || true

if git show-ref --verify --quiet "refs/heads/${branch}"; then
  echo "[worktree] branch exists locally: ${branch}"
else
  git branch "${branch}" "${base}"
fi

git worktree add "${path}" "${branch}"

echo "[worktree] created: ${path}"
echo "[worktree] next: cd \"${path}\""

