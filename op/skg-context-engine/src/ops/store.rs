//! StateStore integration operations.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::intent::Scope;
use layer0::lifecycle::CompactionPolicy;
use layer0::state::{SearchResult, StateStore};
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
    /// At a specific index. Clamped to `ctx.messages().len()`.
    At(usize),
}

/// Batch-fetch values from a [`StateStore`] for a set of search results.
///
/// For each [`SearchResult`], reads the corresponding value. Returns
/// `(key, value)` pairs for results that exist, silently skipping
/// keys deleted between search and fetch.
///
/// This is the fetch step that [`InjectFromStore`] performs internally.
/// Use it to inspect, filter, or rerank results before injection.
pub async fn fetch_search_results(
    store: &dyn StateStore,
    scope: &Scope,
    results: &[SearchResult],
) -> Result<Vec<(String, serde_json::Value)>, EngineError> {
    let mut fetched = Vec::with_capacity(results.len());
    for result in results {
        if let Some(value) = store
            .read(scope, &result.key)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?
        {
            fetched.push((result.key.clone(), value));
        }
    }
    Ok(fetched)
}

/// Extract content from context messages and write it to a [`StateStore`].
///
/// The extractor function transforms the current messages into a JSON value.
/// The result is written to the store under the given scope and key.
///
/// # DIY Alternative
///
/// `FlushToStore` is a convenience. For conditional writes or post-extraction
/// transforms, call the extractor and store directly:
///
/// ```ignore
/// let value = my_extractor(ctx.messages());
/// if should_write(&value) {
///     store.write(&scope, "key", value).await?;
/// }
/// ```
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
        let value = (self.extractor)(ctx.messages());
        self.store
            .write(&self.scope, &self.key, value)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?;
        tracing::info!(key = %self.key, "skg.flush_to_store");
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

        let fetched = fetch_search_results(&*self.store, &self.scope, &results).await?;

        let mut messages = Vec::new();
        for (key, value) in &fetched {
            let text = (self.formatter)(key, value);
            let mut msg = Message::new(self.role.clone(), Content::text(text));
            msg.meta.policy = self.policy;
            messages.push(msg);
        }

        let count = messages.len();

        let insert_at = match &self.position {
            InjectionPosition::AfterSystemPrompt => {
                if ctx
                    .messages()
                    .first()
                    .is_some_and(|m| m.role == Role::System)
                {
                    1
                } else {
                    0
                }
            }
            InjectionPosition::Append => ctx.messages().len(),
            InjectionPosition::At(idx) => (*idx).min(ctx.messages().len()),
        };

        for (i, msg) in messages.into_iter().enumerate() {
            ctx.insert_message(insert_at + i, msg);
        }

        tracing::info!(query = %self.query, injected = count, "skg.inject_from_store");
        Ok(count)
    }
}

/// Inject pre-fetched `(key, value)` pairs as messages into the context.
///
/// Use [`fetch_search_results`] to obtain the pairs, then optionally
/// filter or rerank them before passing to this op.
pub struct InjectSearchResults {
    results: Vec<(String, serde_json::Value)>,
    position: InjectionPosition,
    role: Role,
    policy: CompactionPolicy,
    formatter: Formatter,
}

