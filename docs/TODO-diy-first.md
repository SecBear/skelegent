# DIY-First Decomposition Queue

Principle: **Every multi-step operation must be decomposable into independently callable primitives.**
Convenience wrappers are opt-in, built FROM the primitives. Developers can compose their own
wrappers using their own compositions. If the only way to do X is the bundled convenience, the
design is wrong.

Pattern:
```
PRIMITIVES (always available, independently callable):
  step_1()  →  step_2()  →  step_3()  →  step_4()
                ↑ developer intervenes here

CONVENIENCE (opt-in wrapper over primitives):
  do_all_steps()  ← calls step_1..4 with sensible defaults
```

---

## P0: react_loop — decompose the ReAct loop

**Current problem**: `react_loop` is the only way to run a ReAct agent. It bundles
compile→infer→append→exit-check→approval-check→dispatch→error-format→inject into one
function. A developer who needs custom error handling, tool result transformation, retry
logic, or exit conditions must rewrite the entire loop.

**What to expose as primitives:**

| Primitive | What it does | Currently |
|-----------|-------------|-----------|
| `compile + infer` | Compile context, call provider | Already DIY (`ctx.compile()`, `compiled.infer()`) |
| `append_response` | Add assistant response to context | Already DIY (`ctx.run(AppendResponse)`) |
| `check_exit` | Map `StopReason` → `ExitReason` | Inline in react_loop, not extractable |
| `check_approval` | Filter tool calls needing approval, emit effects | Inline in react_loop, not extractable |
| `dispatch_tool` | Lookup + call one tool, return raw `serde_json::Value` | Bundled in `ExecuteTool` (see P1) |
| `format_tool_result` | Convert `Value` → `String` for model | Inline in `ExecuteTool`, not extractable |
| `format_tool_error` | Convert `EngineError` → tool result string | Hardcoded `format!("Error: {e}")` in react_loop |
| `make_tool_message` | Build a tool-result `Message` from id+name+content | `InferResponse::tool_result_message()` — already public |

**Deliverables:**
- [ ] Extract `check_exit(stop_reason: &StopReason) -> ExitReason` as public fn
- [ ] Extract `check_approval(tool_calls: &[ToolCall], registry: &ToolRegistry) -> Vec<Effect>` as public fn
- [ ] Extract `format_tool_error` as a public fn with a default impl (current `format!("Error: {e}")`)
- [ ] Document the "write your own loop" pattern using these primitives
- [ ] `react_loop` stays unchanged — it becomes a convenience wrapper over the primitives
- [ ] Apply same decomposition to `stream_react_loop`
- [ ] Apply same decomposition to `react_loop_structured`

**Files:** `react.rs`, `stream_react.rs`

---

## P1: ExecuteTool — decompose tool dispatch

**Current problem**: `ExecuteTool` bundles lookup→dispatch→format→metrics in one atomic op.
Developer cannot: retry failed tools, use fallback tools, customize result formatting,
inspect raw JSON before stringification, or opt out of metrics.

**What to expose as primitives:**

| Primitive | What it does | Currently |
|-----------|-------------|-----------|
| `ToolRegistry::get()` | Lookup tool by name | Already public |
| `tool.call()` | Execute tool, return `Value` | Already public |
| `format_tool_result(value: &Value) -> String` | JSON→String conversion | Inline in ExecuteTool, not extractable |

**Deliverables:**
- [ ] Extract `format_tool_result(value: &serde_json::Value) -> String` as public fn
- [ ] Make `ExecuteTool` output the raw `serde_json::Value` instead of `String`
- [ ] Move metrics increment to the convenience layer (react_loop), not the primitive
- [ ] `ExecuteTool` stays as a convenience that composes lookup+call+format — but each step is also callable independently
- [ ] Document the "custom tool dispatch" pattern

**Files:** `ops/tool.rs`, `react.rs`, `stream_react.rs`

---

## P2: InjectFromStore — decompose search→inject pipeline

**Current problem**: `InjectFromStore` bundles search→fetch→format→inject as one atomic op.
Developer cannot: filter results after search, rerank, deduplicate, batch-format multiple
results into one message, or combine results from multiple sources.

**What to expose as primitives:**

| Primitive | What it does | Currently |
|-----------|-------------|-----------|
| `store.search()` | Search the store | Already public (StateStore trait) |
| `store.read()` | Fetch value by key | Already public (StateStore trait) |
| `fetch_search_results(store, scope, results) -> Vec<(String, Value)>` | Batch-fetch values for search results | Inline in InjectFromStore, not extractable |
| `InjectMessages` | Insert messages at position in context | Already exists as op |

**Deliverables:**
- [ ] Extract `fetch_search_results(store: &dyn StateStore, scope: &Scope, results: &[SearchResult]) -> Result<Vec<(String, Value)>>` as public async fn
- [ ] Add `InjectSearchResults` op — takes `Vec<(String, Value)>`, formats, injects (the generic result injector from the design doc)
- [ ] `InjectFromStore` stays unchanged — convenience wrapper over search→fetch→format→inject
- [ ] Document the "custom search pipeline" pattern

**Files:** `ops/store.rs`

---

## P3: CompactionRule — decompose strategy composition

