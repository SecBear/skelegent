# Agentic Decision Map: Gap Analysis (March 2026)

> Synthesized from 8 parallel deep research runs covering all 23 decisions in
> Neuron's Agentic Decision Map v3. Each section maps findings back to Neuron's
> existing decisions, identifies gaps, and proposes new decision points.

---

## Summary of Findings

The Decision Map's 23-decision framework holds up remarkably well. No research
run found a production system that breaks the framework's fundamental structure
(Turn / Composition / Lifecycle). However, the research reveals:

1. **1 genuinely new in-scope decision** (D3D: Output Shape) that no existing decision covers
2. **2 potentially new decisions** that are real but arguably out of scope (eval gating, versioning)
3. **3 proposed decisions that decompose into existing ones** (agent identity, reasoning strategy, autonomy levels)
4. **4 existing decisions with incomplete option spaces** that need significant expansion
5. **Numerous new implementations** that should be added to the existing tables

> **Correction note (same session):** The original version of this document
> proposed 6 genuinely new decisions (23 → 29). After rigorous independence
> testing against real agentic architectures, only 1 is genuinely new and
> in-scope. See `decision-map-new-decision-audit.md` for the full analysis.
> The decision map has been updated to 24 decisions (23 + D3D: Output Shape).

---

## Part 1: Missing Decision Points

These are architectural decisions that production systems in 2025-2026 are
actively making, that do not fit cleanly into any of the existing 23 decisions.

### D-NEW-1: Structured Output Enforcement

**Key question:** How does the system guarantee the shape of model outputs?

| Approach | Guarantee | Used By |
|----------|-----------|---------|
| Best-effort JSON mode | Soft — model usually complies | Most systems (default) |
| JSON Schema at generation time (strict decoding) | Hard — 100% schema adherence | OpenAI Structured Outputs |
| Schema propagation across agent graph | Contract — nodes compose safely | Multi-agent pipelines |

**Why it's missing:** The current framework treats model output as either "text"
or "tool calls" (D4C backfill). It doesn't address the increasingly critical
decision of *constraining output shape* as a type system for agent pipelines.
This is foundational for safe multi-agent composition — a child agent returning
malformed output breaks the parent's reasoning.

**Neuron impact:** Layer 0 wire types should consider whether `OperatorOutput`
carries schema validation metadata. Turn implementations should support
schema-enforced generation where the provider supports it.

---

### D-NEW-2: Agent Identity, Trust, and Authentication

**Key question:** How is the agent identified, authenticated, and authorized
across tool and agent boundaries?

| Approach | Agent Sees Creds? | Trust Model | Used By |
|----------|------------------|-------------|---------|
| User impersonation — agent acts as user | User's full creds | Implicit | Most simple agents |
| Workload identity (SPIFFE/SPIRE) | Dedicated identity | Delegated | Enterprise systems |
| OAuth 2.1 + PKCE per-tool scoping | Scoped tokens | Least-privilege | MCP Nov 2025 spec |
| Intent-based natural language scopes | Ephemeral scoped | User-approved | Emerging research |
| Dynamic trust scoring + JIT access | Adaptive scoped | Continuous eval | Emerging research |

**Why it's missing:** D4B (Credentials) covers whether the agent *sees* secrets.
This new decision covers the agent's own identity — how it authenticates to
tools, how permissions propagate in A2A delegation, and how trust is established
between agents from different vendors/frameworks.

**Neuron impact:** The `auth/` crates already address some of this, but the
protocol layer doesn't model agent-to-agent trust propagation or delegation
semantics. The `Environment` trait's credential injection should be expanded to
support identity delegation chains.

---

### D-NEW-3: Planning and Reasoning Strategy Selection

**Key question:** How does the agent select its reasoning approach for a given
task?

| Approach | Cost | Quality | Used By |
|----------|------|---------|---------|
| Direct response (no explicit reasoning) | Lowest | Lowest | Simple tool-calling |
| Chain-of-Thought (CoT) | Low | Good | Default for most |
| Extended thinking / deep reasoning | High | Highest | Claude thinking, o1 |
| Tree-of-Thought (ToT) | Very high | Best for search | Research systems |
| Reflection / self-critique | Medium | Good for iteration | Evaluator-optimizer loops |
| Dynamic strategy selection per task | Variable | Optimal when calibrated | Emerging |

