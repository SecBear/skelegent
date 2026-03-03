# Are the "New" Decisions Actually New?

> An honest audit of the 6 proposed new decision points from the gap analysis,
> tested against real agentic architectures.

---

## Method

For each proposed decision, I ask:
1. Does an existing decision already cover this concern?
2. Can I find a real system where this concern was answered *independently*
   of the existing decision it might map to?
3. If two systems make the same choice on the existing decision but diverge
   on the proposed new one, it's genuinely independent. If they always
   co-vary, it's a sub-decision or option expansion.

---

## D-NEW-1: Structured Output Enforcement

**Claim:** "How does the system guarantee the shape of model outputs?"

**Test against existing decisions:**
- D4C (Backfill) asks "how do tool results re-enter context?" — this is about
  *inputs to the model*, not *outputs from the model*.
- D3A (Model) asks "which model?" — doesn't address output constraints.
- No existing decision covers constraining the model's output shape.

**Real system test:**

| System | D3A (Model) | D4C (Backfill) | Output enforcement |
|--------|------------|----------------|-------------------|
| Claude Code | Sonnet/Opus | Formatted | None — freeform text/tool_use |
| OpenAI Agents SDK | GPT-4o | Formatted | **Strict JSON Schema decoding** |
| CrewAI | Configurable | Raw | **Pydantic model validation** |
| Aider | Two-tier | Formatted | **Edit format parser with retries** |

Claude Code and OpenAI Agents SDK can use the *same model* (both support
multiple providers) and the *same backfill strategy*, but diverge completely on
output enforcement. Aider's edit format is a structural constraint on output
shape that has nothing to do with model selection or backfill.

**Verdict: Genuinely new.** No existing decision covers this. It's not a
sub-decision of D3A, D4C, or anything else. Every agent makes this choice
(explicitly or implicitly) and the choices are independent of the other 23.

BUT — it could be argued this is a sub-decision of D3A (inference), since it's
a parameter you send *with* the inference request. The counter-argument: model
selection and output constraint are independently variable. You can use the same
model with or without schema enforcement. That independence makes it a separate
decision.

**Recommendation: Add as D3D (Output Shape).** It belongs in the inference
cluster, not as a standalone new category.

---

## D-NEW-2: Agent Identity, Trust, and Authentication

**Claim:** "How is the agent itself authenticated across tool and agent
boundaries?"

**Test against existing decisions:**
- D4B (Credentials) asks "does the agent see secrets?" — about credential
  *handling*, not agent *identity*.
- D2A (Identity) asks "how does the agent know what it is?" — about behavioral
  specification (prompts vs structure), not authentication.
- D4A (Isolation) asks "how trusted is generated code?" — about execution
  sandboxing, not identity.

**Real system test:**

| System | D4B (Credentials) | D2A (Identity) | Agent authentication |
|--------|-------------------|----------------|---------------------|
| Claude Code | Proxy-brokered (never sees tokens) | Markdown agent def | **OAuth 2.1 via MCP, SPIFFE-style workload identity** |
| NanoClaw | Mounted in container | Environment-only | **Container identity = agent identity** |
| OpenClaw | In-process env vars | Maximal prompt | **User's credentials = agent's credentials** |
| Devin | Unknown | Task-scoped | **Dedicated service identity with approval checkpoints** |

OpenClaw and NanoClaw make the *same* D4B choice (credentials available to
agent) but completely different agent identity choices. Claude Code and OpenClaw
make different D4B choices but *could* use the same agent identity model.

However — look more carefully. D2A already covers "how does the agent know what
it is?" and the Decision Map v3 already notes that identity can be
"environment-derived" (NanoClaw). The *authentication* aspect (OAuth, SPIFFE)
is really an implementation detail of D4B and D4A. When you ask "does the agent
see secrets?" you're implicitly asking how it authenticates.

**The real question is A2A delegation.** When Claude Code delegates to a
sub-agent, how do permissions propagate? That's not D4B (which is about the
agent seeing *tool* credentials) — it's a composition concern about trust
between agents.

**Verdict: Not cleanly new. It's spread across three existing decisions.**
- Agent behavioral identity → D2A (already covered)
- Agent authentication to tools → D4B (expand the options)
- Agent-to-agent trust delegation → C1 (child context) needs expansion