**Current problem**: `CompactionRule` runs ONE strategy via enum dispatch. Developer cannot
chain strategies (e.g., sliding_window THEN summarize). The `into_rule()` method hardcodes
trigger predicate (`messages.len() > max`) and priority (50).

**What to expose as primitives:**

| Primitive | What it does | Currently |
|-----------|-------------|-----------|
| `sliding_window()` | Retain N most recent messages | Already public standalone fn |
| `policy_trim()` | Drop by CompactionPolicy | Already public standalone fn |
| `summarize_with()` | LLM summarization | Already public standalone fn |
| `extract_cognitive_state_with()` | LLM state extraction | Already public standalone fn |
| `Rule::new()` | Create a rule with custom trigger/priority | Already public |

**Deliverables:**
- [ ] Add `CompactionRule::with_trigger(trigger: Trigger) -> Self` — override the default predicate
- [ ] Add `CompactionRule::with_priority(priority: i32) -> Self` — override default 50
- [ ] Document the "compose your own compaction" pattern: use standalone fns + `Compact` op + custom `Rule::new()`
- [ ] `CompactionRule` stays as convenience — but the standalone fns ARE the primitives, and this is made explicit in docs

**Files:** `rules/compaction.rs`

---

## P4: summarize_with / extract_cognitive_state_with — expose raw response

**Current problem**: Both functions call `provider.infer()` internally and immediately
parse/format the response. Developer cannot inspect the raw LLM response before it becomes
a `Message` or `Value`. Also, `strip_json_fences()` is called silently with no opt-out.

**What to expose as primitives:**

| Primitive | What it does | Currently |
|-----------|-------------|-----------|
| Build summarization `InferRequest` | Construct the request from messages+config | Inline in summarize_with |
| `provider.infer()` | Call the LLM | Already public |
| Parse response into `Message` | Extract text, set policy | Inline in summarize_with |
| `strip_json_fences()` | Remove markdown code fences from JSON | Private fn |

**Deliverables:**
- [ ] Add `build_summarize_request(messages: &[Message], config: &SummarizeConfig) -> InferRequest` — public fn
- [ ] Add `parse_summarize_response(response: InferResponse, config: &SummarizeConfig) -> Result<Message>` — public fn
- [ ] Make `strip_json_fences()` public
- [ ] Add `build_extract_request(messages: &[Message], config: &ExtractConfig) -> InferRequest` — public fn
- [ ] Add `parse_extract_response(response: InferResponse) -> Result<Value>` — public fn
- [ ] `summarize_with` and `extract_cognitive_state_with` stay unchanged — convenience over build→infer→parse
- [ ] Document the "custom summarization pipeline" pattern

**Files:** `rules/compaction.rs`

---

## P5: FlushToStore — minor: expose inspection point

**Current problem**: Extract→write with no inspection between. Minor severity — the
extractor closure gives the developer control over WHAT is extracted, but not whether
to write (conditional flush) or transform after extraction.

**Deliverables:**
- [ ] No code change needed — the developer can already do this DIY:
  ```rust
  let value = my_extractor(&ctx.messages);
  if should_write(&value) {
    store.write(&scope, "key", value).await?;
  }
  ```
- [ ] Document that `FlushToStore` is a convenience; DIY is `extractor() + store.write()`

**Files:** `ops/store.rs` (docs only)

---

## P6: State store rich APIs (from design doc)

**Deliverables:**
- [ ] `SqliteStore::fts_search()` — promote `pub(crate) fn fts5_search` to public method on `SqliteStore`
- [ ] `FsStore::regex_search()` — promote internal regex impl to public method
- [ ] `CozoStore::datalog_query()` — new public method for raw Datalog
- [ ] `CozoStore::traverse_full()` — graph traversal returning values, not just keys
- [ ] (Future) `SqliteStore::vector_search()` — behind `sqlite-vec` feature flag

**Files:** `extras/state/neuron-state-sqlite/src/`, `extras/state/neuron-state-cozo/src/`, `neuron/state/neuron-state-fs/src/`

---

## P7: Conversation persistence (from design doc)

**Deliverables:**
- [ ] `SaveConversation` op — serialize `ctx.messages` to `StateStore` under scope/key
- [ ] `LoadConversation` op — deserialize `Vec<Message>` from store, replace `ctx.messages`
- [ ] Tests for round-trip save/load
- [ ] Document the pattern: load at start, save after each turn

**Files:** `ops/store.rs`

---

## Implementation Order

Phase 1 (primitives — unblock DIY):
- P0: react_loop decomposition (extract check_exit, check_approval, format_tool_error)
- P1: ExecuteTool decomposition (format_tool_result, raw Value output)
- P4: summarize/extract decomposition (build_request, parse_response, public strip_json_fences)

Phase 2 (new ops + store APIs):
- P2: InjectSearchResults + fetch_search_results
- P6: State store rich APIs (fts_search, regex_search)
- P7: SaveConversation + LoadConversation

Phase 3 (polish):
- P3: CompactionRule builder methods
- P5: FlushToStore docs
- Documentation: "write your own loop", "custom search pipeline", "custom compaction"

All existing APIs stay unchanged. Every change is additive. No breaking changes.
