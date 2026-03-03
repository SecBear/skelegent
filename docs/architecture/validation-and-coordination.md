# Continuous Validation & Agent Coordination

## Part 1: The Validation Loop

### What It Does

A scheduled process that reads your research documents, checks claims against
current reality, and produces structured findings. It NEVER modifies source
documents. It writes to a separate findings log that you review.

### Architecture

```
YOUR DOCUMENTS (read-only to validator)
  agentic-decision-map-v3.md
  composable-agentic-architecture.md
  HANDOFF.md
  landscape-report.md
      │
      │ reads
      ▼
VALIDATOR (Claude via API, scheduled)
      │
      │ writes (append-only)
      ▼
.validation/
  findings.md          ← append-only log of findings
  last-run.json        ← timestamp + summary of last run
  checksums.json       ← SHA256 of each source doc at validation time
```

### The Validator Prompt

The validator runs with a specific system prompt that constrains it to
FINDING, not FIXING:

```
You are a research validator. You have been given a set of architectural
documents about composable agentic AI systems.

Your job:
1. Identify claims that are time-sensitive (library versions, API surfaces,
   competitive landscape, industry adoption claims)
2. For each claim, search for current evidence that confirms or contradicts it
3. Produce a structured finding for each claim checked

You MUST NOT:
- Suggest edits to the documents
- Rewrite any section
- Offer opinions on the architecture
- Suggest new features or directions
- Change the documents in any way

Output format for each finding:
---
FINDING: [sequential number]
DOCUMENT: [which document]
SECTION: [which section]
CLAIM: [the specific claim being validated]
STATUS: CONFIRMED | CONTRADICTED | EVOLVED | STALE | UNVERIFIABLE
EVIDENCE: [what you found, with sources]
SEVERITY: LOW | MEDIUM | HIGH | CRITICAL
  LOW = cosmetic (version number changed, minor detail)
  MEDIUM = claim still directionally correct but details shifted
  HIGH = claim is materially wrong or landscape has changed significantly
  CRITICAL = architectural assumption invalidated
DATE: [today]
---
```

### What Gets Checked

Category 1 — Factual claims with expiration dates:
- "Agent SDK V1 released September 29, 2025" — still V1? V2 out?
- "Kubernetes Agent Sandbox is a SIG Apps subproject" — status changed?
- "November 2025 runC CVEs" — patched? new CVEs?
- "OpenClaw 180K+ stars" — current count? project still active?
- Library versions (Pydantic AI, Temporal SDK, rmcp, etc.)

Category 2 — Competitive landscape:
- New entrants we didn't analyze?
- Did any existing project adopt our architecture's approach?
- Has anyone built the composable trait-boundary approach?
- Any Rust agent crates that overlap with Layer 0?

Category 3 — Architectural assumptions:
- "The Agent SDK runs Claude Code as a subprocess" — still true?
- "Agent SDK doesn't expose model provider interface" — still true?
- "Temporal + orchestration are inseparable" — any system that separated them?
- "No existing project combines Temporal + container isolation + MCP" — still true?

Category 4 — Ecosystem changes:
- New Temporal features relevant to agent orchestration?
- MCP spec changes?
- New Rust async patterns that affect trait design?
- Anthropic product changes (new models, new SDK features)?

### Scheduling

Run weekly. Not daily — the landscape doesn't change that fast, and
more frequent runs increase the chance of false positives from transient
search results.

Use: cron job calling a script that invokes the Claude API with the
validator prompt + document contents, appends findings to .validation/findings.md.

### Implementation

The simplest version is a bash script:

```bash
#!/usr/bin/env bash
# validate.sh — run from repo root

set -euo pipefail

FINDINGS=".validation/findings.md"
LAST_RUN=".validation/last-run.json"
mkdir -p .validation

# Collect documents
DOCS=""
for f in agentic-decision-map-v3.md composable-agentic-architecture.md HANDOFF.md; do
  if [ -f "$f" ]; then
    DOCS+="<document name=\"$f\">\n$(cat "$f")\n</document>\n\n"
  fi
done

# Call Claude API with validator prompt
# (use claude CLI, or curl to API directly)
RESULT=$(claude -p "You are a research validator. [full prompt above]

Here are the documents to validate:

$DOCS

Search the web for current information to check time-sensitive claims.
Focus on CRITICAL and HIGH severity findings. Skip obvious LOW findings
unless you find something surprising.

Output your findings in the structured format specified.")

# Append to findings log
echo "" >> "$FINDINGS"
echo "# Validation Run: $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$FINDINGS"
echo "" >> "$FINDINGS"
echo "$RESULT" >> "$FINDINGS"

# Update last-run metadata
echo "{\"timestamp\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\", \"findings_count\": $(echo "$RESULT" | grep -c "^FINDING:" || true)}" > "$LAST_RUN"

echo "Validation complete. Check $FINDINGS"
```

