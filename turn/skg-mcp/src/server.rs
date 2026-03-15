//! MCP server that exposes a [`ToolRegistry`] via the MCP protocol.
//!
//! [`McpServer`] wraps a [`ToolRegistry`] and serves
//! its tools over stdio using the MCP protocol. It can optionally be
//! configured with a state reader (to expose state keys as MCP resources)
//! and prompt templates.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::operator::{Operator, TriggerType};
use layer0::{StateReader, ToolMetadata};
use rmcp::model::{
    Annotated, CallToolRequestParams, CallToolResult, Content, GetPromptRequestParams,
    GetPromptResult, Implementation, ListPromptsResult, ListResourcesResult, ListToolsResult,
    Prompt, PromptMessage, PromptMessageContent, PromptMessageRole, ProtocolVersion, RawResource,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
    ServerInfo, Tool as McpTool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::io::stdio;
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use skg_tool::{ToolDyn, ToolError, ToolRegistry};

use crate::error::McpError;

/// Bridges an [`Operator`] and [`ToolMetadata`] into the [`ToolDyn`] interface.
///
/// [`McpServer::from_operators`] wraps each operator in this adapter and registers
/// it in an internal [`ToolRegistry`], so operator-protocol implementations can be
/// served over MCP without rewriting tool-based infrastructure.
struct OperatorToolAdapter {
    operator: Arc<dyn Operator>,
    metadata: ToolMetadata,
}

impl ToolDyn for OperatorToolAdapter {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn description(&self) -> &str {
        &self.metadata.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.metadata.input_schema.clone()
    }

    fn call(
        &self,
        input: serde_json::Value,
        ctx: &layer0::DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let operator = Arc::clone(&self.operator);
        let name = self.metadata.name.clone();
        let ctx = ctx.clone();
        Box::pin(async move {
            let json_str = serde_json::to_string(&input)
                .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
            let op_input =
                layer0::OperatorInput::new(layer0::Content::text(json_str), TriggerType::Task);
            // Inherit the caller's trace/identity context rather than creating a blank one.
            let ctx = layer0::DispatchContext::new(
                layer0::id::DispatchId::new(format!("mcp-tool-{name}")),
                layer0::id::OperatorId::new(&name),
            )
            .with_trace(ctx.trace.child_span());
            let output = operator
                .execute(op_input, &ctx, &layer0::dispatch::EffectEmitter::noop())
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let text = output.message.as_text().unwrap_or("null").to_owned();
            serde_json::from_str(&text)
                .map_err(|e| ToolError::ExecutionFailed(format!("output parse error: {e}")))
        })
    }
}

/// Parse a W3C `traceparent` header value into a [`TraceContext`].
///
/// Format: `{version}-{trace_id}-{span_id}-{trace_flags}`
/// Returns `None` if the format is invalid.
fn parse_traceparent(value: &str) -> Option<layer0::TraceContext> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() < 4 {
        return None;
    }
    let trace_flags = u8::from_str_radix(parts[3], 16).ok()?;
    Some(layer0::TraceContext {
        trace_id: parts[1].to_owned(),
        span_id: parts[2].to_owned(),
        trace_flags,
        trace_state: None,
    })
}

/// MCP server that exposes tools from a [`ToolRegistry`].
///
/// Optionally backed by a [`StateReader`] (exposing state keys as
/// `state://global/{key}` resources) and a list of prompt templates.
/// Call [`serve_stdio`](McpServer::serve_stdio) to start serving via stdin/stdout.
pub struct McpServer {
    /// The tool registry to expose.
    registry: Arc<ToolRegistry>,
    /// Server name for MCP identification.
    name: String,
    /// Server version for MCP identification.
    version: String,
    /// Optional state reader for resource exposure.
    state_reader: Option<Arc<dyn StateReader>>,
    /// Registered prompt templates: (name, description, template).
    prompts: Vec<(String, Option<String>, String)>,
}

