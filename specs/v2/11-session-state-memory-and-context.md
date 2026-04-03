# Session State, Memory, and Context

## Purpose

Define the v2 distinction between session state, active context, persistent
state, memory, and structural state.

This spec exists because these concepts are adjacent, heavily overloaded in
agent systems, and must not collapse into a single "memory" abstraction.

## Core Distinctions

### Session State

Session state is infrastructure-facing state tied to a session identity.

It includes:

- session identity
- session-scoped persistence
- approval continuity
- resumable session metadata
- conversation-level continuity not specific to one in-memory context snapshot

Session state belongs to the kernel/state/control architecture.

### Active Context

Active context is the read model the runtime assembles for the current turn.

It includes:

- current messages
- identity/specification material
- currently loaded hot memory
- selected tool results and intermediate context entries
- per-message metadata such as compaction policy, source, and salience

Active context is a runtime concern. It is not the same thing as all session
state, and it is not required to be fully persisted.

### Persistent State

Persistent state is the durable substrate addressed by scopes and state
capabilities.

It includes both agent-relevant and non-agent-relevant data:

- memory entries
- workflow/session records
- durable control data
- audit and trace material
- application-specific records

### Memory

Memory is the agent-relevant slice of persistent state plus retrieval and
curation policy.

Memory may be represented as:

- always-loaded hot knowledge
- on-demand warm retrieval
- cold cross-session search
- curated summaries or promoted facts

Memory is not just a tool. Memory tools are one projection of memory to the
agent.

### Structural State

Structural state is information embodied in the environment or other capability
surfaces rather than in a dedicated persistent memory store.

Examples:

- files in a repository
- git history
- issue trackers
- external APIs
- database schemas

Structural state may behave like memory from the agent's point of view, but it
must not be collapsed into persistent memory storage by type.

## Kernel Responsibilities

Layer 0 must carry the stable nouns needed to represent these concepts:

- session identity types
- scopes for session/workflow/global addressing
- base state contracts
- message/context wire types
- content and artifact types
- semantic event and intent types that reference state or context activity

Layer 0 must not standardize:

- one memory architecture
- one retrieval strategy
- one compaction strategy
- one coding-agent file convention

## Runtime Responsibilities

The runtime owns:

- context assembly
- loading hot memory into active context
- querying warm/cold memory through state capabilities or tools
- deciding what enters the current context window
- compaction and context reduction policy
- context snapshots and introspection read models

The runtime may expose memory to the agent through tools, direct retrieval, or
both. The substrate remains below that interface.

## State Responsibilities

The state layer owns:

- storage and retrieval semantics
- scope isolation
- optional search/graph/blob/versioned capabilities
- session-scoped persistence
- memory hint handling when supported

The state layer does not decide what the runtime should load into context by
default.

## Orchestration Responsibilities

Orchestration owns:

- which session an invocation belongs to
- how session continuity is re-established after suspension or handoff
- when state is flushed before compaction or continue-as-new
- how session and workflow records are coordinated across agents

## Memory As Tool Projection

A memory tool is an agent-facing capability built on top of the memory substrate.

Examples:

- `search_memory`
- `write_note`
- `recall_topic`
- `load_context_snapshot`

These tools are valid and important, but they do not replace:

- session-scoped storage semantics
- persistent state capabilities
- runtime-owned context assembly

The same memory substrate must remain usable by:

- runtimes
- orchestrators
- lifecycle policies
- external queries
- agents through tools

## Hot, Warm, Cold, and Structural Memory

These are policy tiers, not separate protocol kinds.

- hot: always loaded into context
- warm: loaded on demand within the session
- cold: retrieved through cross-session search or recall
- structural: discovered through environment or capability navigation

The kernel may carry advisory hints that help these tiers, but the tiering
policy itself belongs above Layer 0.

## Context Introspection

V2 requires an explicit read model for "what the agent currently sees."

This introspection surface is read-only and belongs above Layer 0. It should be
able to answer:

- what messages are currently in active context
- what metadata each message carries
- what was loaded from memory versus generated in-session
- what was compacted or excluded
- token estimates by section when available

This is distinct from durable event history and distinct from state search.

## Snapshots

Context snapshots are a projection of active context state, not the entire
persistent state substrate.

They are useful for:

- portable save/load of conversation state
- branching sessions
- reproducing a specific context window

They must not be treated as a universal persistence mechanism for all session or
durable control state.

## Compatibility Rules

- Session identity must remain distinct from dispatch identity.
- Session-scoped data must be representable without forcing one context assembly strategy.
- Memory tools must remain optional projections, not the only interface to memory.
- Structural state access must remain representable even when no dedicated memory store exists.

## Relationship to Current Specs

This spec refines and partially supersedes the history/memory portions of
`specs/04-operator-turn-runtime.md` and `specs/07-state-core.md` for the v2
track. It also narrows the overloaded "memory" language in the current
architecture into separate substrate and runtime concerns.

## Minimum Proving Tests

- Session-scoped state remains available across multiple invocations without requiring the same active context snapshot.
- A runtime can assemble active context from hot memory, recent history, and structural state without collapsing them into one store type.
- Memory remains accessible both through direct runtime/state integration and through an agent-facing memory tool projection.
- Context introspection can report the current active context window independently of durable event history.