**Why it's missing:** D3A (Model Selection) covers *which model*, but not *how
hard it thinks*. The two-level routing paradigm (model + compute effort) is now
standard in production. Anthropic exposes thinking budget controls. OpenAI
exposes reasoning effort. This is a separate decision axis from model selection.

**Neuron impact:** The `Provider` trait should support reasoning effort/thinking
budget as a parameter. The turn engine should be able to request different
reasoning modes. This is distinct from model selection and should be a separate
configuration axis.

---

### D-NEW-4: Evaluation, Benchmarking, and Release Gating

**Key question:** How do you know the agent works, and how do you prevent
regressions?

| Approach | Feedback Loop | Used By |
|----------|--------------|---------|
| No formal evaluation | None | Prototypes |
| Offline benchmark suite (GAIA, SWE-bench) | Pre-release gate | Production teams |
| CI/CD-integrated evals (PR comments) | Every commit | Braintrust |
| Online evaluation with drift detection | Continuous | Enterprise systems |
| Automated rollback on eval regression | Closed-loop | Emerging best practice |

**Why it's missing:** L5 (Observability) covers *seeing* what the agent does.
This new decision covers *judging* whether it's working correctly and using that
judgment to gate releases. The evaluation-release-rollback loop is a distinct
architectural concern.

**Neuron impact:** Not a runtime concern for Layer 0, but the umbrella crate
and examples should demonstrate evaluation integration. The hooks system could
emit evaluation-relevant events.

---

### D-NEW-5: Agent Versioning, Rollback, and Registry

**Key question:** How are agents versioned, deployed, and rolled back?

| Approach | Risk | Used By |
|----------|------|---------|
| No versioning — latest always wins | High | Development |
| Semantic versioning of prompts/tools/policies | Medium | Production teams |
| Immutable releases + canary/blue-green deploy | Low | Enterprise |
| Internal agent registry with signing | Lowest | Enterprise governance |

**Why it's missing:** This is the DevOps equivalent for agents. None of the 23
decisions address how agent configurations (prompt + model + tools + policies)
are versioned as a unit, deployed safely, or rolled back.

**Neuron impact:** The orchestration layer should support named, versioned
operator configurations. The umbrella crate should make it natural to snapshot
and restore operator configurations.

---

### D-NEW-6: Autonomy Level Classification

**Key question:** How much can the agent do without human approval?

| Level | Description | Used By |
|-------|-------------|---------|
| Read-only | Agent observes, cannot act | Monitoring agents |
| Propose | Agent suggests actions, human executes | Conservative deployments |
| Execute with approval | Agent acts after human sign-off | Claude Code (default) |
| Autonomous with guardrails | Agent acts freely within bounds | Production coding agents |
| Fully autonomous | No human in loop | Devin (within checkpoints) |

**Why it's missing:** D5 (Exit) and C5 (Observation) touch on human
intervention, but neither addresses the systematic classification of *how much
autonomy* the agent has. This is a first-class governance decision that
determines the shape of the entire human-agent interaction.

**Neuron impact:** The `Environment` or `Hooks` system should support declaring
an autonomy level that affects tool execution approval flows. This could be a
policy on the `ToolExecutionStrategy`.

---

## Part 2: Existing Decisions with Incomplete Option Spaces

These decisions exist in the framework but are missing significant new options
discovered in 2025-2026.

### D2C (Memory): Missing Tiers and Architectures

The hot/warm/cold/structural taxonomy is incomplete. Add:

| New Tier | Description | Example |
|----------|-------------|---------|
| **Context-KV infrastructure** | Dedicated hardware tier for staging KV caches between GPU HBM and storage | NVIDIA ICMS (G3.5) |
| **Graph memory** | First-class knowledge graphs for multi-hop relational reasoning | Mem0 + Neptune, Neo4j GraphRAG |
| **Artifact/trace tier** | Execution traces, diffs, code maps for audit and replay | Agent Trace spec (Cognition AI) |
| **Policy/rules tier** | Hierarchical rule files enforced across sessions | CLAUDE.md, .cursorrules |
| **Topicized episodic files** | Small index in hot memory + on-demand topic files on disk | Claude Code auto-memory |