impl InjectSearchResults {
    /// Create a new `InjectSearchResults` op from pre-fetched `(key, value)` pairs.
    pub fn new(results: Vec<(String, serde_json::Value)>) -> Self {
        Self {
            results,
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
    /// The formatter receives `(key, value)` from the results and returns
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
impl ContextOp for InjectSearchResults {
    type Output = usize;

    async fn execute(&self, ctx: &mut Context) -> Result<usize, EngineError> {
        let mut messages = Vec::new();
        for (key, value) in &self.results {
            let text = (self.formatter)(key, value);
            let mut msg = Message::new(self.role.clone(), Content::text(text));
            msg.meta.policy = self.policy;
            messages.push(msg);
        }

        let count = messages.len();

        let insert_at = match &self.position {
            InjectionPosition::AfterSystemPrompt => {
                if ctx
                    .messages()
                    .first()
                    .is_some_and(|m| m.role == Role::System)
                {
                    1
                } else {
                    0
                }
            }
            InjectionPosition::Append => ctx.messages().len(),
            InjectionPosition::At(idx) => (*idx).min(ctx.messages().len()),
        };

        for (i, msg) in messages.into_iter().enumerate() {
            ctx.insert_message(insert_at + i, msg);
        }

        tracing::info!(injected = count, "skg.inject_search_results");
        Ok(count)
    }
}

/// Serialize the current conversation messages to a [`StateStore`].
///
/// Writes the context messages as a JSON array under the given scope and key.
/// Pair with [`LoadConversation`] to restore.
pub struct SaveConversation {
    store: Arc<dyn StateStore>,
    scope: Scope,
    key: String,
}

impl SaveConversation {
    /// Create a new `SaveConversation` op.
    pub fn new(store: Arc<dyn StateStore>, scope: Scope, key: impl Into<String>) -> Self {
        Self {
            store,
            scope,
            key: key.into(),
        }
    }
}

#[async_trait]
impl ContextOp for SaveConversation {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        let value =
            serde_json::to_value(ctx.messages()).map_err(|e| EngineError::Custom(Box::new(e)))?;
        self.store
            .write(&self.scope, &self.key, value)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?;
        tracing::info!(key = %self.key, "skg.save_conversation");
        Ok(())
    }
}

/// Load conversation messages from a [`StateStore`] into the context.
///
/// Reads a JSON array of messages from the store and replaces the context messages.
/// Returns `None` if the key does not exist (context unchanged).
/// Returns `Some(count)` if loaded (previous messages replaced).
pub struct LoadConversation {
    store: Arc<dyn StateStore>,
    scope: Scope,
    key: String,
}

impl LoadConversation {
    /// Create a new `LoadConversation` op.
    pub fn new(store: Arc<dyn StateStore>, scope: Scope, key: impl Into<String>) -> Self {
        Self {
            store,
            scope,
            key: key.into(),
        }
    }
}

#[async_trait]
impl ContextOp for LoadConversation {
    type Output = Option<usize>;

    async fn execute(&self, ctx: &mut Context) -> Result<Option<usize>, EngineError> {
        let value = self
            .store
            .read(&self.scope, &self.key)
            .await
            .map_err(|e| EngineError::Custom(Box::new(e)))?;

        match value {
            None => Ok(None),
            Some(v) => {
                let messages: Vec<Message> =
                    serde_json::from_value(v).map_err(|err| EngineError::Halted {
                        reason: format!("failed to deserialize conversation: {err}"),
                    })?;
                let len = messages.len();
                ctx.set_messages(messages);
                tracing::info!(key = %self.key, count = len, "skg.load_conversation");
                Ok(Some(len))
            }
        }
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
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

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
        ctx.push_message(Message::new(Role::System, Content::text("main system")));
        ctx.push_message(Message::new(Role::User, Content::text("user question")));

        ctx.run(InjectFromStore::new(
            store.clone(),
            Scope::Global,
            "mem",
            10,
        ))
        .await
        .unwrap();

        // Original system message still at position 0.
        assert_eq!(ctx.messages()[0].role, Role::System);
        assert_eq!(ctx.messages()[0].text_content(), "main system");

        // Two injected system messages at positions 1 and 2.
        assert_eq!(ctx.messages()[1].role, Role::System);
        assert_eq!(ctx.messages()[2].role, Role::System);

        // User message shifted to position 3.
        assert_eq!(ctx.messages()[3].role, Role::User);
        assert_eq!(ctx.messages().len(), 4);
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
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

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
        assert_eq!(ctx.messages().len(), 1);
    }

    #[tokio::test]
    async fn flush_extractor_receives_messages() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello world")));
        ctx.push_message(Message::new(Role::Assistant, Content::text("hi there")));

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
        ctx.push_message(Message::new(Role::System, Content::text("system")));
        ctx.push_message(Message::new(Role::User, Content::text("user msg")));

        ctx.run(
            InjectFromStore::new(store.clone(), Scope::Global, "mem", 10)
                .with_position(InjectionPosition::Append),
        )
        .await
        .unwrap();

        // Memory should be at the end
        assert_eq!(ctx.messages().len(), 3);
        assert_eq!(ctx.messages()[2].role, Role::System);
        assert!(ctx.messages()[2].text_content().contains("appended memory"));
    }

    #[tokio::test]
    async fn inject_with_custom_role_and_formatter() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("fact_1".to_string(), json!("the sky is blue"));
        }

        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        ctx.run(
            InjectFromStore::new(store.clone(), Scope::Global, "fact", 10)
                .with_role(Role::User)
                .with_formatter(|key, value| format!("Fact ({key}): {value}")),
        )
        .await
        .unwrap();

        // Should be injected at position 0 (no system message), with Role::User
        assert_eq!(ctx.messages()[0].role, Role::User);
        assert_eq!(
            ctx.messages()[0].text_content(),
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

        assert_eq!(ctx.messages()[0].meta.policy, CompactionPolicy::Pinned);
    }

    // ── P2: fetch_search_results tests ──

