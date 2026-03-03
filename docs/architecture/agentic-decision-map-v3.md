# The Agentic Decision Map

## How to Read This Document

This is a map of every engineering decision you face when building an agentic AI system, organized around three layers:

1. **The Turn** — the atomic unit of agency. One cycle of receive→reason→act. Every agent, in every pattern, processes turns. The decisions here are universal.
2. **Composition** — how turns relate to each other. Sequential, parallel, hierarchical, peer-to-peer, handoff, observation. This is where patterns diverge.
3. **Lifecycle** — what happens across turns. Memory, compaction, crash recovery, budget. The concerns that span the entire execution.

The research base: Anthropic's "Building Effective Agents" (Dec 2024), OpenAI's "Unrolling the Codex Agent Loop" (Jan 2026), Google Cloud's Agent Design Patterns (Oct 2025), AWS's "Agentic AI Patterns and Workflows" (2025), Braintrust's "Canonical Agent Architecture" (Aug 2025), ReAct (Yao et al. 2023), ACM TOSEM multi-agent survey (2025), Geoffrey Huntley's agent workshop (Oct 2025), plus implementation analysis of OpenClaw, NanoClaw, IronClaw, OpenHands, Goose, Aider, CrewAI, LangGraph, Claude Code, Sortie, and Amp.

The goal is to enumerate all the primitives, map existing solutions inside this framework, and provide a system you can use to fully analyze any agentic AI system — understand its strengths, weaknesses, and design your own.

---

## Architectural Flow

This diagram shows how data, context, and control actually flow through an agentic system at runtime. Decision points are marked `[Dnn]` where they sit on the flow. The three loops — inner (tool execution), outer (multi-turn), and lifecycle (cross-session) — are all visible.

```
═══════════════════════════════════════════════════════════════════════════
 TRIGGERS [D1]                         PERSISTENT STATE
═══════════════════════════════════════════════════════════════════════════

 User message ──┐                     ┌──────────────────────────┐
 Task assign ───┤                     │   MEMORY STORE           │
 Signal ────────┼──▶ INPUT            │                          │
 Schedule ──────┤                     │  hot: CLAUDE.md etc      │
 System event ──┘                     │  warm: files, git, API   │
       │                              │  cold: search index      │
       │                              │  structural: filesystem  │
       ▼                              │                          │
                                      │  [L1] write triggers:    │
                                      │   after tool / task /    │
                                      │   pre-compact / periodic │
                                      └─────┬──────────▲─────────┘
                                            │ read     │ write
                                            ▼          │
═══════════════════════════════════════════════════════════════════════════
 CONTEXT ASSEMBLY [D2]
═══════════════════════════════════════════════════════════════════════════

 ┌────────────────────────────────────────────────────────────────┐
 │                                                                │
 │  Identity [D2A]     History [D2B]     Memory [D2C]            │
 │  prompt ↔ struct    msgs ↔ stateless  hot ↔ warm ↔ cold      │
 │       │                  │                  │                  │
 │       └──────────────────┴──────────────────┘                  │
 │                          │                                     │
 │                          ▼                                     │
 │                   ┌─────────────┐     Tool Surface [D2D]      │
 │                   │  ASSEMBLED  │     schemas ↔ catalog ↔ map │
 │                   │  CONTEXT    │◀────────────┘               │
 │                   │             │                               │
 │                   │  [D2E] budget allocation:                  │
 │                   │  system ~10% | history ~67% | reserve ~22% │
 │                   └──────┬──────┘                               │
 │                          │                                     │
 └──────────────────────────┼─────────────────────────────────────┘
                            │
                            ▼
═══════════════════════════════════════════════════════════════════════════
 THE INNER LOOP (one agent session — the ReAct while-loop)
═══════════════════════════════════════════════════════════════════════════

      ┌─────────────────────────────────────────────────────────┐
      │                                                         │
      │         ┌──────────────────┐                            │
      │         │  INFERENCE [D3]  │                            │
      │         │                  │                            │
      │         │  model [D3A]     │                            │
      │         │  durability [D3B]│                            │
      │         │  retry [D3C]     │                            │
      │         └────────┬─────────┘                            │
      │                  │                                      │
      │                  ▼                                      │
      │         ┌──────────────────┐                            │
      │         │  MODEL RESPONSE  │                            │
      │         └───┬──────────┬───┘                            │
      │             │          │                                │
      │        tool calls    text                               │
      │             │          │                                │
      │             ▼          │                                │
      │    ┌────────────────┐  │                                │
      │    │ TOOL EXECUTION │  │                                │
      │    │                │  │                                │
      │    │ isolation [D4A]│  │                                │
      │    │ creds    [D4B] │  │                                │
      │    └───────┬────────┘  │                                │
      │            │           │                                │
      │            ▼           │                                │
      │    ┌────────────────┐  │                                │
      │    │ BACKFILL [D4C] │  │                                │
      │    │ results into   │  │                                │
      │    │ context        │  │                                │
      │    └───────┬────────┘  │                                │
      │            │           │                                │
      │            │    ┌──────┘                                │
      │   ┌────────┘    │                                       │
      │   │ (loop)      │                                       │
      │   │             ▼                                       │
      │   │    ┌──────────────────┐                             │
      │   │    │  EXIT CHECK [D5] │                             │
      │   │    │                  │                             │
      │   │    │  model-done?     │                             │
      │   │    │  max-turns?      │                             │
      │   │    │  budget?         │                             │
      │   │    │  circuit-break?  │                             │
      │   │    │  observer-halt?  │                             │
      │   │    └───┬──────────┬───┘                             │
      │   │        │          │                                 │
      │   │    CONTINUE      EXIT                               │
      │   │        │          │                                 │
      │   └────────┘          │                                 │
      │                       ▼                                 │
      │                  TURN OUTPUT                            │
      │                                                         │
      │  ┌────────────────────────────────────────────────────┐ │
      │  │ OBSERVER [C5] (concurrent — watches entire loop)   │ │
      │  │                                                    │ │
      │  │ oracle: called by agent as tool (pull, advisory)   │ │
      │  │ guardrail: runs at boundaries (can halt via trip)  │ │
      │  │ observer agent: continuous stream (can halt/inject) │ │
      │  └────────────────────────────────────────────────────┘ │
      └─────────────────────────────────────────────────────────┘
                            │
                            ▼
═══════════════════════════════════════════════════════════════════════════
 OUTPUT ROUTING — where does the turn's output go?
═══════════════════════════════════════════════════════════════════════════

                       TURN OUTPUT
                            │
              ┌─────────────┼──────────────┐
              │             │              │
              ▼             ▼              ▼
         TO USER      TO PARENT       TO NEXT AGENT
         (reply)      (C2: result     (C1: context
                       return)         transfer)
                            │              │
                            │              │
          ┌─────────────────┴──────────────┴──────────────┐
          │  COMPOSITION [C1-C5]                          │
          │                                               │
          │  context: full ↔ task-only ↔ isolated  [C1]   │
          │  results: inject ↔ summary ↔ two-path  [C2]   │
          │  lifecycle: ephemeral ↔ long-lived      [C3]   │
          │  comms: sync ↔ signals ↔ events         [C4]   │
          │  observation: none ↔ guardrail ↔ agent  [C5]   │
          │                                               │
          │  ┌─────────────────────────────────────┐      │
          │  │ CHILD AGENT                         │      │
          │  │ (runs its own complete inner loop    │      │
          │  │  with its own D1-D5 decisions)       │      │
          │  │                                     │      │
          │  │  input ──▶ context ──▶ reason ──┐   │      │
          │  │                          ▲      │   │      │
          │  │                          │  tools│   │      │
          │  │                          └──────┘   │      │
          │  │                     │               │      │
          │  │                  output             │      │
          │  └─────────────────────┼───────────────┘      │
          │                        │                      │
          │    flows back via [C2] │                      │
          └────────────────────────┼──────────────────────┘
                                   │
                                   ▼
═══════════════════════════════════════════════════════════════════════════
 LIFECYCLE (spans all turns — [L1-L5] cut across everything above)
═══════════════════════════════════════════════════════════════════════════

  After turn completes:
  │
  ├──▶ MEMORY WRITE [L1] ──▶ (writes to persistent state, top of diagram)
  │
  ├──▶ COMPACTION CHECK [L2]
  │      │
  │      ├── context ok ──▶ next turn (back to CONTEXT ASSEMBLY)
  │      │
  │      ├── context full ──▶ SUMMARIZE ──▶ flush critical state [L1]
  │      │                     │            then compress old turns
  │      │                     └──▶ next turn (with summary)
  │      │
  │      └── history full ──▶ CONTINUE-AS-NEW ──▶ carry state [L1]
  │                            │                   reset history
  │                            └──▶ next execution (fresh history)
  │
  ├──▶ CRASH? [L3]
  │      │
  │      ├── no durability ──▶ lost (restart from scratch)
  │      ├── checkpoint ──▶ resume from last snapshot
  │      ├── event replay ──▶ reconstruct from log (expensive)
  │      └── durable execution ──▶ replay workflow, skip cached (fast)
  │
  ├──▶ BUDGET CHECK [L4]
  │      │
  │      ├── under budget ──▶ continue
  │      └── over budget ──▶ halt (or downgrade model tier)
  │
  └──▶ OBSERVABLE VIA [L5]: logs / traces / event history / hooks / queries
```

