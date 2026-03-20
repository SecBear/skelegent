//! Agent-controlled memory tools.
//!
//! Four [`ToolDyn`] implementations that let the agent read and write its own
//! memory via tool calls rather than hard-coded orchestrator rules:
//!
//! | Tool | Action |
//! |------|--------|
//! | [`MemoryStoreTool`]  | Write a key/value pair |
//! | [`MemorySearchTool`] | Semantic search over stored values |
//! | [`MemoryRecallTool`] | Read a single key |
//! | [`MemoryForgetTool`] | Delete a key |
//!
//! # Setup
//!
//! Inject an `Arc<dyn StateStore>` into the [`DispatchContext`] before
//! dispatching any memory tool call:
//!
//! ```ignore
//! use std::sync::Arc;
//! use layer0::{DispatchContext, id::{DispatchId, OperatorId}, state::StateStore};
//! use skg_tool::memory::register_memory_tools;
//!
//! let ctx = DispatchContext::new(DispatchId::new("d1"), OperatorId::new("op1"))
//!     .with_extension(Arc::new(my_store) as Arc<dyn StateStore>);
//! ```
//!
//! Optionally inject a [`Scope`] to override the default session scope:
//!
//! ```ignore
//! use layer0::effect::Scope;
//!
//! let ctx = ctx.with_extension(Scope::Global);
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::{
    Lifetime, MemoryScope, Scope, SessionId, StateStore, StoreOptions,
    DispatchContext,
};
use serde_json::{json, Value};

use crate::{ToolDyn, ToolError, ToolRegistry};

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Resolve the dispatch-level [`Scope`] from context extensions.
///
/// Prefers an explicitly injected `Scope`; falls back to a session scope keyed
/// on the `dispatch_id` so every tool invocation has a stable scope without
/// requiring the caller to inject one explicitly.
fn resolve_scope(ctx: &DispatchContext) -> Scope {
    ctx.extensions()
        .get::<Scope>()
        .cloned()
        .unwrap_or_else(|| Scope::Session(SessionId::new(ctx.dispatch_id.as_str())))
}

/// Extract a cloned `Arc<dyn StateStore>` from context extensions.
///
/// Returns [`ToolError::ExecutionFailed`] if no store has been injected.
/// Callers must call `ctx.with_extension(store as Arc<dyn StateStore>)` before
/// dispatching any memory tool.
fn get_store(ctx: &DispatchContext) -> Result<Arc<dyn StateStore>, ToolError> {
    ctx.extensions()
        .get::<Arc<dyn StateStore>>()
        .cloned()
        .ok_or_else(|| {
            ToolError::ExecutionFailed(
                "StateStore not available — inject Arc<dyn StateStore> into DispatchContext extensions".into(),
            )
        })
}

/// Parse an optional input string into a [`MemoryScope`] lifetime hint.
///
/// Accepts `"turn"`, `"session"` (default), `"global"`, or any other string
/// which is treated as an entity scope.
fn parse_memory_scope(s: &str) -> MemoryScope {
    match s {
        "turn" => MemoryScope::Turn,
        "global" => MemoryScope::Global,
        "session" => MemoryScope::Session,
        other => MemoryScope::Entity(other.to_owned()),
    }
}

/// Map a [`MemoryScope`] lifetime hint to a [`Lifetime`] advisory.
fn memory_scope_to_lifetime(ms: &MemoryScope) -> Option<Lifetime> {
    match ms {
        MemoryScope::Turn => Some(Lifetime::Transient),
        MemoryScope::Session => Some(Lifetime::Session),
        MemoryScope::Global | MemoryScope::Entity(_) => Some(Lifetime::Durable),
        // #[non_exhaustive] — future variants get no advisory.
        _ => None,
    }
}

// ── MemoryStoreTool ────────────────────────────────────────────────────────────

/// Store a value in memory with a key and optional scope.
///
/// Reads the dispatch-level [`Scope`] from context extensions (or falls back
/// to a session scope). Writes via `StateStore::write_hinted` so advisory
/// lifetime hints are forwarded to backends that support them.
pub struct MemoryStoreTool;

