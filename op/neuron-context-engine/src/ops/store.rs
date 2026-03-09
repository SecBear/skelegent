//! StateStore integration operations.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::effect::Scope;
use layer0::lifecycle::CompactionPolicy;
use layer0::state::StateStore;
use std::sync::Arc;

/// Type alias for the extractor closure used by [`FlushToStore`].
type Extractor = Arc<dyn Fn(&[Message]) -> serde_json::Value + Send + Sync>;

/// Type alias for the formatter closure used by [`InjectFromStore`].
type Formatter = Arc<dyn Fn(&str, &serde_json::Value) -> String + Send + Sync>;

/// Where to insert injected messages in the context.
#[derive(Debug, Clone, Default)]
pub enum InjectionPosition {
    /// After the first system message (position 1), or position 0 if no system message.
    /// This is the default.
    #[default]
    AfterSystemPrompt,
    /// At the end of the message list.
    Append,
    /// At a specific index. Clamped to `ctx.messages.len()`.
    At(usize),
}

/// Extract content from context messages and write it to a [`StateStore`].
///
/// The extractor function transforms the current messages into a JSON value.
/// The result is written to the store under the given scope and key.
pub struct FlushToStore {
    store: Arc<dyn StateStore>,
    scope: Scope,
    key: String,
    extractor: Extractor,
}

impl FlushToStore {
    /// Create a new `FlushToStore` op.
    ///
    /// The `extractor` closure is called with the current context messages
    /// and must return a JSON value to persist under `scope`/`key`.
    pub fn new(
        store: Arc<dyn StateStore>,
        scope: Scope,
        key: impl Into<String>,
        extractor: impl Fn(&[Message]) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        Self {
            store,
            scope,
            key: key.into(),
            extractor: Arc::new(extractor),
        }
    }
}

#[async_trait]
impl ContextOp for FlushToStore {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        let value = (self.extractor)(&ctx.messages);
        self.store
            .write(&self.scope, &self.key, value)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?;
        tracing::info!(key = %self.key, "neuron.flush_to_store");
        Ok(())
    }
}

/// Search a [`StateStore`] and inject matching results as messages.
///
/// Performs a search query against the store, then inserts each result
/// as a message at the configured [`InjectionPosition`] (default:
/// after any existing system message at position 0).
pub struct InjectFromStore {
    store: Arc<dyn StateStore>,
    scope: Scope,
    query: String,
    limit: usize,
    position: InjectionPosition,
    role: Role,
    policy: CompactionPolicy,
    formatter: Formatter,
}

impl InjectFromStore {
    /// Create a new `InjectFromStore` op.
    ///
    /// Searches the store for `query` and injects up to `limit` results
    /// as system messages into the context, immediately after the
    /// existing system prompt (if any).
    ///
    /// Use the builder methods ([`Self::with_position`], [`Self::with_role`],
    /// [`Self::with_policy`], [`Self::with_formatter`]) to customise behaviour.
    pub fn new(
        store: Arc<dyn StateStore>,
        scope: Scope,
        query: impl Into<String>,
        limit: usize,
    ) -> Self {
        Self {
            store,
            scope,
            query: query.into(),
            limit,
            position: InjectionPosition::AfterSystemPrompt,
            role: Role::System,
            policy: CompactionPolicy::CompressFirst,
            formatter: Arc::new(|key, value| format!("[Memory: {}] {}", key, value)),
        }
    }

    /// Set where injected messages are inserted.
    pub fn with_position(mut self, position: InjectionPosition) -> Self {
        self.position = position;
        self
    }

    /// Set the role for injected messages (default: [`Role::System`]).
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Set the compaction policy for injected messages (default: [`CompactionPolicy::CompressFirst`]).
    pub fn with_policy(mut self, policy: CompactionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set a custom formatter for injected messages.
    ///
    /// The formatter receives `(key, value)` from the store and returns
    /// the message text content.
    pub fn with_formatter(
        mut self,
        formatter: impl Fn(&str, &serde_json::Value) -> String + Send + Sync + 'static,
    ) -> Self {
        self.formatter = Arc::new(formatter);
        self
    }
}

#[async_trait]
impl ContextOp for InjectFromStore {
    type Output = usize;