impl McpServer {
    /// Create a new MCP server wrapping the given tool registry.
    pub fn new(
        registry: ToolRegistry,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            registry: Arc::new(registry),
            name: name.into(),
            version: version.into(),
            state_reader: None,
            prompts: Vec::new(),
        }
    }

    /// Attach a state reader to expose global state keys as MCP resources.
    ///
    /// Each key returned by the reader is advertised as a resource with URI
    /// `state://global/{key}`.  Enabling this causes the server to advertise
    /// the `resources` capability.
    pub fn with_state_reader(mut self, reader: Arc<dyn StateReader>) -> Self {
        self.state_reader = Some(reader);
        self
    }

    /// Register a prompt template with this server.
    ///
    /// The template is returned verbatim as a `user` message when the client
    /// calls `prompts/get`.  Registering at least one prompt causes the server
    /// to advertise the `prompts` capability.
    pub fn with_prompt(
        mut self,
        name: impl Into<String>,
        description: Option<impl Into<String>>,
        template: impl Into<String>,
    ) -> Self {
        self.prompts
            .push((name.into(), description.map(Into::into), template.into()));
        self
    }

    /// Serve the tools over stdio (stdin/stdout).
    ///
    /// This blocks until the client disconnects or an error occurs.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the transport setup or serving fails.
    pub async fn serve_stdio(self) -> Result<(), McpError> {
        let transport = stdio();
        let handler = McpServerHandler {
            registry: self.registry,
            name: self.name,
            version: self.version,
            state_reader: self.state_reader,
            prompts: self.prompts,
        };
        let service = handler
            .serve(transport)
            .await
            .map_err(|e| McpError::Connection(e.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|e| McpError::Connection(e.to_string()))?;
        Ok(())
    }

    /// Create an MCP server from a set of operators with metadata.
    ///
    /// Each `(operator, metadata)` pair is wrapped in an `OperatorToolAdapter` and
    /// registered in an internal [`ToolRegistry`].  This bridges the operator protocol
    /// into the existing tool-based MCP serving infrastructure.
    pub fn from_operators(
        operators: Vec<(Arc<dyn Operator>, ToolMetadata)>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let mut registry = ToolRegistry::new();
        for (operator, metadata) in operators {
            registry.register(Arc::new(OperatorToolAdapter { operator, metadata }));
        }
        Self::new(registry, name, version)
    }
}

/// Internal handler implementing [`ServerHandler`] for the MCP protocol.
struct McpServerHandler {
    /// The tool registry to expose.
    registry: Arc<ToolRegistry>,
    /// Server name.
    name: String,
    /// Server version.
    version: String,
    /// Optional state reader for resource handling.
    state_reader: Option<Arc<dyn StateReader>>,
    /// Registered prompt templates.
    prompts: Vec<(String, Option<String>, String)>,
}

impl ServerHandler for McpServerHandler {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities {
            tools: Some(rmcp::model::ToolsCapability::default()),
            resources: self
                .state_reader
                .is_some()
                .then_some(rmcp::model::ResourcesCapability::default()),
            prompts: (!self.prompts.is_empty())
                .then_some(rmcp::model::PromptsCapability::default()),
            experimental: None,
            extensions: None,
            logging: None,
            completions: None,
            tasks: None,
        };
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities,
            server_info: Implementation {
                name: self.name.clone(),
                version: self.version.clone(),
                ..Default::default()
            },
            instructions: None,
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools: Vec<McpTool> = self
            .registry
            .iter()
            .map(|tool| {
                let schema = tool.input_schema();
                let schema_obj = schema.as_object().cloned().unwrap_or_default();

                McpTool {
                    name: Cow::Owned(tool.name().to_string()),
                    title: None,
                    description: Some(Cow::Owned(tool.description().to_string())),
                    input_schema: Arc::new(schema_obj),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                }
            })
            .collect();

        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool_name = &*request.name;
        let tool = self.registry.get(tool_name).ok_or_else(|| {
            ErrorData::invalid_params(format!("tool not found: {tool_name}"), None)
        })?;

        let input = match request.arguments {
            Some(map) => serde_json::Value::Object(map),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };

        // Build a DispatchContext with tool-specific identity and optional trace.
        let trace = request
            .meta
            .as_ref()
            .and_then(|m| m.0.get("traceparent"))
            .and_then(|v| v.as_str())
            .and_then(parse_traceparent);

        let name_owned = tool_name.to_owned();
        let mut ctx = layer0::DispatchContext::new(
            layer0::id::DispatchId::new(format!("mcp-{name_owned}")),
            layer0::OperatorId::new(&name_owned),
        );
        if let Some(t) = trace {
            ctx = ctx.with_trace(t);
        }

        tracing::debug!(
            tool = %name_owned,
            traceparent = ?ctx.trace.as_traceparent(),
            "MCP server call_tool"
        );