**How to read this diagram**: Follow data from top to bottom for a single turn. The inner loop (INFERENCE → TOOL EXECUTION → BACKFILL → back to INFERENCE) runs multiple times within one turn. After a turn exits, output routes to the user, a parent agent, or a next agent. Lifecycle concerns (memory writes, compaction, crash recovery, budget) run after every turn and feed back to the top — persistent state written at the bottom is read during context assembly at the top. The observer watches the entire inner loop concurrently and can intervene at any point.

---

## Layer 1: The Turn

Every agent, regardless of what pattern it participates in, processes the same atomic cycle — visible as the inner loop in the architectural flow above. A turn is triggered by an input (user message, task assignment, signal from another agent, scheduled trigger, tool result from a previous turn). It produces an output (text response, tool call, delegation request, handoff, or nothing). The while-loop inside one turn is: assemble context → reason (model call) → respond → if tool calls, execute and loop back to reason; if text, exit.

This is the ReAct loop. It's the while-loop. It's Anthropic's "augmented LLM." It's what Braintrust calls the canonical agent architecture. It's what runs inside Claude Code, Codex, OpenClaw, and every other agent. There is nothing else.

The decisions below apply at each point in this cycle.

### Decision 1: What triggers the turn?

The input that starts a turn determines what context is available at the outset.

| Trigger Type | Examples | What's Available at Start |
|-------------|----------|--------------------------|
| **User message** | Chat input, CLI command, Slack message | Session state, user identity, conversation history |
| **Task assignment** | Coordinator dispatching work, CrewAI task handoff | Task description + whatever the assigner chose to include; parent's context is NOT available (by design) |
| **Signal from peer** | Temporal signal, shared-state mutation, filesystem write | Asynchronous; may arrive mid-turn, must be buffered or interrupt |
| **Tool result** | Return from previous tool execution | This is mid-turn, not a new turn — the while-loop continues |
| **Schedule** | Cron trigger, Temporal Schedule, calendar event | No conversational context; must bootstrap entirely from persistent state |
| **System event** | Webhook, file change watch, CI pipeline trigger | External context must be fetched; no user in the loop |

**Engineering consideration**: The trigger type *informs* your context assembly strategy — it doesn't constrain it. You can compose any assembly strategy with any trigger. A scheduled turn *could* reconstruct full conversation history from a database, and a user message turn *could* ignore conversation history and start fresh. But in practice, some combinations are natural (user message + full history) and some require extra work (schedule + full history requires persistence infrastructure). Designing for the hardest trigger (cold start from schedule) forces you to build robust memory and context reconstruction, which then makes every other trigger type work better.

### Decision 2: How is context assembled?

This is the step where the model's entire understanding of its situation is constructed. Everything the model knows comes from this assembled context. What you include shapes behavior; what you exclude creates blind spots.

Context assembly has five sub-decisions, each with its own spectrum:

#### 2A: Identity and behavioral specification

How does the agent know what it is and how to behave?