    async fn execute(&self, ctx: &mut Context) -> Result<usize, EngineError> {
        let results = self
            .store
            .search(&self.scope, &self.query, self.limit)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?;

        let mut messages = Vec::new();
        for result in &results {
            if let Some(value) = self
                .store
                .read(&self.scope, &result.key)
                .await
                .map_err(|e| EngineError::Custom(Box::new(e)))?
            {
                let text = (self.formatter)(&result.key, &value);
                let mut msg = Message::new(self.role.clone(), Content::text(text));
                msg.meta.policy = self.policy;
                messages.push(msg);
            }
        }

        let count = messages.len();

        let insert_at = match &self.position {
            InjectionPosition::AfterSystemPrompt => {
                if ctx.messages.first().is_some_and(|m| m.role == Role::System) {
                    1
                } else {
                    0
                }
            }
            InjectionPosition::Append => ctx.messages.len(),
            InjectionPosition::At(idx) => (*idx).min(ctx.messages.len()),
        };

        for (i, msg) in messages.into_iter().enumerate() {
            ctx.messages.insert(insert_at + i, msg);
        }

        tracing::info!(query = %self.query, injected = count, "neuron.inject_from_store");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use layer0::error::StateError;
    use layer0::state::SearchResult;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::RwLock;

    struct MockStore {
        data: RwLock<HashMap<String, serde_json::Value>>,
    }

    impl MockStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: RwLock::new(HashMap::new()),
            })
        }
    }

    #[async_trait]
    impl StateStore for MockStore {
        async fn read(
            &self,
            _scope: &Scope,
            key: &str,
        ) -> Result<Option<serde_json::Value>, StateError> {
            Ok(self.data.read().unwrap().get(key).cloned())
        }

        async fn write(
            &self,
            _scope: &Scope,
            key: &str,
            value: serde_json::Value,
        ) -> Result<(), StateError> {
            self.data.write().unwrap().insert(key.to_string(), value);
            Ok(())
        }

        async fn delete(&self, _scope: &Scope, key: &str) -> Result<(), StateError> {
            self.data.write().unwrap().remove(key);
            Ok(())
        }

        async fn list(&self, _scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
            let data = self.data.read().unwrap();
            Ok(data
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }

        async fn search(
            &self,
            _scope: &Scope,
            query: &str,
            limit: usize,
        ) -> Result<Vec<SearchResult>, StateError> {
            let data = self.data.read().unwrap();
            let mut results: Vec<SearchResult> = data
                .keys()
                .filter(|k| k.contains(query))
                .take(limit)
                .map(|k| SearchResult::new(k.clone(), 1.0))
                .collect();
            // Sort for deterministic ordering in tests.
            results.sort_by(|a, b| a.key.cmp(&b.key));
            Ok(results)
        }
    }

    #[tokio::test]
    async fn flush_writes_to_store() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::User, Content::text("hello")));

        ctx.run(FlushToStore::new(
            store.clone(),
            Scope::Global,
            "test_key",
            |_msgs| json!({"summary": "test"}),
        ))
        .await
        .unwrap();

        let data = store.data.read().unwrap();
        assert_eq!(data.get("test_key"), Some(&json!({"summary": "test"})));
    }

    #[tokio::test]
    async fn inject_from_store_adds_messages() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("mem_a".to_string(), json!("memory content A"));
            data.insert("mem_b".to_string(), json!("memory content B"));
        }

        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::System, Content::text("main system")));
        ctx.messages
            .push(Message::new(Role::User, Content::text("user question")));

        ctx.run(InjectFromStore::new(
            store.clone(),
            Scope::Global,
            "mem",
            10,
        ))
        .await
        .unwrap();

        // Original system message still at position 0.
        assert_eq!(ctx.messages[0].role, Role::System);
        assert_eq!(ctx.messages[0].text_content(), "main system");

        // Two injected system messages at positions 1 and 2.
        assert_eq!(ctx.messages[1].role, Role::System);
        assert_eq!(ctx.messages[2].role, Role::System);

        // User message shifted to position 3.
        assert_eq!(ctx.messages[3].role, Role::User);
        assert_eq!(ctx.messages.len(), 4);
    }

    #[tokio::test]
    async fn inject_from_store_returns_count() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("mem_1".to_string(), json!("first"));
            data.insert("mem_2".to_string(), json!("second"));
        }

        let mut ctx = Context::new();
        let count = ctx
            .run(InjectFromStore::new(
                store.clone(),
                Scope::Global,
                "mem",
                10,
            ))
            .await
            .unwrap();

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn inject_from_store_empty_results() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::User, Content::text("hello")));

        let count = ctx
            .run(InjectFromStore::new(
                store.clone(),
                Scope::Global,
                "nonexistent",
                10,
            ))
            .await
            .unwrap();

        assert_eq!(count, 0);
        assert_eq!(ctx.messages.len(), 1);
    }

    #[tokio::test]
    async fn flush_extractor_receives_messages() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::User, Content::text("hello world")));
        ctx.messages
            .push(Message::new(Role::Assistant, Content::text("hi there")));

        ctx.run(FlushToStore::new(
            store.clone(),
            Scope::Global,
            "messages_key",
            |msgs| {
                json!({
                    "count": msgs.len(),
                    "first": msgs.first().map(|m| m.text_content()),
                })
            },
        ))
        .await
        .unwrap();

        let data = store.data.read().unwrap();
        let stored = data.get("messages_key").unwrap();
        assert_eq!(stored["count"], 2);
        assert_eq!(stored["first"], "hello world");
    }

    #[tokio::test]
    async fn inject_with_append_position() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("mem_x".to_string(), json!("appended memory"));
        }

        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::System, Content::text("system")));
        ctx.messages
            .push(Message::new(Role::User, Content::text("user msg")));

        ctx.run(
            InjectFromStore::new(store.clone(), Scope::Global, "mem", 10)
                .with_position(InjectionPosition::Append),
        )
        .await
        .unwrap();

        // Memory should be at the end
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(ctx.messages[2].role, Role::System);
        assert!(ctx.messages[2].text_content().contains("appended memory"));
    }

    #[tokio::test]
    async fn inject_with_custom_role_and_formatter() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("fact_1".to_string(), json!("the sky is blue"));
        }

        let mut ctx = Context::new();
        ctx.messages
            .push(Message::new(Role::User, Content::text("hello")));

        ctx.run(
            InjectFromStore::new(store.clone(), Scope::Global, "fact", 10)
                .with_role(Role::User)
                .with_formatter(|key, value| format!("Fact ({key}): {value}")),
        )
        .await
        .unwrap();

        // Should be injected at position 0 (no system message), with Role::User
        assert_eq!(ctx.messages[0].role, Role::User);
        assert_eq!(
            ctx.messages[0].text_content(),
            "Fact (fact_1): \"the sky is blue\""
        );
    }

    #[tokio::test]
    async fn inject_with_custom_policy() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("mem_p".to_string(), json!("pinned memory"));
        }

        let mut ctx = Context::new();
        ctx.run(
            InjectFromStore::new(store.clone(), Scope::Global, "mem", 10)
                .with_policy(CompactionPolicy::Pinned),
        )
        .await
        .unwrap();

        assert_eq!(ctx.messages[0].meta.policy, CompactionPolicy::Pinned);
    }
}