impl ToolDyn for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a value in memory with a key and optional scope."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["key", "value"],
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Key to store the value under."
                },
                "value": {
                    "description": "Value to store (any JSON type)."
                },
                "scope": {
                    "type": "string",
                    "description": "Memory lifetime scope: 'turn', 'session' (default), 'global', or an entity name.",
                    "default": "session"
                }
            },
            "additionalProperties": false
        })
    }

    fn call(
        &self,
        input: Value,
        ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        let store = get_store(ctx);
        let scope = resolve_scope(ctx);
        Box::pin(async move {
            let store = store?;
            let obj = input
                .as_object()
                .ok_or_else(|| ToolError::InvalidInput("input must be a JSON object".into()))?;
            let key = obj
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidInput("'key' must be a non-null string".into()))?
                .to_owned();
            // 'value' is required but may itself be null — check key presence.
            if !obj.contains_key("value") {
                return Err(ToolError::InvalidInput("'value' is required".into()));
            }
            let value = obj["value"].clone();
            let scope_str = obj
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("session");
            let memory_scope = parse_memory_scope(scope_str);
            let options = StoreOptions {
                lifetime: memory_scope_to_lifetime(&memory_scope),
                ..Default::default()
            };
            store
                .write_hinted(&scope, &key, value, &options)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            Ok(json!({ "stored": true, "key": key, "scope": scope_str }))
        })
    }
}

// ── MemorySearchTool ───────────────────────────────────────────────────────────

/// Search memory for relevant information.
///
/// Calls `StateStore::search` then fetches the full value for each result so
/// callers receive `{ key, value, score }` rather than a bare key/score pair.
/// Backends that do not implement semantic search return an empty array.
pub struct MemorySearchTool;

impl ToolDyn for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search memory for relevant information."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query string to search for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 5).",
                    "default": 5,
                    "minimum": 1
                }
            },
            "additionalProperties": false
        })
    }

    fn call(
        &self,
        input: Value,
        ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        let store = get_store(ctx);
        let scope = resolve_scope(ctx);
        Box::pin(async move {
            let store = store?;
            let obj = input
                .as_object()
                .ok_or_else(|| ToolError::InvalidInput("input must be a JSON object".into()))?;
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidInput("'query' must be a non-null string".into()))?
                .to_owned();
            let limit = obj
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;
            let results = store
                .search(&scope, &query, limit)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            // Fetch the stored value for each result so callers get full entries.
            let mut output = Vec::with_capacity(results.len());
            for sr in results {
                let val = store
                    .read(&scope, &sr.key)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
                    .unwrap_or(Value::Null);
                output.push(json!({ "key": sr.key, "value": val, "score": sr.score }));
            }
            Ok(Value::Array(output))
        })
    }
}

// ── MemoryRecallTool ───────────────────────────────────────────────────────────

/// Recall a specific value by key.
///
/// Returns the stored value directly, or `{ "found": false }` if the key does
/// not exist in the current scope.
pub struct MemoryRecallTool;

impl ToolDyn for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Recall a specific value by key."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["key"],
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Key to recall."
                }
            },
            "additionalProperties": false
        })
    }

    fn call(
        &self,
        input: Value,
        ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        let store = get_store(ctx);
        let scope = resolve_scope(ctx);
        Box::pin(async move {
            let store = store?;
            let key = input["key"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidInput("'key' must be a non-null string".into()))?
                .to_owned();
            match store
                .read(&scope, &key)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
            {
                Some(val) => Ok(val),
                None => Ok(json!({ "found": false })),
            }
        })
    }
}

// ── MemoryForgetTool ───────────────────────────────────────────────────────────

/// Delete a value from memory.
///
/// No-op if the key does not exist (consistent with `StateStore::delete`
/// semantics). Always returns `{ "deleted": true, "key": "..." }`.
pub struct MemoryForgetTool;

impl ToolDyn for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Delete a value from memory."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["key"],
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Key to delete."
                }
            },
            "additionalProperties": false
        })
    }

    fn call(
        &self,
        input: Value,
        ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        let store = get_store(ctx);
        let scope = resolve_scope(ctx);
        Box::pin(async move {
            let store = store?;
            let key = input["key"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidInput("'key' must be a non-null string".into()))?
                .to_owned();
            store
                .delete(&scope, &key)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            Ok(json!({ "deleted": true, "key": key }))
        })
    }
}

// ── Registration ───────────────────────────────────────────────────────────────