**Recommendation: Don't add as new. Expand D4B to include "agent workload
identity" options, and expand C1 to address permission/trust propagation.**

---

## D-NEW-3: Reasoning Strategy Selection

**Claim:** "How hard does the model think? (separate from which model)"

**Test against existing decisions:**
- D3A (Model Selection) asks "which model handles this?" and lists single →
  two-tier → three-tier → difficulty-aware.

**Real system test:**

| System | D3A (Model) | Reasoning effort |
|--------|------------|-----------------|
| Claude Code | Three-tier (Opus/Sonnet/Haiku) | **Also varies thinking budget per task** |
| Cursor | Composer selects model | **Also selects "thinking" level** |
| Aider | Two-tier (architect/editor) | **No thinking budget control** |
| Simple chatbot | Single model | **Single effort level** |

Here's the key test: Claude Code using Sonnet 4.6 with low thinking budget vs.
Sonnet 4.6 with maximum thinking budget. Same model (D3A answer is identical),
completely different reasoning effort. The cost difference is 4-10x. This is a
real, independent axis.

But wait — re-read D3A's spectrum: "single ↔ two-tier ↔ three-tier ↔
difficulty-aware." The "difficulty-aware" option already implies routing based
on task complexity. And "two-tier: architect/editor" is really about
*reasoning effort* — the architect "thinks harder" than the editor. The
decision map already captures the *concept* of varying cognitive effort; it just
frames it as "use different models" rather than "use same model at different
effort levels."

The 2025-2026 shift is that providers now expose thinking budget as an
*explicit parameter on the same model*. This makes the two-level routing
paradigm visible: you're not swapping models, you're dialing effort.

**Verdict: Not genuinely new. It's an expansion of D3A.** The existing decision
already covers "match capability to task difficulty." What's changed is that the
*mechanism* now includes effort tuning on a single model, not just model
swapping. The table needs a new row, not a new decision.

**Recommendation: Add to D3A's table:**

| **Two-level routing** — select model AND reasoning effort per task | Best | Best — fine-grained cost/quality control | Claude Code (Sonnet + thinking budget), Cursor |

And update the engineering consideration to note that model selection and
compute effort are now independently tunable axes within the same decision.

---

## D-NEW-4: Evaluation, Benchmarking, and Release Gating

**Claim:** "How do you know the agent works? How do you prevent regressions?"

**Test against existing decisions:**
- L5 (Observability) asks "what can you see?" — about runtime visibility.
- No existing decision addresses pre-deployment or continuous evaluation.

**Real system test:**

| System | L5 (Observability) | Evaluation strategy |
|--------|-------------------|---------------------|
| Claude Code | Hooks + tracing | **No formal eval gating** |
| Devin | Internal telemetry | **SWE-bench Verified as release gate** |
| Braintrust users | OTel traces | **CI/CD eval with PR comments + auto-rollback** |
| Most open source agents | Logging | **None** |

Systems with identical observability choices make completely different evaluation
decisions. Devin and Claude Code both have sophisticated observability but
diverge on whether eval gates releases.

But — is this really an *architectural* decision about the agent, or a
*development process* decision about the team? The decision map claims to cover
"every engineering decision you face when building an agentic AI system." Eval
gating is an engineering decision. But it's not a decision about the agent's
*runtime architecture* — it's about the CI/CD pipeline around it.

Compare to L5 (Observability): that's a runtime architectural decision (what
hooks exist, what's traced). Eval gating is about *what you do with the traces*
after the fact.

**Verdict: Genuinely new, but arguably out of scope.** It's a real decision
that every team makes, and it's independent of the 23. But it's a development
lifecycle decision, not a runtime architecture decision. The decision map's
three layers (Turn, Composition, Lifecycle) are all about *what happens at
runtime*.

**Recommendation: Add to the "Open Questions" section rather than the decision
list. Or add as L6 if the map's scope expands to cover development lifecycle.**

---

## D-NEW-5: Agent Versioning & Rollback

**Claim:** "How are agents versioned, deployed, and rolled back?"

**Test against existing decisions:**
- No existing decision addresses this.

**Same analysis as D-NEW-4.** This is a deployment/operations concern, not a
runtime architecture concern. It's clearly a real decision (Devin uses canary
deploys; most open-source agents have no versioning), but it lives outside the
Turn/Composition/Lifecycle framework.