For notification, the script can:
- Exit with non-zero if any CRITICAL or HIGH findings
- Post to a Slack webhook if findings exist
- Create a GitHub issue automatically
- Simply: send you an email / push notification via ntfy.sh

The ntfy.sh approach is simplest for a solo developer:

```bash
# At end of validate.sh
HIGH_COUNT=$(echo "$RESULT" | grep -c "SEVERITY: \(HIGH\|CRITICAL\)" || true)
if [ "$HIGH_COUNT" -gt 0 ]; then
  curl -d "Validation found $HIGH_COUNT high/critical findings. Check .validation/findings.md" \
    ntfy.sh/your-private-topic
fi
```

---

## Part 2: Interrupting and Re-Orienting Claude Code

### The Problem

You're running Claude Code on the implementation. The validator finds something
that changes an architectural assumption. You need to:
1. Stop current work cleanly
2. Update the source-of-truth documents
3. Re-orient the agent to the new reality
4. Resume without context rot

### The Solution: Single Source of Truth + Session Boundaries

The key insight: Claude Code's context window IS the context rot vector.
The longer a session runs, the more stale instructions and abandoned
approaches accumulate. The fix is architectural, not procedural.

**Rule 1: Documents are the source of truth, not conversation history.**

Claude Code should read HANDOFF.md, composable-agentic-architecture.md,
and agentic-decision-map-v3.md at the START of every session. Not "remember
what we discussed" — read the files. If the files change, the agent's
understanding changes on next session start. This is why the documents
must be precise and complete.

**Rule 2: Sessions are short and task-scoped.**

Don't run one marathon session for "implement Phase 1." Run:
- Session 1: "Create crate skeleton with Cargo.toml and module structure per HANDOFF.md"
- Session 2: "Implement content.rs and id.rs per HANDOFF.md"
- Session 3: "Implement turn.rs per HANDOFF.md"
- etc.

Each session re-reads the documents. If you changed them between sessions,
the agent picks up the changes automatically. No re-orientation needed —
the source of truth IS re-oriented.

**Rule 3: Interrupt = end session + update docs + start new session.**

When the validator flags something:
1. End the current Claude Code session (Ctrl+C or let it finish its current task)
2. Review the finding. Decide whether it warrants a document change.
3. If yes: update the relevant document(s) yourself. Be precise.
4. If the change affects in-progress code: note what needs to change in HANDOFF.md
   under a new section "## Corrections" or update the relevant phase.
5. Start a new Claude Code session. It reads the updated documents. Done.

No "hey, we changed direction" conversation. No "forget what I said before."
The documents changed. The agent reads the documents. The agent follows the
documents.

### CLAUDE.md as the Coordination Layer

The CLAUDE.md in your repo should encode this protocol:

```markdown
# CLAUDE.md

## Session Protocol

At the start of every session:
1. Read HANDOFF.md completely — this is the implementation spec
2. Read composable-agentic-architecture.md if you need to understand
   WHY a design decision was made
3. Read agentic-decision-map-v3.md if you need to understand the
   full design space at a decision point
4. Check .validation/findings.md for any CRITICAL or HIGH findings
   that might affect your current task — if found, STOP and report
   to the user before proceeding

## Working Protocol

- Implement exactly what HANDOFF.md specifies. Do not improvise.
- If HANDOFF.md is ambiguous, check composable-agentic-architecture.md
  for clarification.
- If still ambiguous, ASK — do not guess.
- All trait signatures must match HANDOFF.md exactly unless a Correction
  has been noted.
- Run tests after every file you create or modify.

## What Not To Do

- Do not reorganize the module structure unless HANDOFF.md says to
- Do not add dependencies beyond what HANDOFF.md specifies
- Do not rename types or traits
- Do not add methods to protocol traits
- Do not "improve" the architecture
```

### The Correction Pattern

When you need to change direction mid-implementation, add a Corrections
section to HANDOFF.md:

```markdown
## Corrections

### C-001: SignalPayload needs sequence number (2026-02-25)

FINDING: Temporal signals are unordered. Without sequence numbers,
signal replay after crash can reorder messages.

CHANGE: Add `sequence: u64` field to `SignalPayload` in effect.rs.
Update all references.

AFFECTS: effect.rs, any code already using SignalPayload.

### C-002: Environment::run should take Arc<dyn Turn> not &dyn Turn (2026-02-26)

FINDING: Docker environment needs to send the Turn across a thread
boundary (spawning container). &dyn Turn doesn't work across threads
without 'static.

CHANGE: Change Environment::run signature. See updated trait in
environment.rs section.

AFFECTS: environment.rs, all Environment implementations.
```

Claude Code reads this at session start. It knows what changed, why,
and what to fix. No conversation needed. No drift possible — the
correction is in the document, versioned in git, and will be read
by every future session.

---

## Part 3: The Full Workflow