/// Register all memory tools into a [`ToolRegistry`].
///
/// Registers [`MemoryStoreTool`], [`MemorySearchTool`], [`MemoryRecallTool`],
/// and [`MemoryForgetTool`] in one call. Existing tools with the same names
/// are overwritten.
pub fn register_memory_tools(registry: &mut ToolRegistry) {
    registry.register(Arc::new(MemoryStoreTool));
    registry.register(Arc::new(MemorySearchTool));
    registry.register(Arc::new(MemoryRecallTool));
    registry.register(Arc::new(MemoryForgetTool));
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::{
        effect::Scope,
        id::{DispatchId, OperatorId, SessionId},
        state::StateStore,
        test_utils::InMemoryStore,
    };
    use serde_json::json;
    use std::sync::Arc;

    /// Build a test context with an injected `InMemoryStore`.
    fn ctx_with_store(store: Arc<dyn StateStore>) -> DispatchContext {
        DispatchContext::new(DispatchId::new("test-dispatch"), OperatorId::new("test-op"))
            .with_extension(store)
    }

    /// Scope that matches what `resolve_scope` falls back to for the test context.
    fn test_scope() -> Scope {
        Scope::Session(SessionId::new("test-dispatch"))
    }

    #[tokio::test]
    async fn memory_store_writes_to_state() {
        let store = Arc::new(InMemoryStore::new());
        let ctx = ctx_with_store(Arc::clone(&store) as Arc<dyn StateStore>);

        let result = MemoryStoreTool
            .call(json!({ "key": "foo", "value": 42 }), &ctx)
            .await
            .expect("store should succeed");

        assert_eq!(result["stored"], json!(true));
        assert_eq!(result["key"], json!("foo"));

        let stored = store
            .read(&test_scope(), "foo")
            .await
            .expect("read should succeed");
        assert_eq!(stored, Some(json!(42)));
    }

    #[tokio::test]
    async fn memory_recall_reads_from_state() {
        let store = Arc::new(InMemoryStore::new());
        store
            .write(&test_scope(), "bar", json!("hello"))
            .await
            .expect("pre-populate");
        let ctx = ctx_with_store(Arc::clone(&store) as Arc<dyn StateStore>);

        let result = MemoryRecallTool
            .call(json!({ "key": "bar" }), &ctx)
            .await
            .expect("recall should succeed");

        assert_eq!(result, json!("hello"));

        // Missing key returns { found: false }.
        let missing = MemoryRecallTool
            .call(json!({ "key": "missing" }), &ctx)
            .await
            .expect("recall of missing key should not error");
        assert_eq!(missing, json!({ "found": false }));
    }

    #[tokio::test]
    async fn memory_search_returns_results() {
        let store = Arc::new(InMemoryStore::new());
        store
            .write(&test_scope(), "k1", json!("value one"))
            .await
            .expect("pre-populate k1");
        store
            .write(&test_scope(), "k2", json!("value two"))
            .await
            .expect("pre-populate k2");
        let ctx = ctx_with_store(Arc::clone(&store) as Arc<dyn StateStore>);

        let result = MemorySearchTool
            .call(json!({ "query": "value", "limit": 10 }), &ctx)
            .await
            .expect("search should not error");

        // InMemoryStore does not implement semantic search, so the result is an
        // empty array. This test verifies the tool runs end-to-end without error
        // and returns an array type.
        assert!(result.is_array(), "result must be an array");
    }

    #[tokio::test]
    async fn memory_forget_deletes() {
        let store = Arc::new(InMemoryStore::new());
        store
            .write(&test_scope(), "to-delete", json!(true))
            .await
            .expect("pre-populate");
        let ctx = ctx_with_store(Arc::clone(&store) as Arc<dyn StateStore>);

        let result = MemoryForgetTool
            .call(json!({ "key": "to-delete" }), &ctx)
            .await
            .expect("forget should succeed");

        assert_eq!(result["deleted"], json!(true));
        assert_eq!(result["key"], json!("to-delete"));

        let after = store
            .read(&test_scope(), "to-delete")
            .await
            .expect("read after delete");
        assert_eq!(after, None, "key must be absent after forget");
    }

    #[test]
    fn register_memory_tools_adds_four() {
        let mut registry = ToolRegistry::new();
        register_memory_tools(&mut registry);
        assert_eq!(registry.len(), 4);
        assert!(registry.get("memory_store").is_some());
        assert!(registry.get("memory_search").is_some());
        assert!(registry.get("memory_recall").is_some());
        assert!(registry.get("memory_forget").is_some());
    }
}