    #[tokio::test]
    async fn test_fetch_search_results_basic() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("key_a".to_string(), json!("value_a"));
            data.insert("key_b".to_string(), json!("value_b"));
        }

        let results = vec![
            SearchResult::new("key_a".to_string(), 1.0),
            SearchResult::new("key_b".to_string(), 0.9),
        ];

        let fetched = fetch_search_results(&*store, &Scope::Global, &results)
            .await
            .unwrap();

        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].0, "key_a");
        assert_eq!(fetched[0].1, json!("value_a"));
        assert_eq!(fetched[1].0, "key_b");
        assert_eq!(fetched[1].1, json!("value_b"));
    }

    #[tokio::test]
    async fn test_fetch_search_results_missing_key() {
        let store = MockStore::new();
        {
            let mut data = store.data.write().unwrap();
            data.insert("exists".to_string(), json!(42));
        }

        let results = vec![
            SearchResult::new("exists".to_string(), 1.0),
            SearchResult::new("gone".to_string(), 0.5),
        ];

        let fetched = fetch_search_results(&*store, &Scope::Global, &results)
            .await
            .unwrap();

        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].0, "exists");
    }

    #[tokio::test]
    async fn test_fetch_search_results_empty() {
        let store = MockStore::new();
        let results: Vec<SearchResult> = vec![];

        let fetched = fetch_search_results(&*store, &Scope::Global, &results)
            .await
            .unwrap();

        assert!(fetched.is_empty());
    }

    // ── P2: InjectSearchResults tests ──

    #[tokio::test]
    async fn test_inject_search_results_basic() {
        let results = vec![
            ("key_a".to_string(), json!("value_a")),
            ("key_b".to_string(), json!("value_b")),
        ];

        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("system prompt")));
        ctx.push_message(Message::new(Role::User, Content::text("user msg")));

        let count = ctx.run(InjectSearchResults::new(results)).await.unwrap();

        assert_eq!(count, 2);
        assert_eq!(ctx.messages().len(), 4);
        // System prompt still first.
        assert_eq!(ctx.messages()[0].text_content(), "system prompt");
        // Injected after system prompt.
        assert!(ctx.messages()[1].text_content().contains("key_a"));
        assert!(ctx.messages()[2].text_content().contains("key_b"));
        // User message shifted.
        assert_eq!(ctx.messages()[3].text_content(), "user msg");
    }

    #[tokio::test]
    async fn test_inject_search_results_empty() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        let count = ctx.run(InjectSearchResults::new(vec![])).await.unwrap();

        assert_eq!(count, 0);
        assert_eq!(ctx.messages().len(), 1);
    }

    // ── P7: Conversation persistence tests ──

    #[tokio::test]
    async fn test_save_conversation() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));
        ctx.push_message(Message::new(Role::Assistant, Content::text("hi")));

        ctx.run(SaveConversation::new(store.clone(), Scope::Global, "conv"))
            .await
            .unwrap();

        let data = store.data.read().unwrap();
        let stored = data.get("conv").unwrap();
        let arr = stored.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[tokio::test]
    async fn test_load_conversation_existing() {
        let store = MockStore::new();
        // Save messages via SaveConversation first.
        {
            let mut ctx = Context::new();
            ctx.push_message(Message::new(Role::User, Content::text("saved msg")));
            ctx.run(SaveConversation::new(store.clone(), Scope::Global, "conv"))
                .await
                .unwrap();
        }

        let mut ctx = Context::new();
        let result = ctx
            .run(LoadConversation::new(store.clone(), Scope::Global, "conv"))
            .await
            .unwrap();

        assert_eq!(result, Some(1));
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "saved msg");
    }

    #[tokio::test]
    async fn test_load_conversation_missing() {
        let store = MockStore::new();
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("original")));

        let result = ctx
            .run(LoadConversation::new(
                store.clone(),
                Scope::Global,
                "nonexistent",
            ))
            .await
            .unwrap();

        assert_eq!(result, None);
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "original");
    }

    #[tokio::test]
    async fn test_save_load_roundtrip() {
        let store = MockStore::new();

        // Save
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("sys")));
        ctx.push_message(Message::new(Role::User, Content::text("question")));
        ctx.push_message(Message::new(Role::Assistant, Content::text("answer")));
        ctx.run(SaveConversation::new(
            store.clone(),
            Scope::Global,
            "roundtrip",
        ))
        .await
        .unwrap();

        // Load into fresh context
        let mut ctx2 = Context::new();
        let result = ctx2
            .run(LoadConversation::new(
                store.clone(),
                Scope::Global,
                "roundtrip",
            ))
            .await
            .unwrap();

        assert_eq!(result, Some(3));
        assert_eq!(ctx2.messages().len(), 3);
        assert_eq!(ctx2.messages()[0].role, Role::System);
        assert_eq!(ctx2.messages()[0].text_content(), "sys");
        assert_eq!(ctx2.messages()[1].role, Role::User);
        assert_eq!(ctx2.messages()[1].text_content(), "question");
        assert_eq!(ctx2.messages()[2].role, Role::Assistant);
        assert_eq!(ctx2.messages()[2].text_content(), "answer");
    }

    #[tokio::test]
    async fn test_load_conversation_replaces() {
        let store = MockStore::new();

        // Save one message
        {
            let mut ctx = Context::new();
            ctx.push_message(Message::new(Role::User, Content::text("new msg")));
            ctx.run(SaveConversation::new(
                store.clone(),
                Scope::Global,
                "replace",
            ))
            .await
            .unwrap();
        }

        // Load into context that already has messages
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("old system")));
        ctx.push_message(Message::new(Role::User, Content::text("old user")));
        ctx.push_message(Message::new(
            Role::Assistant,
            Content::text("old assistant"),
        ));

        let result = ctx
            .run(LoadConversation::new(
                store.clone(),
                Scope::Global,
                "replace",
            ))
            .await
            .unwrap();

        assert_eq!(result, Some(1));
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "new msg");
    }
}