| Approach | Token Cost | Control | Fragility | Used By |
|----------|-----------|---------|-----------|---------|
| **Maximal prompt injection** — multi-section system prompt with conditional logic, 7+ bootstrap files loaded every turn | 3,500–20,000+ tokens fixed | Maximum | High — one bad edit changes all behavior | OpenClaw (11 sections, 7 bootstrap files) |
| **Markdown agent definition** — single .md file with YAML frontmatter for metadata, prose body for behavior | 500–2,000 tokens | Good | Medium — scoped to one agent | Claude Code (.claude/agents/*.md) |
| **Role string** — one-paragraph persona description | 100–500 tokens | Minimal | Low — simple to maintain | CrewAI, basic LangChain agents |
| **Environment-only** — no explicit behavioral prompt; behavior emerges from what tools and files the agent can see | ~0 tokens in prompt | Structural | Low — but behavior is implicit and harder to steer | NanoClaw (container filesystem = identity) |

**The spectrum**: Textual control (inject behavior via prompt) ↔ Structural control (constrain behavior via environment). OpenClaw is maximally textual. NanoClaw is maximally structural. Most systems sit somewhere between. The structural approach is more robust (harder to break with a bad prompt edit) but less expressive (can't easily tell the agent "be concise" by restricting its filesystem).

**A note on environment-only approaches**: Even with zero explicit behavioral prompt, the agent still needs *something* that causes it to read files, discover tools, and take action. This isn't magic — it comes from the harness. The agentic loop itself provides the initial prompt or instruction that starts the model reasoning. In NanoClaw, the Claude Agent SDK's built-in agentic behavior does this: the model inherently attempts to use its available tools to accomplish the task it receives. The "environment-only" label means the *agent-specific* behavioral specification is structural, not that there's zero prompt anywhere in the stack. The base model's training, the SDK's default system prompt, and the harness's initial message all contribute. What's absent is an *additional* agent-specific behavioral layer — the agent's identity comes from what it can see and do, not from what it's told to be.

#### 2B: Conversation history and state

What does the agent remember from previous interactions in this session?

| Approach | Persistence | Across Sessions? | Used By |
|----------|------------|-------------------|---------|
| **Full message array** — every user/assistant/tool message from session start | In-memory | No (lost on restart) | Most single-session agents, Claude Code, Codex |
| **Event-sourced log** — immutable event stream recording every action | Persistent | Yes | OpenHands |
| **Database-backed session** — conversation stored in PostgreSQL/SQLite | Persistent | Yes | IronClaw |
| **Workflow event history** — durable execution framework records every activity result | Persistent + replayable | Yes (survives crashes) | Temporal-based systems |
| **Stateless reconstruction** — full history sent with every API call, no server-side session | Stateless | Depends on caller | Codex (Responses API) |

**Engineering consideration**: Stateless reconstruction (Codex's approach) is the simplest and most resilient — every call is self-contained, so there's no session state to corrupt, lose, or migrate. The cost is network transfer (sending full history every call). Prompt caching makes this viable — cached prefixes eliminate the redundant compute cost. This is the pattern converging toward industry standard for single-agent systems. Multi-agent systems need something richer (durable event history, event-sourced logs) because state spans multiple agents.

#### 2C: Memory (persistent knowledge across sessions)

How does the agent access information from previous sessions, other agents, or external knowledge?

| Layer | Always Loaded? | Retrieval | Used By |
|-------|---------------|-----------|---------|
| **Hot memory** — curated state always in context (identity, goals, key facts) | Yes | None needed | OpenClaw (MEMORY.md), Claude Code (CLAUDE.md) |
| **Warm memory** — loaded on-demand within a session via tool call | Via explicit request | Keyword, path-based | OpenClaw (daily files), Aider (added files) |
| **Cold memory** — retrieved across sessions via search | Via search | BM25, vector, hybrid | OpenClaw (hybrid), IronClaw (FTS+pgvector), CrewAI (ChromaDB) |
| **Structural memory** — implicit in environment state | Implicit | Navigation | NanoClaw (filesystem), Aider (git repo), OpenHands (event stream) |

**The critical pattern**: Hot memory is a tax on every turn. Cold memory is free until you need it, then expensive to retrieve (extra inference step or tool call). The architectural decision is: what must every turn know (put it in hot), what might some turns need (make it warm/cold), and what can be inferred from the environment (structural)?

**How structural memory actually works**: The agent doesn't magically know what's in its filesystem or git repo. The harness provides the agent with tools (file read, directory list, grep, git log) and the model uses them to discover its environment. The "structural" part means the information exists in the environment rather than being pre-loaded into context — but *discovery requires action*. This is why structural memory has higher latency than hot memory: the agent must spend reasoning steps and tool calls to find what it needs. Some systems reduce this cost by injecting a lightweight index into context — Aider's repo map is exactly this: a compressed structural summary (tree-sitter-generated function/class signatures, ~1K tokens) that tells the model what exists so it can request specific files efficiently.

Every system with compaction implements a **pre-compaction memory flush** — before destroying old conversation turns, the agent silently writes important state to persistent storage. This is OS virtual memory's "dirty page writeback" applied to cognition.

#### 2D: Tool and capability surface

How does the agent know what it can do?

| Approach | Overhead Per Tool | Scalability | Used By |
|----------|------------------|-------------|---------|
| **Static schema injection** — all tool JSON schemas included in every turn | ~50-200 tokens/tool | Poor beyond ~20 tools | Most systems (default) |
| **Per-agent scoping** — each agent only sees tools it's allowed to use | Same per-tool, fewer tools | Better — scope reduces noise | Claude Code (sub-agents) |
| **Lazy catalog** — tool names/descriptions in context, full schema loaded on demand | ~20 tokens/tool in catalog | Good — scales to hundreds | Claude Code (skills), Anthropic Tool Search Tool |
| **Compressed structural map** — environment summary (function signatures, file tree) injected instead of tool schemas | ~1K tokens for entire codebase | Excellent | Aider (tree-sitter repo map) |
| **Extension-driven** — tools are MCP extensions, user configures which are active | Variable | Good — modular | Goose, MCP-based systems |

**Key data point** (from Braintrust): Tool definitions account for 10.7% of total tokens in a typical agent conversation. Tool *responses* account for 67.6%. Combined, tools comprise ~80% of what the model processes.

**What this means and doesn't mean**: This data establishes that tool output formatting deserves significant engineering attention — poorly formatted tool responses pollute the majority of the model's context. But it does *not* mean tool design is more important than system prompt engineering in absolute terms. The system prompt defines the agent's identity, goals, reasoning patterns, and — critically — how it interprets and uses tool outputs. A well-crafted system prompt shapes how the model consumes the other 80%. The relationship is compositional: the system prompt is the lens through which tool outputs are interpreted. Both matter; optimizing only one is leaving performance on the table.

**The practical takeaway**: Treat tool output formatting as a first-class engineering concern (most teams under-invest here), but don't conclude that system prompt engineering is less impactful. The highest-performing systems invest heavily in both.

**Open research area**: Aider's repo map approach — compressing structural information into a relevance-ranked index — suggests a general pattern for capability surface design that hasn't been widely explored. How do you optimally summarize what an agent can do or access in minimal tokens? Tree-sitter works for code. What's the equivalent for API surfaces, database schemas, document collections? This is an active area worth investigating.

#### 2E: Context budget allocation

How do you divide the finite context window across competing demands?

Reference allocation (Claude Code, Opus 200K window):
```
System prompt + tool schemas:  ~10%   (20K tokens)
Compaction reserve:            ~22.5% (45K tokens)
Conversation + tool results:   ~67.5% (135K tokens)
```

Huntley's principle: "The more you allocate to a context window, the worse the performance of the context window will be, and your outcomes will deteriorate." This applies to MCP tool schemas especially — it's easy to fall into allocating 76K+ tokens just for tool definitions, leaving only 100K usable. Less is more.

**Engineering consideration**: The compaction reserve is critical and non-obvious. If you fill the context to 100% before compacting, you have no room to run the compaction inference itself. Claude Code reserves ~22.5% so there's always room to summarize before hitting the wall. Systems without a compaction reserve (or without compaction at all) simply crash or truncate when context fills — Aider solves this by starting a new session, losing conversation state.

### Decision 3: How does inference happen?

The model is called. This seems simple but has significant engineering decisions.

#### 3A: Model selection

| Approach | Cost Efficiency | Quality | Used By |
|----------|----------------|---------|---------|
| **Single model for everything** | Low (overpay for easy tasks) | Consistent | NanoClaw, Goose, most simple agents |
| **Two-tier: architect/editor** — strong model plans edits, fast model applies them | Good | Good — each model does what it's best at | Aider |
| **Three-tier** — match model capability to task difficulty | Best | Best — when routing is correct | Claude Code (sub-agents) |
| **Difficulty-aware routing** — classify task difficulty, route to appropriate model tier | Optimal (64% cost at SOTA quality per DAAO paper) | Highest | Academic (DAAO) |
| **Two-level routing** — select model AND reasoning effort per task (e.g., same model with low vs. high thinking budget) | Best — fine-grained cost/quality control | Best — when calibrated | Claude Code (Sonnet + thinking budget), Cursor, o1/o3 reasoning effort parameter |
| **Speculative parallel** — send same request to multiple models, use first good result | Expensive (multiple calls) | Highest (best of N) | Safety-critical pipelines, consensus/verification systems |

#### 3B: Inference durability

What happens if the process crashes mid-inference?

| Approach | Token Waste on Crash | Recovery | Used By |
|----------|---------------------|----------|---------|
| **None** — crash = lost work, restart from scratch | Full session cost | Manual restart | OpenClaw, NanoClaw, Claude Code, Goose, Aider |
| **Checkpoint** — save state before each call, resume from checkpoint | Since last checkpoint | Automatic from checkpoint | IronClaw (PostgreSQL) |
| **Event replay** — replay immutable event log to reconstruct state | Zero (events are permanent) | Automatic but expensive (replay cost) | OpenHands |
| **Durable execution** — framework replays workflow from event history, activities skip (cached results) | Zero (activity results cached) | Automatic, exact state restoration | Temporal-based systems |

**Engineering consideration**: For short-lived single-agent tasks (answer a question, write a function), durability doesn't matter — the cost of restarting is low. For long-running multi-agent tasks (500+ iterations, multi-hour research), durability is essential — a crash at iteration 400 without recovery means significant wasted tokens and hours of lost work.

#### 3C: Timeout and retry strategy

LLM inference can take 30 seconds to 5+ minutes for complex reasoning.

| Concern | Consideration |
|---------|--------------|
| **Timeout** | Must be longer than maximum expected inference time. Extended thinking can take 3-5 minutes. Default timeouts (30s, 60s) are often too short. |
| **Retry** | Rate limit errors (429) need exponential backoff with Retry-After header respect. Budget-exceeded errors should NOT retry. Safety refusals should NOT retry. |
| **Heartbeat** | For long-running inference inside a durable framework, the activity must heartbeat periodically to prove it's alive. Otherwise the framework assumes it's dead and retries, doubling cost. |
| **Conflict** | If both the SDK and the orchestrator have retry logic, they will conflict. Disable one. Use a single retry authority. |

#### 3D: Output shape enforcement

How does the system constrain the shape of the model's output?

| Approach | Guarantee | Token Overhead | Used By |
|----------|-----------|---------------|---------|
| **No enforcement** — model outputs freeform text or tool_use | None — hope the model complies | Zero | Claude Code, most simple agents |
| **JSON mode** — model instructed to produce JSON, no schema validation | Soft — usually valid JSON, no shape guarantee | Minimal | OpenAI (response_format: json) |
| **Strict JSON Schema decoding** — constrained decoding guarantees output matches schema | Hard — 100% schema adherence at generation time | Zero runtime (constraint applied during decoding) | OpenAI Structured Outputs, Anthropic tool_use |
| **Application-level parsing with retry** — custom parser validates output, retries on failure | Medium — depends on retry budget | Retry cost on parse failure | Aider (edit format parser), CrewAI (Pydantic validation) |
| **Schema propagation across agent graph** — parent defines output contract, child must conform | Contract — compositional type safety | Schema validation overhead | Multi-agent pipelines with typed interfaces |

**Why this is a separate decision**: Model selection (D3A) determines *which* model. Backfill (D4C) determines how tool *results* re-enter context. Output shape determines how the model's own *responses* are constrained. These are independently variable — you can use the same model with or without schema enforcement, and the same backfill strategy regardless of output constraints.

**The compositional argument**: In multi-agent systems, a child agent returning malformed output breaks the parent's reasoning. Output shape enforcement is a type system for agent pipelines. Without it, composition is fragile — it works when the model cooperates, breaks when it doesn't.

**Engineering consideration**: Strict schema decoding (when the provider supports it) is strictly superior to application-level parsing for structured output — zero token overhead, 100% conformance, no retry cost. The tradeoff is expressiveness: you can only enforce shapes the provider's schema language can represent. Complex output patterns (interleaved reasoning + structured data) may still need application-level parsing.

### Decision 4: How is the model's response handled?

The model returns either text (done reasoning) or tool calls (needs to act). This branch is the core of the while-loop.

#### 4A: Tool execution isolation

How much do you trust the code/commands the model generates?

| Approach | Boundary | What's Isolated | Performance Cost | Used By |
|----------|----------|----------------|-----------------|---------|
| **No isolation** — execute in host process | None | Nothing | Zero | Aider, Goose, CrewAI, most frameworks |
| **Permission gate** — ask user before dangerous operations | Human judgment | Nothing (but user approves) | Latency (human wait) | Claude Code |
| **Application sandbox** — allowlists, policy chains | Process-level | Network, filesystem (configurable) | Minimal | OpenClaw |
| **Container** — each agent in its own container | Container boundary | Filesystem, network | Container startup, IPC | NanoClaw, OpenHands |
| **WASM sandbox** — capability-based, no host access | WASM boundary | Everything | WASM overhead, limited syscalls | IronClaw |
| **Multi-layer** — gVisor/Kata + network policy + k8s + credential sidecar | 4 independent boundaries | Kernel, network, resources, credentials | ~5-10% gVisor syscall overhead | K8s Agent Sandbox pattern |

**The spectrum**: Trust ↔ Isolation. More trust = less overhead, more capability, more risk. More isolation = more safety, more complexity, more latency. The right choice depends on threat model: personal assistant on your laptop (trust is fine) vs production system executing code from untrusted user inputs (multi-layer isolation is warranted).

#### 4B: Credential handling during tool execution

Does the agent ever see API tokens, passwords, or secrets?

| Approach | Agent Sees Credentials? | Leak Vector | Used By |
|----------|----------------------|-------------|---------|
| **In-process environment variables** | Yes | Output, logs, tool calls | OpenClaw (default), Goose, Aider, CrewAI |
| **Mounted files in container** | Only mounted ones | Same, but scoped by mount | NanoClaw |
| **Boundary injection with leak detection** | No — injected at sandbox edge, scanned both directions | Minimal — active detection | IronClaw (WASM boundary) |
| **Sidecar injection** — sidecar holds credentials, agent calls tools via sidecar, sidecar adds auth | No — agent never touches tokens | Minimal — network-level separation | MCP sidecar pattern |
| **Proxy-mediated access** — agent makes requests through a proxy that injects credentials and enforces policy | No — proxy adds auth headers | Proxy misconfiguration | Claude Code (git credential proxy) |
| **Workload identity (SPIFFE/SPIRE)** — agent has its own cryptographic identity, requests scoped tokens at runtime | Ephemeral scoped tokens only | Token scope misconfiguration | Enterprise Kubernetes deployments, MCP Nov 2025 spec (OAuth 2.1) |
| **LLM-assisted risk scoring** — credential access gated by dynamic risk assessment of each tool call | Conditionally — depends on risk score | Risk model miscalibration | OpenHands (SecurityAnalyzer + ConfirmationPolicy) |

**Agent identity vs. credential handling**: This decision covers whether the agent *sees* secrets. The related question of the agent's *own* identity — how it authenticates to external services, how permissions propagate in agent-to-agent delegation — overlaps with D2A (behavioral identity) and C1 (child context). In enterprise multi-agent systems, the agent's workload identity determines what credentials it can request, making identity a prerequisite for credential handling.

#### 4C: Result integration (backfill)

How do tool results re-enter the context for the next reasoning step?

| Approach | Token Cost | Safety | Used By |
|----------|-----------|--------|---------|
| **Raw injection** — complete tool output inserted as-is | Full output size | Low — potential context pollution, injection risk | Most systems (default) |
| **Formatted injection** — tool output reformatted for readability before insertion | Same or less | Medium — cleaner for model reasoning | Braintrust recommendation |
| **Security-stripped** — sensitive fields removed before insertion | Less | Higher — prevents accidental exposure | OpenClaw (strips toolResult.details) |
| **Safety-sanitized** — content escaped/wrapped, prompt injection detected | Less + processing overhead | Highest | IronClaw (content escaping + injection detection) |

**Key insight** (from Braintrust): Tool responses account for 67.6% of all tokens in a typical agent conversation. Since tool outputs constitute the majority of what the model reasons over on subsequent turns, formatting them well — concise, relevant, structured for the model's consumption — is a high-leverage optimization that most teams under-invest in. Treat tool output design as carefully as you'd treat prompt design: the tool output *is* the prompt for the next reasoning step.

### Decision 5: What ends the turn?

After the model responds (text or tool call results have been processed), something must decide: continue the while-loop, or exit?

| Mechanism | How It Works | Risk | Used By |
|-----------|-------------|------|---------|
| **Model signals done** — model returns text (no tool calls), the turn ends | Implicit — absence of tool calls = done | Model may stop too early or loop unnecessarily | Every system (basic while-loop exit) |
| **Max turns limit** — hard cap on reasoning steps within one turn | Counter | May cut off before task is complete | Claude Code, most production systems |
| **Budget limit** — stop when token/dollar cost exceeds threshold | Cost tracking | May leave task incomplete | Claude Code (max_budget_usd) |
| **Goal evaluation** — assess whether objective is met | Extra inference step | Expensive (requires an evaluation call) | CrewAI (task output validation) |
| **Circuit breaker** — stop after N consecutive failures | Error counter | May give up too early on hard tasks | Codex (parse failure fallback) |
| **Timeout** — wall-clock time limit | Timer | Blunt instrument, doesn't account for progress | Common in production deployments |
| **Observer halt** — external process watching execution decides to stop it | Parallel evaluation | Observer may be wrong; adds latency/cost | OpenAI Agents SDK (guardrail tripwire), AWS observer pattern |
| **LLM self-assessment** — dedicated reasoning pass: "is the goal achieved?" with evidence evaluation | Extra inference step | May self-deceive; expensive | Devin (task completion assessment) |
| **Programmatic verification** — machine-verifiable checks (file exists, tests pass, API returns 200) | Deterministic check | Narrow — only checks what's codified | CI-gated agents, SWE-bench harnesses |
| **Loop/stuck detection** — semantic similarity of recent outputs, repeated tool call patterns | Similarity threshold | May false-positive on legitimately iterative work | Emerging best practice |
| **Infeasibility recognition** — model determines task is impossible and exits with explanation | Model judgment | May give up too early on hard problems | Research systems |

**Engineering consideration**: Production systems should layer multiple independent stop conditions. A single condition creates failure modes — "model signals done" alone lets the model loop forever if it's confused; "max turns" alone cuts off genuinely hard tasks. The combination of model-done + max-turns + budget + circuit-breaker provides defense-in-depth for termination.

---

## Layer 2: Composition

A single turn, running in a while-loop, is a complete agent. But many problems require multiple agents working together. Composition is how turns from different agents relate to each other.

### The Six Composition Primitives

These are the atomic operations. Every multi-agent pattern is a combination of these:

```
CHAIN:      A ──▶ B ──▶ C              (output of A feeds B feeds C)

FAN-OUT:    A ──┬▶ B                   (A's output feeds B, C, D
                ├▶ C                    simultaneously)
                └▶ D

FAN-IN:     B ──┐
            C ──┼──▶ A                 (B, C, D's outputs merge into A)
            D ──┘

DELEGATE:   A ──▶ [B runs] ──▶ A      (A spawns B, waits for result,
                                        continues with B's output)

HANDOFF:    A ──▶ B                    (A transfers control to B,
                                        A terminates, B inherits
                                        conversation state)

OBSERVE:    ┌─── O ───┐               (O watches A's execution
            │  watches │                concurrently; can read
            │  ↓    ↑  │                context, inject signals,
            │  A runs  │                modify state, halt, or
            └──────────┘                redirect)
```

The first five follow a simple rule: **one agent's output becomes another agent's input.** The sixth — OBSERVE — breaks this rule. The observer doesn't wait for output; it watches the *process* concurrently and may intervene at any point.

### The Observer Primitive: Three Manifestations

The OBSERVE primitive appears in three distinct forms across the ecosystem, each with different capabilities:

| Manifestation | Can Read? | Can Halt? | Can Inject/Modify? | Latency | Used By |
|--------------|-----------|-----------|-------------------|---------|---------|
| **Oracle** — a separate LLM registered as a tool that the doing-agent calls for higher-order reasoning | Only when called | No (advisory only) | Returns guidance that the agent may or may not follow | Per-call | Amp (oracle tool) |
| **Guardrail** — validation logic that runs in parallel with agent execution, can trigger tripwires to halt | Input and/or output | Yes (tripwire halts execution) | Can rewrite input (pre-flight), block output | Parallel with agent | OpenAI Agents SDK (input/output/tool guardrails) |
| **Observer agent** — a separate agent that continuously monitors telemetry, logs, and agent state | Continuous (telemetry stream) | Yes (trigger escalation or kill) | Yes (modify context, reassign goals, inject signals) | Continuous/async | AWS observer pattern, security monitoring agents |

**Key architectural distinction**: The oracle is *pull-based* — the doing-agent decides when to consult it. The guardrail is *checkpoint-based* — it runs at defined points (input, output, tool use). The observer agent is *continuous* — it watches the stream and can intervene at any time.

**Why this matters**: The observer primitive enables capabilities that no other primitive provides: continuous course-correction (an observer watching for drift and adjusting the agent's goals mid-execution), security monitoring (an observer scanning for prompt injection or unauthorized data access in real-time), quality assurance (an observer evaluating intermediate outputs and requesting revisions before they propagate), and budget governance (an observer tracking aggregate cost across agents and halting when limits approach). These are all cases where waiting for the agent's output is too late — you need concurrent visibility into the process.

**The compositional question**: Does OBSERVE compose with the other five primitives? Yes — an observer can watch any pattern. You can observe a chain, observe a fan-out, observe an orchestrator-workers setup. The observer is orthogonal to the topology of the agents it watches. This makes it a true cross-cutting concern, similar to lifecycle concerns (L1-L5) but operating at the composition layer.

### Composition Patterns (Built from Primitives)

| Pattern | Primitives Used | Topology | Example |
|---------|----------------|----------|---------|
| **Prompt chain** | CHAIN | Linear | Generate outline → validate → write document |
| **Router** | CHAIN (conditional) | Fork based on classification | Route billing questions to billing agent, tech to support |
| **Parallel workers** | FAN-OUT + FAN-IN | Star | Run security review, code review, style check simultaneously |
| **Orchestrator-workers** | DELEGATE (repeated in loop) | Hierarchical | Coordinator plans, dispatches workers, reviews results, replans |
| **Evaluator-optimizer** | DELEGATE (in feedback loop) | Cyclic pair | Generator produces output, evaluator critiques, generator revises |
| **Team** | FAN-OUT + signals/shared state | Mesh | Long-lived agents communicate via signals or shared memory |
| **Peer handoff** | HANDOFF | Sequential transfer | Triage agent → billing agent → escalation agent |
| **Pipeline** | CHAIN + DELEGATE | Linear with expansion | Each stage may internally delegate to sub-agents |
| **Governed execution** | Any pattern + OBSERVE | Any + observer | Any of the above with guardrails, monitoring, or oracle consultation |

### Composition Decision Points

These decisions arise whenever you compose multiple agents, regardless of which pattern you're using:

#### C1: What context does the child/next agent receive?

This is the most consequential composition decision. The parent/previous agent has rich context. The child/next agent needs focused context.

| Approach | What Transfers | What's Stripped | Used By |
|----------|---------------|----------------|---------|
| **Full context inheritance** — child sees everything parent sees | All messages, all tool results, all state | Nothing | LangGraph (graph state accumulates) |
| **Task-only injection** — child gets a task description and relevant data, nothing else | Task prompt + selected data | Parent's conversation, tool history, identity | Claude Code (sub-agents), CrewAI |
| **Summary injection** — child gets a summary of parent's state | Summary of relevant context | Raw conversation, tool outputs, intermediate reasoning | Anthropic multi-agent research recommendation |
| **Structural isolation** — child only sees what's in its own filesystem/container | Only mounted/provisioned resources | Everything not explicitly provided | NanoClaw (container) |
| **Conversation inheritance** — child gets the full conversation but fresh tool/capability surface | Conversation history | Parent's tools, parent's identity/instructions | OpenAI Agents SDK (handoff) |

**The tradeoff**: Too much context → child wastes tokens on irrelevant information, risks confusion, may violate security boundaries. Too little context → child lacks information to do its job, produces poor results, asks questions it can't get answers to.

**The key question**: Is the child's context boundary enforced by **prompt** (you tell the LLM to ignore parent context — fragile, the model can choose to attend to it anyway) or by **infrastructure** (the child literally cannot see parent context because it's in a separate process/container/pod — robust, but requires IPC for any communication)?

**Trust and permission propagation**: When a parent delegates to a child, should the child inherit the parent's permissions? This is the agent equivalent of privilege delegation in distributed systems.

| Trust Model | Permissions | Risk | Used By |
|------------|------------|------|---------|
| **Full delegation** — child inherits parent's credentials and permissions | Same as parent | Over-privileged child | Simple delegation (most systems) |
| **Scoped delegation** — parent explicitly grants a subset of its permissions | Restricted to task | Scope may be too narrow or too broad | OAuth 2.1 scoped tokens, MCP capability grants |
| **Independent identity** — child has its own credentials, no inheritance from parent | Own permissions only | Child may lack needed access | Container-isolated agents (NanoClaw) |
| **Dynamic risk-gated** — child's permission level adjusts based on risk assessment of each action | Adaptive | Risk model failure | OpenHands (SecurityAnalyzer) |

**The key insight**: Permission propagation in agent composition is analogous to capability-based security in operating systems. The safest model is scoped delegation (grant minimum necessary), but most current systems default to full delegation (child acts as parent) because it's simpler.

#### C2: How do results flow back?

| Approach | What Returns | Token Cost to Parent | Audit Trail | Used By |
|----------|-------------|---------------------|------------|---------|
| **Direct injection** — child's full output inserted into parent's context | Complete output | Full output size | Only in parent's context | OpenClaw, basic LangChain |
| **Summary only** — child's output summarized before returning to parent | Condensed summary | Much smaller | Summary only (unless logged separately) | Claude Code (sub-agents) |
| **Two-path: storage + summary** — full output to persistent storage, summary to parent | Summary to context, full output to storage | Small (summary) | Full (in storage) | Systems with persistent memory layer |
| **Shared state mutation** — child mutates shared state, parent reads it | State diff | Depends on read pattern | In state history | LangGraph (graph state), CrewAI (shared memory) |
| **Signal/event** — child sends an asynchronous message, parent receives when ready | Message payload | Message size | In event log | Temporal signals, event-driven systems |

#### C3: What is the child's lifecycle?

| Approach | Duration | Can Receive Follow-ups? | Resource Cost | Used By |
|----------|----------|------------------------|---------------|---------|
| **Fire-and-forget** — child runs, returns result, terminates | One task | No | Minimal (ephemeral) | Claude Code (sub-agents) |
| **Long-lived** — child stays running, accepts new tasks via signals | Multiple tasks | Yes | Ongoing (process/container stays alive) | CrewAI (persistent agents) |
| **Task-scoped** — child lives for the duration of one task, then terminates | One task | No (but may be re-created for next task) | Moderate | Most orchestration frameworks |
| **Conversation-scoped** — child inherits and continues the conversation | Rest of conversation | Yes (IS the conversation now) | Same as parent (replaced it) | OpenAI Agents SDK (handoff) |

#### C4: How do agents communicate?

| Mechanism | Latency | Durability | Ordering Guarantee | Used By |
|-----------|---------|------------|-------------------|---------|
| **Function call/return** — synchronous, parent calls child as a function | Lowest | None | Implicit (call order) | Claude Code, basic delegation |
| **Shared filesystem** — agents read/write files, poll for changes | Medium | Persistent (filesystem) | None (poll-based) | NanoClaw (JSON IPC), Aider (git) |
| **Shared state object** — agents mutate and read from a common data structure | Low | In-memory (lost on crash) | Depends on implementation | LangGraph (graph state), CrewAI |
| **Signals** — fire-and-forget async messages via framework | Low-medium | Durable (framework persists) | Delivery order, not processing order | Temporal signals |
| **Database/queue** — agents communicate via persistent store | Medium | Persistent | Depends on implementation | IronClaw (PostgreSQL), enterprise patterns |
| **Event stream** — immutable, append-only log all agents can read | Medium | Persistent, replayable | Total order | OpenHands (event stream) |

#### C5: Who can observe/intervene in execution?

| Approach | Visibility | Intervention Power | Cost | Used By |
|----------|-----------|-------------------|------|---------|
| **No observer** — agents run autonomously | None | None | Zero | Most simple agent setups |
| **Human-in-the-loop** — human reviews at checkpoints | At checkpoints | Full (approve/reject/redirect) | Human time | Claude Code (permission gate) |
| **Oracle tool** — agent can call a reasoning-specialist LLM | When agent decides to call | Advisory only | Per-call inference cost | Amp |
| **Parallel guardrails** — validation runs alongside agent, can halt | Input/output/tool boundaries | Halt via tripwire | Inference cost per guardrail | OpenAI Agents SDK |
| **Continuous observer** — separate agent monitors telemetry stream | Continuous | Halt, redirect, inject, modify | Ongoing inference cost | AWS observer agent, security monitors |

#### Named Autonomy Configurations (D4A + C5 combinations)

The combination of isolation (D4A) and observation (C5) choices produces recognizable autonomy postures. These are not a separate decision — they're named configurations of two existing decisions:

| Autonomy Level | D4A (Isolation) | C5 (Observation) | Example |
|---------------|----------------|------------------|---------|
| **Read-only** | No execution allowed | Human approves everything | Monitoring/analysis agents |
| **Propose** | No execution | Human executes suggestions | Conservative enterprise deployments |
| **Execute with approval** | Permission gate | Human-in-the-loop | Claude Code (default) |
| **Autonomous with guardrails** | Container/sandbox | Parallel guardrails | OpenAI Agents SDK, Devin (within sandbox) |
| **Dynamic autonomy** | Risk-scored gating | Auto-approve low-risk, pause high-risk | OpenHands (SecurityAnalyzer) |
| **Fully autonomous** | Any/none | No observer | Aider (within session), batch processing agents |

**The insight**: You don't make an "autonomy level" decision separately from D4A and C5. Your D4A and C5 choices *determine* your autonomy level. This taxonomy is useful for communication ("we run at execute-with-approval level") but the actual engineering decisions remain D4A and C5.

---

## Layer 3: Lifecycle

These concerns span multiple turns and multiple agents. They're not tied to any specific point in the turn cycle — they cut across everything.

### L1: Memory Persistence — When and What to Write

Memory is not just a context assembly concern (read). It's equally a lifecycle concern (write). The question is: when does the agent commit state to persistent storage, and what does it write?

| Write Trigger | What's Written | Why Here | Used By |
|---------------|---------------|----------|---------|
| **After every tool use** | Tool result + any state changes | Maximum durability, audit trail | OpenHands (event stream appends every action) |
| **After task completion** | Task result summary | Natural checkpoint | CrewAI (long-term memory update) |
| **Before compaction** | Critical state that would be lost when old turns are summarized | The "dirty page writeback" — flush before eviction | OpenClaw (memory flush), Claude Code (CLAUDE.md + todolist + plan) |
| **Before continue-as-new** | Workflow state that carries forward | Durable execution framework resets event history — state must be explicitly carried | Temporal-based systems |
| **On agent termination** | Final output + any learnings | Capture work product before context window is destroyed | Claude Code (sub-agent returns) |
| **Periodically** | Accumulated state changes | Balance durability vs write overhead | OpenClaw (MEMORY.md updates) |

### L2: Compaction — Managing the Finite Context Window

Every system with a context window faces this: eventually, the conversation grows too large. What happens?

| Strategy | How It Works | What Survives | Cost | Used By |
|----------|-------------|--------------|------|---------|
| **LLM summarization** — old turns are compressed into a summary by the model | LLM generates a summary of older conversation, replaces raw turns | Summary + bootstrap files (re-injected) | One inference call to summarize | OpenClaw, Claude Code, Goose |
| **Start fresh** — when context fills, start a new session | User starts a new chat; git/filesystem state persists | Structural memory (git, files) only | Zero (but loses conversation context) | Aider |
| **Truncation** — drop oldest messages | Remove messages from the front of the history | Recent messages only | Zero | Simple implementations |
| **Continue-as-new** — framework resets execution history, carries forward explicit state | New workflow execution with carryover state | Explicitly carried state + persistent storage | Framework overhead only (no LLM call) | Temporal-based systems |
| **Sliding window + RAG** — keep recent messages, make older ones searchable | Recent in context, old retrievable via search | Everything (in different tiers) | Search/embedding overhead | IronClaw, some CrewAI configurations |

**The pre-compaction flush** (critical pattern): Before compaction destroys information, write important state to persistent storage. Then compact. Then continue with the summary + persistent state available via memory tools. This is the single most important lifecycle mechanism for long-running agents.

### L3: Crash Recovery

What happens when the process dies, the container crashes, or the network fails?

| Strategy | Recovery | Token Waste | Time Waste | Used By |
|----------|----------|------------|------------|---------|
| **None** — restart from scratch | Manual | All tokens since session start | All time since start | OpenClaw, NanoClaw, Claude Code, Goose, Aider |
| **Session checkpoint** — periodic state snapshots to database | Resume from last checkpoint | Tokens since last checkpoint | Time since last checkpoint | IronClaw |
| **Event replay** — replay immutable event log | Reconstruct pre-crash state | Zero (events are permanent) | Replay time (can be significant) | OpenHands |
| **Durable execution replay** — framework replays workflow, activities skip via cached results | Exact pre-crash state restored | Zero (activity results cached) | Near-instant (no re-execution) | Temporal-based systems |

### L4: Budget and Cost Control

How do you prevent a runaway agent from burning $500 in API calls?

| Mechanism | Granularity | Enforcement Point | Used By |
|-----------|------------|-------------------|---------|
| **Per-turn token limit** | Individual LLM call | Model API parameter (max_tokens) | Universal |
| **Per-session cost cap** | Entire agent session | SDK parameter or hook | Claude Code (max_budget_usd) |
| **Per-workflow budget** | All agents in a workflow | Orchestrator tracks aggregate cost | Workflow-based systems |
| **Model tier routing** | Per-task | Routing logic assigns cheaper models to easier tasks | Claude Code (sub-agents use Haiku), Aider (architect mode) |
| **Circuit breaker** | Error rate | Stop after N consecutive failures | Codex (parse failure fallback) |

### L5: Observability

What can you see about what the agent is doing?

| Approach | Granularity | Overhead | Used By |
|----------|-----------|----------|---------|
| **Logging** — agent writes to log files/stdout | Whatever you log | Minimal | Universal |
| **Tracing** — structured trace of every LLM call, tool call, decision | Per-operation | Moderate | OpenAI Agents SDK (built-in tracing), LangSmith |
| **Event history** — every operation recorded in durable event log | Per-operation, persistent | Moderate | Temporal event history, OpenHands (event stream) |
| **Hooks** — infrastructure callbacks at agent lifecycle points | Per-hook | Minimal per hook | Claude Code SDK (PreToolUse, PostToolUse, Stop) |
| **Dashboard queries** — read-only queries against live agent state | On-demand | Zero (query-time only) | Temporal queries, CrewAI (monitoring) |

---

## The Decision Topology

Here's how the decisions relate to each other — which choices inform or constrain which other choices.

```
TRIGGER TYPE (D1)
    │
    ├─── informs ──▶ CONTEXT ASSEMBLY (D2)
    │                    │
    │                    ├── IDENTITY (2A)
    │                    ├── HISTORY (2B) ◀── constrained by ── COMPACTION (L2)
    │                    ├── MEMORY (2C) ◀─── constrained by ── MEMORY WRITES (L1)
    │                    ├── TOOLS (2D)
    │                    └── BUDGET (2E)
    │
    └─── determines ──▶ what context is available at turn start
    
INFERENCE (D3)
    ├── MODEL SELECTION (3A) ◀── constrained by ── BUDGET (L4)
    ├── DURABILITY (3B) ◀─────── determines ──▶ CRASH RECOVERY (L3)
    ├── RETRY (3C) ◀──────────── constrained by ── BUDGET (L4)
    └── OUTPUT SHAPE (3D) ◀──── constrains ──▶ COMPOSITION (C1, C2)

RESPONSE HANDLING (D4)
    ├── ISOLATION (4A) ◀──── constrains ──▶ CREDENTIAL HANDLING (4B)
    ├── CREDENTIALS (4B)
    └── BACKFILL (4C) ◀──── impacts ──▶ CONTEXT BUDGET (2E)

TURN EXIT (D5)
    ├── constrained by ── BUDGET (L4)
    ├── constrained by ── COMPACTION need (L2)
    ├── may trigger ── MEMORY WRITE (L1)
    └── may be triggered by ── OBSERVER (C5)

COMPOSITION (C1-C5)
    ├── CHILD CONTEXT (C1) ◀── informed by ── ISOLATION (4A)
    ├── RESULT RETURN (C2) ◀── impacts ──▶ CONTEXT BUDGET (2E)
    ├── LIFECYCLE (C3) ◀──── constrained by ── RESOURCE BUDGET
    ├── COMMUNICATION (C4) ◀── constrained by ── DURABILITY choice (3B)
    └── OBSERVATION (C5) ◀── independent (orthogonal to all patterns)
```

**The two most consequential decisions** (most other decisions flow from these):

1. **Durability model** (D3B / L3): If you choose "no durability" (most systems), you get simplicity but lose crash recovery and observability. If you choose durable execution, you get crash recovery, replay, and full audit trail, but you must structure all I/O as activities and manage event history size. This single decision shapes your entire infrastructure layer.

2. **Isolation model** (D4A): If you choose "no isolation" (most frameworks), you get maximum capability and simplicity but accept that the agent can do anything your process can do. If you choose multi-layer isolation, you get security guarantees but need container orchestration, sidecar processes, and network policy management. This single decision determines your deployment architecture.

---

## Summary: All Decisions at a Glance

### The Turn (Universal — every agent answers these)
| # | Decision | Key Question |
|---|----------|-------------|
| D1 | Trigger | What starts this turn? (user, task, signal, schedule, system event) |
| D2A | Identity | How does the agent know what it is? (prompt injection ↔ structural constraint) |
| D2B | History | How is conversation state maintained? (in-memory ↔ event-sourced ↔ stateless) |
| D2C | Memory | How is persistent knowledge accessed? (hot ↔ warm ↔ cold ↔ structural) |
| D2D | Tools | How does the agent know what it can do? (static schemas ↔ lazy catalog ↔ structural map) |
| D2E | Budget | How is the context window allocated? (what % for system, history, reserve?) |
| D3A | Model | Which model handles this, and how hard does it think? (single → two-tier → two-level routing → speculative) |
| D3B | Durability | What survives a crash? (nothing ↔ checkpoint ↔ event replay ↔ durable execution) |
| D3C | Retry | How are failures handled? (SDK retry ↔ framework retry ↔ no retry for non-transient) |
| D3D | Output shape | How is the model's output constrained? (none → JSON mode → strict schema → typed pipeline) |
| D4A | Isolation | How trusted is generated code? (none ↔ permission gate ↔ container ↔ multi-layer) |
| D4B | Credentials | Does the agent see secrets? (in-process → mounted → proxy-mediated → boundary-injected → workload identity) |
| D4C | Backfill | How do tool results re-enter context? (raw ↔ formatted ↔ sanitized) |
| D5 | Exit | What ends the turn? (model done → max turns → budget → goal eval → loop detection → self-assessment → circuit breaker → observer) |

### Composition (When Multiple Agents)
| # | Decision | Key Question |
|---|----------|-------------|
| C1 | Child context | What does the next agent see? (full inheritance ↔ task-only ↔ structural isolation) |
| C2 | Result return | How do results flow back? (direct inject ↔ summary ↔ two-path ↔ shared state) |
| C3 | Lifecycle | How long does the child live? (fire-and-forget ↔ long-lived ↔ conversation-scoped) |
| C4 | Communication | How do agents talk? (function call ↔ filesystem ↔ signals ↔ events ↔ shared state) |
| C5 | Observation | Who watches execution? (nobody ↔ human ↔ oracle tool ↔ guardrails ↔ observer agent) |

### Lifecycle (Cross-Cutting)
| # | Decision | Key Question |
|---|----------|-------------|
| L1 | Memory writes | When and what to persist? (every tool use ↔ task completion ↔ pre-compaction flush) |
| L2 | Compaction | How to manage context growth? (summarize ↔ truncate ↔ continue-as-new ↔ fresh session) |
| L3 | Crash recovery | What survives a crash? (nothing ↔ checkpoint ↔ event replay ↔ durable execution replay) |
| L4 | Budget | How to control cost? (token limits ↔ session caps ↔ workflow budget ↔ model routing) |
| L5 | Observability | What can you see? (logs ↔ traces ↔ event history ↔ hooks ↔ queries) |

**Total: 24 architectural decisions.** Every agentic AI system, from a 10-line script to a production multi-agent platform, makes a choice (explicit or implicit) on each of these. The choices interact — durability constrains composition, isolation constrains credentials, output shape constrains composition, compaction constrains memory, budget constrains everything. Understanding the topology of these decisions is the foundation for making intelligent ones.

---

## Open Questions

These are unresolved research areas that should be periodically investigated to enrich this document:

### Context Engineering
- **Optimal capability surface compression**: Aider's tree-sitter repo map compresses codebase awareness into ~1K tokens. What's the equivalent technique for API surfaces, database schemas, document collections, or tool catalogs? Is there a general theory of "minimum viable context" for different domain types?
- **System prompt vs tool output relative influence**: Braintrust's data shows tools are ~80% of tokens, but does token volume correlate with influence on model behavior? Research on attention patterns, positional encoding effects, and instruction-following priority could clarify the actual causal relationship.
- **Context window degradation curves**: Huntley claims more allocation = worse performance. What's the actual degradation function? Is it linear, exponential, or threshold-based? Does it differ by model family? At what point does adding context actively hurt?

### Observer Pattern
- **Observer architecture taxonomy**: What are all the intervention points an observer can have? (pre-turn, mid-inference, post-tool, post-turn, continuous) How do these map to existing implementations beyond OpenAI guardrails and AWS observer agents?
- **Observer overhead vs value**: When does the cost of running an observer (inference cost, latency) pay for itself in prevented errors, security catches, or quality improvements? Is there data on this from production systems?
- **Observer composability**: Can observers observe other observers? What are the failure modes of observer chains? Is there an optimal observer topology for different risk profiles?
- **Active context steering**: Can an observer continuously enrich an agent's context mid-execution (not just halt/approve)? What are the mechanisms — signal injection, memory mutation, context rewriting? What are the safety implications of an observer that can modify context?

### Compaction & Memory
- **Pre-compaction flush completeness**: How do you know the flush captured everything important? Is there a way to measure information loss across compaction? Can you validate flush quality automatically?
- **Cross-session memory retrieval quality**: How well do hybrid search approaches (BM25 + vector) actually work for agent memory retrieval? What's the recall rate for important context? Are there better approaches emerging?
- **Memory conflict resolution**: When multiple agents write to shared memory concurrently, how should conflicts be resolved? Git merge strategies? Last-write-wins? CRDTs?

### Composition
- **Optimal context stripping for delegation**: How much context should a parent pass to a child? Is there a measurable relationship between context size and child task performance? Can you auto-tune this?
- **Handoff state transfer**: The OpenAI handoff pattern transfers conversation history. What's the optimal amount of state to carry across a handoff? Full history? Summary? Just the last N turns?
- **Dynamic pattern selection**: Can a system automatically choose the right composition pattern (chain vs parallel vs orchestrator) based on task characteristics? What features of a task predict which pattern will perform best?

### Isolation & Security
- **Isolation overhead at scale**: What's the actual performance cost of multi-layer isolation (gVisor + network policy + credential sidecar) under realistic workloads? How does this scale with agent count?
- **Agent-generated code threat modeling**: What are the actual attack vectors when an LLM generates and executes code from untrusted inputs? Beyond container escape, what about data exfiltration via tool outputs, timing side channels, or resource exhaustion?

### Development Lifecycle (Out-of-Scope for Runtime Architecture, but Real Decisions)
These are decisions every production team makes, but they operate outside the Turn/Composition/Lifecycle runtime framework. They're documented here as open questions rather than decision points because they concern the engineering process around agents, not the agents' runtime architecture.

- **Evaluation and release gating**: How do you know the agent works? How do you prevent regressions? Devin gates releases on SWE-bench Verified. Braintrust integrates evals into CI/CD. Most open-source agents have no formal evaluation. This is a real decision that's genuinely independent of the 24 runtime decisions. Should it become L6?
- **Agent versioning and rollback**: How is the agent's configuration (prompt + model + tools + policies) versioned as a unit? LangGraph Cloud supports graph versioning with deployment slots. Devin uses formal releases with rollback. Most systems have no versioning. Should it become L7?
- **Agent identity registries**: As agents proliferate, how do you discover, authenticate, and govern them? MCP registries and A2A protocol (Google, 2025) address tool and agent discovery. No production system has a comprehensive agent registry yet. Is this a future decision point?

### Fundamentals
- **Is the turn the right atomic unit?** Could there be a sub-turn primitive (individual tool calls, individual reasoning steps) that provides better composability? The fine-grained Temporal activity approach (each LLM call is a separate activity) suggests this, but adds complexity.
- **Convergence or divergence?** Are agentic architectures converging toward a standard stack, or are they diverging into specialized niches? Track: do new systems introduced in 2026 map cleanly onto this framework's 24 decisions, or do they introduce genuinely new primitives?
- **What's missing from this framework?** Use this document to analyze 5+ new agentic systems as they emerge. If a system makes a decision that doesn't fit any of the 24, that's a signal this framework needs a new primitive.
