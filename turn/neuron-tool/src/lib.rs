#![deny(missing_docs)]
//! Tool interface and registry for neuron.
//!
//! Defines the [`ToolDyn`] trait for object-safe tool abstraction and
//! [`ToolRegistry`] for managing collections of tools. Any tool source
//! (local function, MCP server, HTTP endpoint) implements [`ToolDyn`].

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;

/// Errors from tool operations.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ToolError {
    /// The requested tool was not found in the registry.
    #[error("tool not found: {0}")]
    NotFound(String),

    /// Tool execution failed.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    /// The input provided to the tool was invalid.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Concurrency hint for tool scheduling.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToolConcurrencyHint {
    /// Safe to run alongside other shared tools in the same batch.
    Shared,
    /// Must run alone (barrier before and after).
    #[default]
    Exclusive,
}

/// Optional streaming interface for tools.
pub trait ToolDynStreaming: Send + Sync + 'static + ToolDyn {
    /// Execute the tool with streaming chunk updates.
    fn call_streaming<'a>(
        &'a self,
        input: serde_json::Value,
        on_chunk: Box<dyn Fn(&str) + Send + Sync + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + Send + 'a>>;
}
/// Object-safe trait for tool implementations.
///
/// Any tool source (local function, MCP server, HTTP endpoint) implements
/// this trait. Tools are stored as `Arc<dyn ToolDyn>` in [`ToolRegistry`].
pub trait ToolDyn: Send + Sync {
    /// The tool's unique name.
    fn name(&self) -> &str;

    /// Human-readable description of what the tool does.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given input.
    fn call(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>;

    /// If this tool also supports streaming, return a reference to its streaming interface.
    /// Default is None; streaming is opt-in and non-disruptive.
    fn maybe_streaming(&self) -> Option<&dyn ToolDynStreaming> {
        None
    }

    /// Optional concurrency hint used by planners/deciders.
    ///
    /// Default is Exclusive to preserve backward-compatible behavior.
    fn concurrency_hint(&self) -> ToolConcurrencyHint {
        ToolConcurrencyHint::Exclusive
    }
}

/// A tool wrapper that exposes a different name while delegating behavior to an inner tool.
///
/// This is useful when importing tools from external systems (e.g. MCP servers) where the
/// upstream tool names are not stable or do not match the caller's desired naming scheme.
pub struct AliasedTool {
    alias: String,
    inner: Arc<dyn ToolDyn>,
}

impl AliasedTool {
    /// Create a new aliased tool wrapper.
    pub fn new(alias: impl Into<String>, inner: Arc<dyn ToolDyn>) -> Self {
        Self {
            alias: alias.into(),
            inner,
        }
    }

    /// Access the wrapped tool.
    pub fn inner(&self) -> &Arc<dyn ToolDyn> {
        &self.inner
    }
}

impl ToolDyn for AliasedTool {
    fn name(&self) -> &str {
        &self.alias
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn input_schema(&self) -> serde_json::Value {
        self.inner.input_schema()
    }

    fn call(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        self.inner.call(input)
    }

    fn concurrency_hint(&self) -> ToolConcurrencyHint {
        self.inner.concurrency_hint()
    }
}

/// Registry of tools available to a turn.
///
/// Holds tools as `Arc<dyn ToolDyn>` keyed by name. The turn's ReAct loop
/// uses this to look up and execute tools requested by the model.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolDyn>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites any existing tool with the same name.
    pub fn register(&mut self, tool: Arc<dyn ToolDyn>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn ToolDyn>> {
        self.tools.get(name)
    }

    /// Iterate over all registered tools.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn ToolDyn>> {
        self.tools.values()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn tool_dyn_is_object_safe() {
        _assert_send_sync::<Arc<dyn ToolDyn>>();
    }

    #[test]
    fn tool_error_display() {
        assert_eq!(
            ToolError::NotFound("bash".into()).to_string(),
            "tool not found: bash"
        );
        assert_eq!(
            ToolError::ExecutionFailed("timeout".into()).to_string(),
            "execution failed: timeout"
        );
        assert_eq!(
            ToolError::InvalidInput("missing field".into()).to_string(),
            "invalid input: missing field"
        );
    }

    struct EchoTool;

    impl ToolDyn for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input back"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
    }

    struct FailTool;

    impl ToolDyn for FailTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async { Err(ToolError::ExecutionFailed("always fails".into())) })
        }
    }

    #[test]
    fn registry_add_and_get() {
        let mut reg = ToolRegistry::new();
        assert!(reg.is_empty());

        reg.register(Arc::new(EchoTool));
        assert_eq!(reg.len(), 1);
        assert!(reg.get("echo").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn registry_iter() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        reg.register(Arc::new(FailTool));

        let names: Vec<&str> = reg.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"echo"));
        assert!(names.contains(&"fail"));
    }

    #[tokio::test]
    async fn registry_call_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));

        let tool = reg.get("echo").unwrap();
        let result = tool.call(json!({"msg": "hello"})).await.unwrap();
        assert_eq!(result, json!({"echoed": {"msg": "hello"}}));
    }

    #[tokio::test]
    async fn aliased_tool_exposes_alias_name_and_delegates() {
        let inner: Arc<dyn ToolDyn> = Arc::new(EchoTool);
        let tool: Arc<dyn ToolDyn> = Arc::new(AliasedTool::new("echo_alias", Arc::clone(&inner)));

        assert_eq!(tool.name(), "echo_alias");
        assert_eq!(tool.description(), inner.description());

        let result = tool.call(json!({"msg": "hi"})).await.unwrap();
        assert_eq!(result, json!({"echoed": {"msg": "hi"}}));
    }

    #[tokio::test]
    async fn registry_call_failing_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FailTool));

        let tool = reg.get("fail").unwrap();
        let result = tool.call(json!({})).await;
        assert!(result.is_err());
    }

    #[test]
    fn registry_overwrite() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        assert_eq!(reg.len(), 1);

        // Register another tool with the same name
        reg.register(Arc::new(EchoTool));
        assert_eq!(reg.len(), 1);
    }

    struct StreamerTool;
    impl ToolDyn for StreamerTool {
        fn name(&self) -> &str {
            "streamer"
        }
        fn description(&self) -> &str {
            "Streams chunks"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type":"object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async { Ok(serde_json::json!({"status":"done"})) })
        }
        fn maybe_streaming(&self) -> Option<&dyn ToolDynStreaming> {
            Some(self)
        }
    }
    impl ToolDynStreaming for StreamerTool {
        fn call_streaming<'a>(
            &'a self,
            _input: serde_json::Value,
            on_chunk: Box<dyn Fn(&str) + Send + Sync + 'a>,
        ) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + Send + 'a>> {
            Box::pin(async move {
                on_chunk("one");
                on_chunk("two");
                on_chunk("three");
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn streaming_tool_emits_chunks_and_completes() {
        use std::sync::{
            Arc as StdArc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let count = StdArc::new(AtomicUsize::new(0));
        let seen: StdArc<Mutex<Vec<String>>> = StdArc::new(Mutex::new(vec![]));
        let c2 = count.clone();
        let s2 = seen.clone();
        let tool = StreamerTool;
        let on_chunk = Box::new(move |c: &str| {
            c2.fetch_add(1, Ordering::SeqCst);
            s2.lock().unwrap().push(c.to_string());
        });
        let res = tool.call_streaming(serde_json::json!({}), on_chunk).await;
        assert!(res.is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 3);
        let got = seen.lock().unwrap().clone();
        assert_eq!(got, vec!["one", "two", "three"]);
    }
}