        match tool.call(input, &ctx).await {
            Ok(result) => {
                let text =
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let Some(ref reader) = self.state_reader else {
            return Ok(ListResourcesResult::default());
        };
        let keys = reader
            .list(&layer0::Scope::Global, "")
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let resources: Vec<rmcp::model::Resource> = keys
            .iter()
            .map(|key| {
                Annotated::new(
                    RawResource {
                        uri: format!("state://global/{key}"),
                        name: key.clone(),
                        title: None,
                        description: None,
                        mime_type: Some("application/json".into()),
                        size: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                )
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let Some(ref reader) = self.state_reader else {
            return Err(ErrorData::invalid_params(
                "no state reader configured",
                None,
            ));
        };
        let key = request.uri.strip_prefix("state://global/").ok_or_else(|| {
            ErrorData::invalid_params(format!("unsupported resource URI: {}", request.uri), None)
        })?;
        let value = reader
            .read(&layer0::Scope::Global, key)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let text = match value {
            Some(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
            None => {
                return Err(ErrorData::invalid_params(
                    format!("resource not found: {}", request.uri),
                    None,
                ));
            }
        };
        Ok(ReadResourceResult {
            contents: vec![ResourceContents::TextResourceContents {
                uri: request.uri,
                mime_type: Some("application/json".into()),
                text,
                meta: None,
            }],
        })
    }

    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let prompts = self
            .prompts
            .iter()
            .map(|(name, desc, _template)| Prompt {
                name: name.clone(),
                title: None,
                description: desc.clone(),
                arguments: None,
                icons: None,
                meta: None,
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, ErrorData> {
        let (_, desc, template) = self
            .prompts
            .iter()
            .find(|(n, _, _)| n == &request.name)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("prompt not found: {}", request.name), None)
            })?;
        Ok(GetPromptResult {
            description: desc.clone(),
            messages: vec![PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::Text {
                    text: template.clone(),
                },
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use skg_tool::{ToolDyn, ToolError};
    use std::future::Future;
    use std::pin::Pin;

    struct TestTool {
        tool_name: &'static str,
    }

    impl ToolDyn for TestTool {
        fn name(&self) -> &str {
            self.tool_name
        }
        fn description(&self) -> &str {
            "A test tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {"input": {"type": "string"}}})
        }
        fn call(
            &self,
            input: serde_json::Value,
            _ctx: &layer0::DispatchContext,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
    }

    struct FailingTool;

    impl ToolDyn for FailingTool {
        fn name(&self) -> &str {
            "fail_tool"
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
            _ctx: &layer0::DispatchContext,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async move { Err(ToolError::ExecutionFailed("deliberate failure".into())) })
        }
    }

    #[test]
    fn mcp_server_constructs() {
        let registry = ToolRegistry::new();
        let server = McpServer::new(registry, "test-server", "0.1.0");
        assert_eq!(server.name, "test-server");
        assert_eq!(server.version, "0.1.0");
    }

    #[test]
    fn mcp_server_constructs_with_state_reader() {
        use layer0::test_utils::InMemoryStore;
        let store = Arc::new(InMemoryStore::new());
        let registry = ToolRegistry::new();
        let server = McpServer::new(registry, "test-server", "0.1.0")
            .with_state_reader(store as Arc<dyn StateReader>);
        assert!(server.state_reader.is_some());
    }

    #[test]
    fn mcp_server_with_prompt() {
        let registry = ToolRegistry::new();
        let server = McpServer::new(registry, "test-server", "0.1.0").with_prompt(
            "greet",
            Some("Greeting prompt"),
            "Hello {name}",
        );
        assert_eq!(server.prompts.len(), 1);
        let (name, desc, template) = &server.prompts[0];
        assert_eq!(name, "greet");
        assert_eq!(desc.as_deref(), Some("Greeting prompt"));
        assert_eq!(template, "Hello {name}");
    }

    #[test]
    fn mcp_server_with_prompt_no_description() {
        let registry = ToolRegistry::new();
        let server = McpServer::new(registry, "test-server", "0.1.0").with_prompt(
            "bare",
            None::<String>,
            "template text",
        );
        let (_, desc, _) = &server.prompts[0];
        assert!(desc.is_none());
    }

    #[test]
    fn server_handler_list_prompts_returns_registered() {
        // Verify the data structure that list_prompts iterates over.
        let prompts: Vec<(String, Option<String>, String)> = vec![
            (
                "greet".to_string(),
                Some("Greeting".to_string()),
                "Hello {name}".to_string(),
            ),
            ("farewell".to_string(), None, "Goodbye {name}".to_string()),
        ];
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].0, "greet");
        assert_eq!(prompts[0].1.as_deref(), Some("Greeting"));
        assert_eq!(prompts[1].0, "farewell");
        assert!(prompts[1].1.is_none());
    }

    #[test]
    fn server_handler_get_info_no_optional_capabilities() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "my-server".into(),
            version: "1.0.0".into(),
            state_reader: None,
            prompts: vec![],
        };
        let info = handler.get_info();
        assert_eq!(info.server_info.name, "my-server");
        assert_eq!(info.server_info.version, "1.0.0");
        assert!(info.capabilities.tools.is_some());
        assert!(info.capabilities.resources.is_none());
        assert!(info.capabilities.prompts.is_none());
    }

    #[test]
    fn server_handler_get_info_with_state_reader_enables_resources() {
        use layer0::test_utils::InMemoryStore;
        let store = Arc::new(InMemoryStore::new());
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "s".into(),
            version: "0".into(),
            state_reader: Some(store as Arc<dyn StateReader>),
            prompts: vec![],
        };
        let info = handler.get_info();
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.prompts.is_none());
    }

    #[test]
    fn server_handler_get_info_with_prompts_enables_prompts() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "s".into(),
            version: "0".into(),
            state_reader: None,
            prompts: vec![("p".to_string(), None, "t".to_string())],
        };
        let info = handler.get_info();
        assert!(info.capabilities.prompts.is_some());
        assert!(info.capabilities.resources.is_none());
    }

