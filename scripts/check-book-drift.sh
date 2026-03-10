#!/usr/bin/env bash
# check-book-drift.sh — Detect documentation drift between the book and codebase.
#
# Checks:
#   1. Workspace version/edition/MSRV in book matches Cargo.toml
#   2. Every crate name in the book exists in the workspace
#   3. No references to types/structs that were renamed or deleted
#   4. No golden decision vocabulary leaking in (D1-D5, C1-C5, L1-L5)
#
# Usage: ./scripts/check-book-drift.sh
# Exit code 0 = clean, 1 = drift detected.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BOOK_DIR="$REPO_ROOT/docs/book/src"
CARGO_TOML="$REPO_ROOT/Cargo.toml"
ERRORS=0

red()   { printf '\033[1;31m%s\033[0m\n' "$*"; }
green() { printf '\033[1;32m%s\033[0m\n' "$*"; }
check() { printf '  checking: %s\n' "$*"; }

echo "=== Book drift detection ==="
echo

# ---------------------------------------------------------------------------
# 1. Cargo.toml facts
# ---------------------------------------------------------------------------
EDITION=$(grep -m1 '^edition' "$CARGO_TOML" | sed 's/.*"\(.*\)".*/\1/')
MSRV=$(grep -m1 '^rust-version' "$CARGO_TOML" | sed 's/.*"\(.*\)".*/\1/')
VERSION=$(grep -m1 '^version' "$CARGO_TOML" | sed 's/.*"\(.*\)".*/\1/')
# Major.minor for loose version refs in docs (e.g. version = "0.4")
VERSION_MM="${VERSION%.*}"

check "edition = $EDITION, MSRV = $MSRV, version = $VERSION ($VERSION_MM)"

# Stale edition
if grep -rn "edition 2021\|edition 2018\|edition 2015" "$BOOK_DIR" --include='*.md' | grep -v "edition $EDITION"; then
    red "DRIFT: Book references wrong Rust edition (expected $EDITION)"
    ERRORS=$((ERRORS + 1))
fi

# Stale MSRV — look for "MSRV <digits>" pattern
if grep -rn "MSRV [0-9]" "$BOOK_DIR" --include='*.md' | grep -v "MSRV $MSRV"; then
    red "DRIFT: Book references wrong MSRV (expected $MSRV)"
    ERRORS=$((ERRORS + 1))
fi

# Stale version strings — match version = "X.Y" in toml blocks
STALE_VERSIONS=$(grep -rn 'version = "' "$BOOK_DIR" --include='*.md' | grep -v "version = \"$VERSION_MM" | grep -v 'version = "0\.\*"' || true)
if [ -n "$STALE_VERSIONS" ]; then
    echo "$STALE_VERSIONS"
    red "DRIFT: Book has version strings not matching $VERSION_MM"
    ERRORS=$((ERRORS + 1))
fi

# ---------------------------------------------------------------------------
# 2. Workspace member names
# ---------------------------------------------------------------------------
check "workspace members exist in book"

# Extract workspace member directory names, map to crate names (dir basename)
MEMBERS=$(sed -n '/^members/,/^\]/p' "$CARGO_TOML" \
    | grep '"' \
    | sed 's/.*"\(.*\)".*/\1/' \
    | xargs -I{} basename {} \
    | grep -v custom_operator_barrier \
    | sort)

# Check each crate name appears somewhere in the book
for crate in $MEMBERS; do
    if ! grep -rq "$crate" "$BOOK_DIR" --include='*.md'; then
        red "DRIFT: Workspace member '$crate' not mentioned anywhere in book"
        ERRORS=$((ERRORS + 1))
    fi
done

# ---------------------------------------------------------------------------
# 3. Known renamed/deleted types
# ---------------------------------------------------------------------------
check "no references to renamed/deleted types"

BANNED_TYPES=(
    "LocalOrchestrator"
    "LocalEnvironment"
    "RequestFailed"
)

for typ in "${BANNED_TYPES[@]}"; do
    HITS=$(grep -rn "$typ" "$BOOK_DIR" --include='*.md' || true)
    if [ -n "$HITS" ]; then
        echo "$HITS"
        red "DRIFT: Book references renamed/deleted type '$typ'"
        ERRORS=$((ERRORS + 1))
    fi
done

# ---------------------------------------------------------------------------
# 4. Golden decision vocabulary
# ---------------------------------------------------------------------------
check "no golden decision vocabulary"

# Match D1-D5, C1-C5, L1-L5 as standalone identifiers (word boundaries)
# Exclude (L<n>) which are layer annotations in diagrams
GOLDEN_HITS=$(grep -rEn '(^|[^a-zA-Z(])[DCL][1-5]($|[^0-9a-zA-Z)])' "$BOOK_DIR" --include='*.md' || true)
if [ -n "$GOLDEN_HITS" ]; then
    echo "$GOLDEN_HITS"
    red "DRIFT: Book contains golden decision vocabulary"
    ERRORS=$((ERRORS + 1))
fi

# ---------------------------------------------------------------------------
# 5. Public type spot-checks (extract from actual code, verify book matches)
# ---------------------------------------------------------------------------
check "key public types referenced correctly"

# ProviderError variants — check the book mentions the actual variants
PROVIDER_ERROR_FILE="$REPO_ROOT/turn/skg-turn/src/provider.rs"
if [ -f "$PROVIDER_ERROR_FILE" ]; then
    # Extract variant names from the enum
    VARIANTS=$(grep -oE '^[[:space:]]+[A-Z][a-zA-Z]+[[:space:]]*[\({]' "$PROVIDER_ERROR_FILE" | awk '{print $1}' | tr -d '({' | sort -u)
    for v in $VARIANTS; do
        # Only check non-trivial variants (skip Other)
        if [ "$v" = "Other" ]; then continue; fi
        if ! grep -rq "$v" "$BOOK_DIR/reference/error-handling.md"; then
            red "DRIFT: ProviderError::$v not documented in error-handling.md"
            ERRORS=$((ERRORS + 1))
        fi
    done
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
if [ "$ERRORS" -eq 0 ]; then
    green "No drift detected."
    exit 0
else
    red "$ERRORS drift issue(s) found."
    exit 1
fi