Also missing: **Hybrid Graph RAG** as a retrieval architecture (vector + lexical
+ graph fusion via RRF), which is becoming the 2026 baseline for enterprise
memory retrieval.

### D2D (Tools): Missing Management Patterns

The tool surface decision is missing critical new patterns:

| New Pattern | Description | Impact |
|-------------|-------------|--------|
| **Tool Search / Discovery-first** | Lightweight search tool discovers relevant tools on-demand (85-95% context savings) | Anthropic Tool Search Tool |
| **Lazy / Deferred loading** | Tool schemas loaded only when needed (`defer_loading: true`) | Anthropic best practice |
| **Programmatic tool calling** | Model generates code to orchestrate tools (up to 98.7% token reduction) | Anthropic, production systems |
| **MCP Gateway** | Central control plane federating many tool servers with auth, caching, rate limiting | Emerging best practice |
| **Registry-first discovery** | MCP Registry as searchable catalog for tools | MCP ecosystem |
| **Tool execution metadata** | Concurrency hints, cost estimation, timeout hints in tool schema | **GAP — no standard exists** |

The last item is directly relevant to Neuron: the framework already has
`ToolExecutionStrategy` with Shared/Exclusive hints, which is *ahead* of the
industry. This should be formalized and potentially contributed back as a
proposed MCP extension.

### D3A (Model Selection): Two-Level Routing

Model selection is now a two-level decision:

1. **Level 1: Model selection** — which model family/provider
2. **Level 2: Compute/effort selection** — how hard the model thinks

This is missing from the decision map. Production stacks report 4-10x cost
reduction with cost-aware two-level routing. Additional missing techniques:

- Speculative parallel execution (send to multiple models, use first good result)
- Consensus/verification passes for safety-critical outputs
- Cache-aware prompt planning (optimize for cache hits)
- Cascading with quality gates (cheap model first, escalate on uncertainty)

### D5 (Exit): Missing Termination Strategies

The exit decision is missing several production-proven strategies:

| New Strategy | Description |
|-------------|-------------|
| **LLM self-assessment** | Dedicated reasoning pass: "is the goal achieved?" with evidence |
| **Programmatic verification** | Machine-verifiable checks (file exists, API state, schema conformance) |
| **Progress/loop detection** | Semantic similarity of recent thoughts; repeated tool calls detection |
| **Explicit state machine** | PLANNING → EXECUTING → WAITING_FOR_APPROVAL → TERMINATED |
| **Stuck detection + human escalation** | Checkpoint state and route to human instead of terminating |
| **Infeasibility recognition** | Detecting when task is impossible and terminating with "infeasible" |

The research emphasizes: **mis-termination is more common than model failure**.
This should inform Neuron's `ExitReason` enum — it should include variants like
`Infeasible`, `StuckDetected`, `HumanEscalation`.

---

## Part 3: Major New Implementations to Track

### MCP Evolution (Nov 2025 spec)
- Tasks API for long-running async operations
- OAuth 2.1 authorization framework
- Client ID Metadata Documents (CIMD)
- Governed by Linux Foundation's Agentic AI Foundation
- 10,000+ public servers

### OpenAI Responses API (March 2025)
- Agentic loop: multi-tool calls in single request
- Built-in tools: web search, file search, computer use
- Connectors for remote MCP servers
- Conversations API for stateful context
- `/responses/compact` endpoint for context compaction
- Async background tasks

### LangGraph Durability
- Graph-native checkpointing with time-travel
- Pending writes (save partial progress in failed super-steps)
- sync/async/exit persistence modes
- Native human-in-the-loop integration

### OpenTelemetry for Agents
- GenAI SIG standardizing `gen_ai.*` semantic conventions
- Emerging `agent.*` attributes for agent-specific telemetry
- OTel Collector for fan-out, redaction, enrichment
- Adopted by LangSmith, Datadog, Langfuse, Arize Phoenix

### Composition Patterns Beyond the Six Primitives
- **Market/Auction**: Agents bid for tasks, trade resources
- **Debate/Consensus**: Agents vote, reach quorum
- **Blackboard/Event Bus**: Async shared-state communication
- **Cyclical state graphs**: Explicit cycles for plan-act-observe loops
- **Agent teams with file-based coordination**: Lock files on shared repos