| System | Versioning approach |
|--------|-------------------|
| Claude Code | Implicit (latest CLI version) |
| Devin | **Formal releases with rollback** |
| CrewAI apps | **User manages prompt/tool versions** |
| LangGraph Cloud | **Graph versioning with deployment slots** |

**Verdict: Genuinely new, genuinely out of scope.**

**Recommendation: Same as D-NEW-4. Document in "Open Questions" or expand the
map's scope to include a "Development & Operations" layer.**

---

## D-NEW-6: Autonomy Level Classification

**Claim:** "How much can the agent do without human approval?"

**Test against existing decisions:**
- D4A (Isolation) asks "how trusted is generated code?" — includes "permission
  gate" as an option.
- D5 (Exit) includes "observer halt."
- C5 (Observation) asks "who watches execution?" — includes "human-in-the-loop."

**Real system test:**

| System | D4A (Isolation) | C5 (Observation) | Autonomy level |
|--------|----------------|------------------|---------------|
| Claude Code | Permission gate | Human-in-the-loop | **Execute with approval** |
| Aider | No isolation | No observer | **Fully autonomous** (within session) |
| Devin | Container | Human checkpoints | **Autonomous with mandatory plan/PR review** |
| OpenHands | Docker + SecurityAnalyzer | Risk-graded auto-approval | **Dynamic: auto-approve low-risk, pause high-risk** |

This is interesting. Claude Code and Devin both use "permission gate" (D4A) and
"human-in-the-loop" (C5), but their autonomy models are quite different: Claude
Code asks per-tool-call, Devin asks at plan and PR boundaries. OpenHands
introduces *dynamic* autonomy based on risk scoring.

But — trace these back. Claude Code's "per-tool-call approval" is an
implementation of D4A's "permission gate." Devin's "plan checkpoint" is an
implementation of C5's "human-in-the-loop." OpenHands' risk scoring is a
*refinement* of D4A's permission gate (auto-approve low risk, gate high risk).

The "autonomy level" concept is really a *policy that combines D4A and C5
choices into a coherent posture*. It's not a new independent axis — it's a
named configuration of existing axes.

Read-only = D4A:no execution + C5:human approves everything
Propose = D4A:no execution + C5:human executes
Execute with approval = D4A:permission gate + C5:human-in-the-loop
Autonomous with guardrails = D4A:sandbox + C5:guardrails
Fully autonomous = D4A:any + C5:no observer

**Verdict: Not genuinely new. It's a composite of D4A and C5.** Useful as a
*taxonomy* for naming common configurations, but not an independent decision
axis. You don't make this decision separately from D4A and C5 — it *is* your
D4A + C5 choices.

**Recommendation: Add as a "Common Configurations" sidebar in the Composition
section, mapping named autonomy levels to D4A + C5 combinations.**

---

## Final Scorecard

| Proposed Decision | Verdict | Action |
|-------------------|---------|--------|
| D-NEW-1: Structured Output | **Genuinely new** | Add as D3D (Output Shape) in inference cluster |
| D-NEW-2: Agent Identity & Trust | **Not new** — spread across D2A, D4B, C1 | Expand D4B and C1 option tables |
| D-NEW-3: Reasoning Strategy | **Not new** — expansion of D3A | Add two-level routing row to D3A table |
| D-NEW-4: Evaluation & Gating | **New but out of scope** | Add to Open Questions or new L6 |
| D-NEW-5: Versioning & Rollback | **New but out of scope** | Add to Open Questions or new L6 |
| D-NEW-6: Autonomy Levels | **Not new** — composite of D4A + C5 | Add as named configurations sidebar |

**Net result: 1 genuinely new in-scope decision (D3D: Output Shape), 2 new
out-of-scope decisions (eval gating, versioning), and 3 that are expansions of
existing decisions.**

The proposed jump from 23 → 29 was overcounting. The honest count is:
- **24 decisions** if we add D3D and keep scope to runtime architecture
- **26 decisions** if we also add L6 (Eval/Gating) and L7 (Versioning/Rollback) by expanding scope to development lifecycle
- Plus significant expansions to D3A, D4B, C1, and D5 option tables

---

*Audited 2026-03-03. Method: independence testing via real system comparison.*