```
         YOU                    VALIDATOR              CLAUDE CODE
          │                        │                       │
          │   (weekly cron)        │                       │
          │                        │                       │
          │                   ┌────┴────┐                  │
          │                   │ Read    │                  │
          │                   │ docs    │                  │
          │                   │ Search  │                  │
          │                   │ web     │                  │
          │                   │ Write   │                  │
          │                   │ findings│                  │
          │                   └────┬────┘                  │
          │                        │                       │
          │◄── notification ───────┤                       │
          │    (if HIGH/CRITICAL)  │                       │
          │                        │                       │
     ┌────┴────┐                   │                       │
     │ Review  │                   │                       │
     │ finding │                   │                       │
     └────┬────┘                   │                       │
          │                        │                       │
     Is it real?                   │                       │
     ├── No: ignore                │                       │
     │                             │                       │
     └── Yes:                      │                       │
          │                        │                       │
     ┌────┴────┐                   │                       │
     │ Update  │                   │                  (session N
     │ source  │                   │                   running)
     │ docs    │                   │                       │
     └────┬────┘                   │                       │
          │                        │                       │
     Need to interrupt?            │                       │
     ├── No: changes picked        │                       │
     │   up next session           │                       │
     │                             │                       │
     └── Yes:                      │                       │
          │                        │                       │
     ┌────┴────────┐               │                       │
     │ End current │               │                  ◄── Ctrl+C
     │ session     │               │                       │
     │             │               │                       │
     │ Add to      │               │                       │
     │ Corrections │               │                       │
     └────┬────────┘               │                       │
          │                        │                       │
          │                        │                  ┌────┴────┐
          │   "start Phase N task" │                  │ New     │
          │ ──────────────────────────────────────►   │ session │
          │                        │                  │ Reads   │
          │                        │                  │ docs +  │
          │                        │                  │ correct-│
          │                        │                  │ ions    │
          │                        │                  └─────────┘
```

---

## Part 4: Directory Structure

### Two repos, not one.

**Repo 1: The Layer 0 crate** (new repo)
```
CORE/                         ← new repo, named whatever the crate is called
  Cargo.toml
  src/
    lib.rs
    content.rs
    turn.rs
    effect.rs
    orchestrator.rs
    state.rs
    environment.rs
    hook.rs
    lifecycle.rs
    error.rs
    id.rs
  CLAUDE.md                   ← session protocol for Claude Code
  HANDOFF.md                  ← implementation spec
  composable-agentic-architecture.md
  agentic-decision-map-v3.md
  .validation/
    findings.md
    last-run.json
  validate.sh
```

**Workspace: neuron** (redesign/v2 — executed)

> **Note:** The plan above has been executed. The two-repo approach was merged into
> a single workspace on the `redesign/v2` branch. The actual workspace structure is:

```
neuron/
  layer0/                           ← Layer 0: protocol traits + message types

  turn/
    neuron-turn/                     ← Layer 1: turn provider abstraction
    neuron-turn-kit/                 ← Layer 1: turn decomposition primitives
    neuron-context/                  ← Layer 1: conversation context
    neuron-tool/                     ← Layer 1: tool registry + middleware
    neuron-mcp/                      ← Layer 1: MCP client integration

  op/
    neuron-op-react/                 ← Layer 1: ReAct operator
    neuron-op-single-shot/           ← Layer 1: single-shot operator

  provider/
    neuron-provider-anthropic/       ← Layer 1: Anthropic provider
    neuron-provider-openai/          ← Layer 1: OpenAI provider
    neuron-provider-ollama/          ← Layer 1: Ollama provider

  orch/
    neuron-orch-local/               ← Layer 2: local orchestrator
    neuron-orch-kit/                 ← Layer 2: orchestration utilities

  effects/
    neuron-effects-core/             ← Layer 2: effect executor trait
    neuron-effects-local/            ← Layer 2: local effect interpreter

  state/
    neuron-state-memory/             ← Layer 3: in-memory state store
    neuron-state-fs/                 ← Layer 3: filesystem state store

  env/
    neuron-env-local/                ← Layer 4: local environment

  secret/
    neuron-secret/                   ← Layer 4: secret trait + 6 backends

  auth/
    neuron-auth/                     ← Layer 4: auth trait + 4 backends

  crypto/
    neuron-crypto/                   ← Layer 4: crypto trait + 2 backends

  hooks/
    neuron-hooks/                    ← Layer 5: hook registry
    neuron-hook-security/            ← Layer 5: security hooks

  neuron/                            ← Umbrella crate (re-exports)

See `NEURON-REDESIGN-PLAN.md` for the full 6-layer architecture and design rationale.

Don't touch neuron until Phase 1-2 are solid and published. The trait
crate must be right before anything depends on it. If you start modifying
neuron simultaneously, you get two moving targets and every change to the
traits cascades into neuron changes. Sequential, not parallel.
