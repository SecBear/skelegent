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

## Status: ALL COMPLETE

### Phase 1 — Primitives (unblock DIY)

- [x] **P0: react_loop** — `check_exit()`, `check_approval()`, `format_tool_error()` extracted as public fns
- [x] **P1: ExecuteTool** — `format_tool_result()` extracted as public fn
- [x] **P4: summarize/extract** — `SummarizeConfig::build_request()`, `SummarizeConfig::parse_response()`, `ExtractConfig::build_request()`, `ExtractConfig::parse_response()`, `strip_json_fences()` all public

### Phase 2 — New ops + store APIs

- [x] **P2: InjectFromStore** — `fetch_search_results()` extracted; `InjectSearchResults` op added
- [x] **P6: State store rich APIs** — `SqliteStore::fts5_search()` + `FtsMatch` promoted to public
- [x] **P7: Conversation persistence** — `SaveConversation` + `LoadConversation` ops added

### Phase 3 — Polish

- [x] **P3: CompactionRule** — `with_trigger()` + `with_priority()` builder methods; DIY composition documented
- [x] **P5: FlushToStore** — DIY alternative documented

---

All existing APIs stay unchanged. Every change is additive. No breaking changes.