These don't break the six-primitive model (they're compositions of the
existing primitives) but are worth documenting as named patterns.

---

## Part 4: What Neuron Already Gets Right

The research validates several of Neuron's architectural choices:

1. **Effects boundary** — The declaration-separated-from-execution principle is
   exactly what production systems are converging on. Operators declaring intent,
   orchestrators executing, is the winning pattern.

2. **Tool execution metadata** — Neuron's `ToolExecutionStrategy` with
   Shared/Exclusive concurrency hints is *ahead* of the industry. No standard
   exists for this in MCP yet.

3. **Hook system** — The pre/post inference/tool hook points map cleanly to
   what OpenTelemetry GenAI is standardizing. Neuron should emit OTel-compatible
   spans from hooks.

4. **ExitReason enum** — The explicit enumeration of exit reasons is exactly
   what production guidance recommends (explicit state machine for termination).

5. **Composability philosophy** — The six composition primitives remain complete.
   New patterns (market, debate, cyclical graphs) are compositions of the
   existing six.

6. **Layer 0 stability** — Protocol stability as the foundation is the exact
   principle that MCP's success validates.

---

## Part 5: Recommended Actions for Neuron

### High Priority (Architectural Impact)

1. **Add D-NEW-1 (Structured Output) to the decision map.** Consider whether
   Layer 0 `InferenceRequest` should carry output schema constraints.

2. **Add D-NEW-3 (Reasoning Strategy) to the decision map.** The `Provider`
   trait should support reasoning effort as a parameter distinct from model
   selection.

3. **Expand D5 (Exit) in the decision map and code.** Add `Infeasible`,
   `StuckDetected`, `LoopDetected`, `HumanEscalation` to `ExitReason`.

4. **Expand D2D (Tools) for lazy loading and search.** The tool system should
   support deferred schema loading and tool search as first-class patterns.

### Medium Priority (Ecosystem Alignment)

5. **Add OTel semantic convention support to hooks.** Emit `gen_ai.*` and
   `agent.*` attributes from the hook system. This is the industry standard
   converging now.

6. **Add D-NEW-2 (Agent Identity) to the decision map.** Model identity
   delegation chains in the environment protocol.

7. **Add D-NEW-6 (Autonomy Level) to the decision map.** Support declaring
   autonomy levels that affect tool execution approval flows.

8. **Document the two-level routing paradigm** for D3A. Model selection and
   compute effort are separate axes.

### Lower Priority (Future-Proofing)

9. **Track MCP Tasks API** for async tool operations. The effects system
   should be able to model long-running tool calls.

10. **Track A2A protocol** (Google) as a complement to MCP for agent-to-agent
    delegation semantics.

11. **Consider graph memory** as a state backend option alongside the existing
    memory/fs backends.

12. **Add D-NEW-4 (Evaluation) and D-NEW-5 (Versioning)** to the decision map
    as lifecycle concerns.

---

## Corrected Decision Count

Previous (this document, initial version): 23 → 29 (+6 new). **Overcounted.**

After independence testing against real architectures:

| Proposed | Verdict | Action Taken |
|----------|---------|-------------|
| D-NEW-1: Structured Output | **Genuinely new** | Added as D3D (Output Shape) |
| D-NEW-2: Agent Identity & Trust | Decomposes into D2A + D4B + C1 | Expanded D4B and C1 option tables |
| D-NEW-3: Reasoning Strategy | Expansion of D3A | Added two-level routing row to D3A |
| D-NEW-4: Evaluation & Gating | New but out-of-scope (DevOps, not runtime) | Added to Open Questions |
| D-NEW-5: Versioning & Rollback | New but out-of-scope (DevOps, not runtime) | Added to Open Questions |
| D-NEW-6: Autonomy Levels | Composite of D4A + C5 | Added as named-configurations sidebar |

**Updated total: 24 decisions** (23 original + D3D: Output Shape).
Optionally 26 if the map's scope expands to include DevOps lifecycle (L6: Eval, L7: Versioning).

---

*Generated from 8 parallel deep research runs on 2026-03-03.*
*Source reports archived at `docs/architecture/research-2026-03/`.*
*Independence audit at `docs/architecture/decision-map-new-decision-audit.md`.*