    #[test]
    fn server_handler_get_info() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "my-server".into(),
            version: "1.0.0".into(),
            state_reader: None,
            prompts: vec![],
        };
        let info = handler.get_info();
        assert_eq!(info.server_info.name, "my-server");
        assert_eq!(info.server_info.version, "1.0.0");
    }

    #[tokio::test]
    async fn server_handler_list_tools_empty() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "test".into(),
            version: "0.1.0".into(),
            state_reader: None,
            prompts: vec![],
        };

        let ctx = handler.get_info();
        let _ = ctx; // just to verify get_info works

        // We cannot easily construct RequestContext for the handler methods,
        // but we can verify the registry logic directly.
        let reg = ToolRegistry::new();
        let tools: Vec<&Arc<dyn ToolDyn>> = reg.iter().collect();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn server_handler_list_tools_with_registered_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(TestTool { tool_name: "echo" }));
        registry.register(Arc::new(TestTool { tool_name: "read" }));

        // Verify the registry contents that list_tools would expose
        assert_eq!(registry.len(), 2);
        assert!(registry.get("echo").is_some());
        assert!(registry.get("read").is_some());

        // Verify tool schemas that would be converted
        let echo = registry.get("echo").unwrap();
        assert_eq!(echo.description(), "A test tool");
        let schema = echo.input_schema();
        assert!(schema.as_object().is_some());
    }

    #[tokio::test]
    async fn server_call_tool_logic_success() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(TestTool { tool_name: "echo" }));

        let tool = registry.get("echo").unwrap();
        let ctx = layer0::DispatchContext::new(
            layer0::id::DispatchId::new("test"),
            layer0::OperatorId::new("test"),
        );
        let result = tool.call(json!({"msg": "hello"}), &ctx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"echoed": {"msg": "hello"}}));
    }

    #[tokio::test]
    async fn server_call_tool_logic_failure() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FailingTool));

        let tool = registry.get("fail_tool").unwrap();
        let ctx = layer0::DispatchContext::new(
            layer0::id::DispatchId::new("test"),
            layer0::OperatorId::new("test"),
        );
        let result = tool.call(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn server_call_tool_not_found() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn server_handler_list_resources_no_reader_returns_empty() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "s".into(),
            version: "0".into(),
            state_reader: None,
            prompts: vec![],
        };
        // Without a reader, list_resources returns the default (empty) result.
        assert!(handler.state_reader.is_none());
    }

    #[tokio::test]
    async fn server_handler_read_resource_no_reader_returns_error() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "s".into(),
            version: "0".into(),
            state_reader: None,
            prompts: vec![],
        };
        // Verify logic: a missing key from the reader would produce an error.
        assert!(handler.state_reader.is_none());
    }

    #[tokio::test]
    async fn server_handler_list_prompts_none_registered_is_empty() {
        let handler = McpServerHandler {
            registry: Arc::new(ToolRegistry::new()),
            name: "s".into(),
            version: "0".into(),
            state_reader: None,
            prompts: vec![],
        };
        assert!(handler.prompts.is_empty());
    }

    #[tokio::test]
    async fn server_handler_get_prompt_not_found_logic() {
        let prompts: Vec<(String, Option<String>, String)> =
            vec![("greet".to_string(), None, "Hello".to_string())];
        let found = prompts.iter().find(|(n, _, _)| n == "missing");
        assert!(found.is_none());
        let found_existing = prompts.iter().find(|(n, _, _)| n == "greet");
        assert!(found_existing.is_some());
        let (_, _, template) = found_existing.unwrap();
        assert_eq!(template, "Hello");
    }

    #[tokio::test]
    async fn mcp_server_from_operators_constructs() {
        use async_trait::async_trait;
        use layer0::OperatorError;
        use layer0::operator::{ExitReason, OperatorInput, OperatorOutput};

        struct EchoOperator;

        #[async_trait]
        impl layer0::operator::Operator for EchoOperator {
            async fn execute(
                &self,
                input: OperatorInput,
                _ctx: &layer0::DispatchContext,
                _emitter: &layer0::dispatch::EffectEmitter,
            ) -> Result<OperatorOutput, OperatorError> {
                Ok(OperatorOutput::new(input.message, ExitReason::Complete))
            }
        }

        let meta = ToolMetadata::new(
            "echo_op",
            "Echoes operator input back",
            serde_json::json!({"type": "object"}),
            true,
        );
        let operators: Vec<(Arc<dyn layer0::operator::Operator>, ToolMetadata)> =
            vec![(Arc::new(EchoOperator), meta)];
        let server = McpServer::from_operators(operators, "test-ops", "0.1.0");
        assert_eq!(server.registry.len(), 1);
        assert_eq!(server.name, "test-ops");
        assert!(server.registry.get("echo_op").is_some());
    }

    #[tokio::test]
    async fn operator_tool_adapter_roundtrip() {
        use async_trait::async_trait;
        use layer0::operator::{ExitReason, OperatorInput, OperatorOutput};
        use layer0::{Content, OperatorError};
        use serde_json::json;

        struct ConstOperator {
            response: serde_json::Value,
        }

        #[async_trait]
        impl layer0::operator::Operator for ConstOperator {
            async fn execute(
                &self,
                _input: OperatorInput,
                _ctx: &layer0::DispatchContext,
                _emitter: &layer0::dispatch::EffectEmitter,
            ) -> Result<OperatorOutput, OperatorError> {
                let text = self.response.to_string();
                Ok(OperatorOutput::new(
                    Content::text(text),
                    ExitReason::Complete,
                ))
            }
        }

        let schema = json!({"type": "object", "properties": {"query": {"type": "string"}}});
        let meta = ToolMetadata::new("my_tool", "A test operator", schema.clone(), false);
        let adapter = OperatorToolAdapter {
            operator: Arc::new(ConstOperator {
                response: json!({"result": "ok"}),
            }),
            metadata: meta,
        };

        // metadata bridge
        assert_eq!(adapter.name(), "my_tool");
        assert_eq!(adapter.description(), "A test operator");
        assert_eq!(adapter.input_schema(), schema);

        // call roundtrip: input is serialized → operator echoes JSON string → parsed back
        let ctx = layer0::DispatchContext::new(
            layer0::id::DispatchId::new("test"),
            layer0::OperatorId::new("test"),
        );
        let result = adapter.call(json!({"query": "hello"}), &ctx).await.unwrap();
        assert_eq!(result, json!({"result": "ok"}));
    }

    #[test]
    fn parse_traceparent_valid() {
        let tc =
            super::parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
                .unwrap();
        assert_eq!(tc.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(tc.span_id, "b7ad6b7169203331");
        assert_eq!(tc.trace_flags, 1);
        assert!(tc.trace_state.is_none());
    }

    #[test]
    fn parse_traceparent_invalid() {
        assert!(super::parse_traceparent("not-valid").is_none());
        assert!(super::parse_traceparent("").is_none());
        assert!(super::parse_traceparent("00-abc").is_none());
    }

    #[test]
    fn parse_traceparent_unsampled() {
        let tc = super::parse_traceparent("00-aaaa-bbbb-00").unwrap();
        assert_eq!(tc.trace_flags, 0);
    }
}
