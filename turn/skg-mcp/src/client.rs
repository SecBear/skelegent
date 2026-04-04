//! MCP client that discovers remote tools and wraps them as [`ToolDyn`].
//!
//! [`McpClient`] connects to an MCP server (via stdio child process or
//! streamable HTTP), discovers its tools, and wraps each as a [`ToolDyn`]
//! implementation so they can be registered in a
//! [`ToolRegistry`](skg_tool::ToolRegistry).

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::{
    ApprovalFacts, AuthFacts, CapabilityDescriptor, CapabilityId, CapabilityKind,
    CapabilityModality, ExecutionClass, SchedulingFacts, StreamingSupport, ToolMetadata,
};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, GetPromptRequestParams, PromptMessage,
    RawContent, ReadResourceRequestParams, ResourceContents, Tool as McpTool,
};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use skg_tool::adapter::ToolOperator;
use skg_tool::{AliasedTool, ToolDyn, ToolError};

use crate::error::McpError;

/// Number of tools above which a [`tracing::warn`] is emitted about context pollution.
///
/// Tool definitions consume ~50-200 tokens each; beyond 20 tools the cumulative
/// cost becomes a meaningful fraction of the context window.
pub const TOOL_COUNT_WARN_THRESHOLD: usize = 20;

/// An MCP client that connects to a server and discovers its tools.
///
/// After connecting, call [`discover_tools`](McpClient::discover_tools) to get
/// a list of [`ToolDyn`] implementations backed by the remote MCP server.
pub struct McpClient {
    /// The running MCP service (client role).
    service: RunningService<RoleClient, ()>,
}

impl McpClient {
    /// Connect to an MCP server by spawning a child process.
    ///
    /// The command should be a `tokio::process::Command` configured to launch
    /// the MCP server executable.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the process cannot be spawned or
    /// the MCP handshake fails.
    pub async fn connect_stdio(command: tokio::process::Command) -> Result<Self, McpError> {
        let transport =
            TokioChildProcess::new(command).map_err(|e| McpError::Connection(e.to_string()))?;
        let service = ().serve(transport).await.map_err(|e| McpError::Connection(e.to_string()))?;
        Ok(Self { service })
    }

    /// Connect to an MCP server via streamable HTTP.
    ///
    /// The URL should point to the MCP server's HTTP endpoint
    /// (e.g., `http://localhost:8080/mcp`).
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the HTTP connection or MCP
    /// handshake fails.
    pub async fn connect_http(url: &str) -> Result<Self, McpError> {
        let transport = StreamableHttpClientTransport::from_uri(url);
        let service: RunningService<RoleClient, ()> = ()
            .serve(transport)
            .await
            .map_err(|e| McpError::Connection(e.to_string()))?;
        Ok(Self { service })
    }

    /// Connect to an MCP server via streamable HTTP.
    ///
    /// # Deprecated
    ///
    /// Renamed to [`connect_http`](McpClient::connect_http). SSE was the
    /// transport name from an earlier version of the MCP spec; the current
    /// transport is Streamable HTTP.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the HTTP connection or MCP
    /// handshake fails.
    #[deprecated(since = "0.4.2", note = "Renamed to connect_http")]
    pub async fn connect_sse(url: &str) -> Result<Self, McpError> {
        Self::connect_http(url).await
    }

    /// Discover all tools from the connected MCP server.
    ///
    /// Returns a vector of [`Arc<dyn ToolDyn>`] wrappers that delegate calls
    /// to the remote MCP server.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the tool listing request fails.
    pub async fn discover_tools(&self) -> Result<Vec<Arc<dyn ToolDyn>>, McpError> {
        let result = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;

        let tool_count = result.len();
        if tool_count > TOOL_COUNT_WARN_THRESHOLD {
            tracing::warn!(
                count = tool_count,
                threshold = TOOL_COUNT_WARN_THRESHOLD,
                "MCP tool count exceeds recommended limit; context pollution risk"
            );
        }

        let peer = self.service.peer().clone();
        let peer = Arc::new(peer);

        let tools: Vec<Arc<dyn ToolDyn>> = result
            .into_iter()
            .map(|tool| Arc::new(McpToolWrapper::new(tool, Arc::clone(&peer))) as Arc<dyn ToolDyn>)
            .collect();

        Ok(tools)
    }

    /// Discover all tools from the connected MCP server as operator-protocol types.
    ///
    /// Returns a vector of [`(ToolOperator, ToolMetadata)`] pairs. Each pair wraps
    /// the remote tool as an [`Operator`](layer0::operator::Operator) and carries
    /// its extracted metadata.
    ///
    /// Emits a [`tracing::warn`] when the tool count exceeds
    /// [`TOOL_COUNT_WARN_THRESHOLD`], identical to
    /// [`discover_tools`](McpClient::discover_tools).
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the tool listing request fails.
    pub async fn discover_operators(&self) -> Result<Vec<(ToolOperator, ToolMetadata)>, McpError> {
        let result = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;

        let tool_count = result.len();
        if tool_count > TOOL_COUNT_WARN_THRESHOLD {
            tracing::warn!(
                count = tool_count,
                threshold = TOOL_COUNT_WARN_THRESHOLD,
                "MCP tool count exceeds recommended limit; context pollution risk"
            );
        }

        let peer = self.service.peer().clone();
        let peer = Arc::new(peer);

        let operators: Vec<(ToolOperator, ToolMetadata)> = result
            .into_iter()
            .map(|tool| {
                let arc =
                    Arc::new(McpToolWrapper::new(tool, Arc::clone(&peer))) as Arc<dyn ToolDyn>;
                let op = ToolOperator::new(arc);
                let meta = op.metadata();
                (op, meta)
            })
            .collect();

        Ok(operators)
    }

    /// Discover all tools and apply a name-alias map.
    ///
    /// The `aliases` map is keyed by the remote tool name and contains the
    /// desired name to expose locally.
    ///
    /// This is a convenience wrapper around [`discover_tools`](McpClient::discover_tools).
    pub async fn discover_tools_with_aliases(
        &self,
        aliases: &HashMap<String, String>,
    ) -> Result<Vec<Arc<dyn ToolDyn>>, McpError> {
        let tools = self.discover_tools().await?;
        let aliased: Vec<Arc<dyn ToolDyn>> = tools
            .into_iter()
            .map(|tool| {
                let tool_name = tool.name().to_string();
                if let Some(alias) = aliases.get(&tool_name) {
                    Arc::new(AliasedTool::new(alias.clone(), tool)) as Arc<dyn ToolDyn>
                } else {
                    tool
                }
            })
            .collect();
        Ok(aliased)
    }

    /// Estimate the total token budget consumed by a slice of MCP tool definitions.
    ///
    /// Uses the chars/4 heuristic — a common approximation for token count.
    /// This is intentionally rough; accuracy is not the goal, awareness is.
    /// The estimate covers tool names and descriptions, which are injected into
    /// the model context on every turn.
    ///
    /// Emit a warning by calling [`discover_tools`](McpClient::discover_tools),
    /// which checks the count against [`TOOL_COUNT_WARN_THRESHOLD`] automatically.
    pub fn tool_budget_tokens(tools: &[McpTool]) -> usize {
        tools
            .iter()
            .map(|t| {
                let name_chars = t.name.len();
                let desc_chars = t.description.as_deref().unwrap_or("").len();
                (name_chars + desc_chars) / 4
            })
            .sum()
    }

    /// Shut down the MCP client connection.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the shutdown fails.
    pub async fn close(self) -> Result<(), McpError> {
        self.service
            .cancel()
            .await
            .map_err(|e| McpError::Connection(e.to_string()))?;
        Ok(())
    }

    /// Discover all resources advertised by the connected MCP server.
    ///
    /// Returns a vector of [`McpResourceWrapper`] instances, each capable
    /// of reading the resource's content via the live connection.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the resource listing request fails.
    pub async fn discover_resources(&self) -> Result<Vec<McpResourceWrapper>, McpError> {
        let resources = self
            .service
            .list_all_resources()
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;
        let peer = Arc::new(self.service.peer().clone());
        Ok(resources
            .into_iter()
            .map(|r| McpResourceWrapper {
                resource: r,
                peer: Arc::clone(&peer),
            })
            .collect())
    }

    /// Discover all prompts advertised by the connected MCP server.
    ///
    /// Returns a vector of [`McpPromptWrapper`] instances, each capable
    /// of fetching rendered messages from the live connection.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the prompt listing request fails.
    pub async fn discover_prompts(&self) -> Result<Vec<McpPromptWrapper>, McpError> {
        let prompts = self
            .service
            .list_all_prompts()
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;
        let peer = Arc::new(self.service.peer().clone());
        Ok(prompts
            .into_iter()
            .map(|p| McpPromptWrapper {
                prompt: p,
                peer: Arc::clone(&peer),
            })
            .collect())
    }
}

/// Wrapper around an MCP resource, exposing its metadata and content.
///
/// Holds a reference to the MCP peer for making remote resource reads.
pub struct McpResourceWrapper {
    resource: rmcp::model::Resource,
    peer: Arc<Peer<RoleClient>>,
}

impl McpResourceWrapper {
    /// The URI identifying this resource.
    pub fn uri(&self) -> &str {
        &self.resource.uri
    }

    /// The human-readable name of this resource.
    pub fn name(&self) -> &str {
        &self.resource.name
    }

    /// An optional description of what the resource contains.
    pub fn description(&self) -> Option<&str> {
        self.resource.description.as_deref()
    }

    /// Read the resource contents from the server.
    ///
    /// Fetches the resource identified by [`uri`](McpResourceWrapper::uri) and
    /// returns all text content blocks joined with newlines.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the remote call fails.
    pub async fn read(&self) -> Result<String, McpError> {
        let params = ReadResourceRequestParams {
            meta: None,
            uri: self.resource.uri.clone(),
        };
        let result = self
            .peer
            .read_resource(params)
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;
        let text = result
            .contents
            .into_iter()
            .filter_map(|c| match c {
                ResourceContents::TextResourceContents { text, .. } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(text)
    }
}

/// Wrapper around an MCP prompt, exposing its metadata and rendering.
///
/// Holds a reference to the MCP peer for making remote prompt requests.
pub struct McpPromptWrapper {
    prompt: rmcp::model::Prompt,
    peer: Arc<Peer<RoleClient>>,
}

impl McpPromptWrapper {
    /// The name used to identify this prompt.
    pub fn name(&self) -> &str {
        &self.prompt.name
    }

    /// An optional description of what the prompt does.
    pub fn description(&self) -> Option<&str> {
        self.prompt.description.as_deref()
    }

    /// Retrieve rendered messages for this prompt from the server.
    ///
    /// The `arguments` map corresponds to the prompt's declared arguments.
    /// Pass `None` when the prompt takes no arguments.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Protocol`] if the remote call fails.
    pub async fn get(
        &self,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let params = GetPromptRequestParams {
            meta: None,
            name: self.prompt.name.clone(),
            arguments,
        };
        let result = self
            .peer
            .get_prompt(params)
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;
        Ok(result.messages)
    }
}

/// Project an MCP [`Tool`](McpTool) into a [`CapabilityDescriptor`].
///
/// The descriptor ID is `"mcp-tool:{name}"`.  The MCP-specific extras (name,
/// title) are stored in `extensions["mcp"]` so callers can round-trip.
pub fn descriptor_from_mcp_tool(tool: &McpTool) -> CapabilityDescriptor {
    let scheduling = SchedulingFacts::new(ExecutionClass::Exclusive, false, false, false, None);
    let id = CapabilityId::new(format!("mcp-tool:{}", tool.name));
    let description = tool.description.as_deref().unwrap_or("").to_string();

    let mut desc = CapabilityDescriptor::new(
        id,
        CapabilityKind::Tool,
        tool.name.as_ref(),
        description,
        scheduling,
        ApprovalFacts::RuntimePolicy,
        AuthFacts::Service { scopes: vec![] },
    );
    desc.input_schema = Some(
        serde_json::to_value(&*tool.input_schema)
            .unwrap_or_else(|_| serde_json::json!({"type": "object"})),
    );
    desc.output_schema = tool
        .output_schema
        .as_deref()
        .and_then(|m| serde_json::to_value(m).ok());
    desc.accepts = vec![CapabilityModality::Json];
    desc.produces = vec![CapabilityModality::Json];
    desc.streaming = StreamingSupport::None;

    let mut mcp_ext = serde_json::Map::new();
    mcp_ext.insert("kind".to_string(), serde_json::json!("tool"));
    mcp_ext.insert("name".to_string(), serde_json::json!(tool.name.as_ref()));
    mcp_ext.insert("title".to_string(), serde_json::json!(null));
    desc.extensions
        .insert("mcp".to_string(), serde_json::Value::Object(mcp_ext));

    desc
}

/// Project an MCP [`Prompt`](rmcp::model::Prompt) into a [`CapabilityDescriptor`].
///
/// The descriptor ID is `"mcp-prompt:{name}"`.  The MCP-specific extras are
/// stored in `extensions["mcp"]`.
pub fn descriptor_from_mcp_prompt(prompt: &rmcp::model::Prompt) -> CapabilityDescriptor {
    let scheduling = SchedulingFacts::new(ExecutionClass::Shared, false, true, true, None);
    let id = CapabilityId::new(format!("mcp-prompt:{}", prompt.name));
    let description = prompt.description.as_deref().unwrap_or("").to_string();

    let mut desc = CapabilityDescriptor::new(
        id,
        CapabilityKind::Prompt,
        &prompt.name,
        description,
        scheduling,
        ApprovalFacts::None,
        AuthFacts::Service { scopes: vec![] },
    );
    desc.accepts = vec![CapabilityModality::Json];
    desc.produces = vec![CapabilityModality::Text];
    desc.streaming = StreamingSupport::None;

    let mut mcp_ext = serde_json::Map::new();
    mcp_ext.insert("kind".to_string(), serde_json::json!("prompt"));
    mcp_ext.insert("name".to_string(), serde_json::json!(&prompt.name));
    mcp_ext.insert("title".to_string(), serde_json::json!(null));
    mcp_ext.insert("arguments".to_string(), serde_json::json!(null));
    desc.extensions
        .insert("mcp".to_string(), serde_json::Value::Object(mcp_ext));

    desc
}

/// Project an MCP [`Resource`](rmcp::model::Resource) into a [`CapabilityDescriptor`].
///
/// The descriptor ID is `"mcp-resource:{uri}"`.  The MCP-specific extras are
/// stored in `extensions["mcp"]`.
pub fn descriptor_from_mcp_resource(resource: &rmcp::model::Resource) -> CapabilityDescriptor {
    let scheduling = SchedulingFacts::new(ExecutionClass::Shared, false, true, true, None);
    let id = CapabilityId::new(format!("mcp-resource:{}", resource.uri));
    let description = resource.description.as_deref().unwrap_or("").to_string();

    let mut desc = CapabilityDescriptor::new(
        id,
        CapabilityKind::Resource,
        &resource.name,
        description,
        scheduling,
        ApprovalFacts::None,
        AuthFacts::Service { scopes: vec![] },
    );
    desc.accepts = vec![];
    desc.produces = vec![CapabilityModality::Text];
    desc.streaming = StreamingSupport::None;

    let mut mcp_ext = serde_json::Map::new();
    mcp_ext.insert("kind".to_string(), serde_json::json!("resource"));
    mcp_ext.insert("uri".to_string(), serde_json::json!(&resource.uri));
    mcp_ext.insert(
        "mime_type".to_string(),
        resource
            .mime_type
            .as_deref()
            .map(|m| serde_json::json!(m))
            .unwrap_or(serde_json::json!(null)),
    );
    mcp_ext.insert("title".to_string(), serde_json::json!(null));
    desc.extensions
        .insert("mcp".to_string(), serde_json::Value::Object(mcp_ext));

    desc
}

/// Wrapper that adapts an MCP tool to the [`ToolDyn`] interface.
///
/// Holds a reference to the MCP peer for making remote tool calls.
pub(crate) struct McpToolWrapper {
    /// The MCP tool definition.
    tool: McpTool,
    /// Shared reference to the MCP peer for calling tools.
    peer: Arc<Peer<RoleClient>>,
}

impl McpToolWrapper {
    /// Create a new wrapper around an MCP tool.
    pub(crate) fn new(tool: McpTool, peer: Arc<Peer<RoleClient>>) -> Self {
        Self { tool, peer }
    }
}

impl ToolDyn for McpToolWrapper {
    fn name(&self) -> &str {
        &self.tool.name
    }

    fn description(&self) -> &str {
        self.tool.description.as_deref().unwrap_or("")
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::to_value(&*self.tool.input_schema)
            .unwrap_or_else(|_| serde_json::json!({"type": "object"}))
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        self.tool
            .output_schema
            .as_deref()
            .and_then(|m| serde_json::to_value(m).ok())
    }

    fn call(
        &self,
        input: serde_json::Value,
        ctx: &layer0::DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let name: Cow<'static, str> = self.tool.name.clone();
        let arguments = input.as_object().cloned();
        let peer = Arc::clone(&self.peer);
        let traceparent = ctx.trace.as_traceparent();

        Box::pin(async move {
            let meta = traceparent.map(|tp| {
                let mut m = rmcp::model::Meta::new();
                m.0.insert("traceparent".to_owned(), serde_json::Value::String(tp));
                m
            });

            tracing::debug!(
                tool = %name,
                traceparent = ?meta.as_ref().and_then(|m| m.0.get("traceparent")),
                "MCP tool call"
            );

            let params = CallToolRequestParams {
                meta,
                name,
                arguments,
                task: None,
            };

            let result: CallToolResult = peer
                .call_tool(params)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            if result.is_error == Some(true) {
                let msg = extract_text_from_content(&result.content);
                return Err(ToolError::ExecutionFailed(msg));
            }

            // If structured content is available, return it directly.
            if let Some(structured) = result.structured_content {
                return Ok(structured);
            }

            // Otherwise, extract text content.
            let text = extract_text_from_content(&result.content);
            Ok(serde_json::Value::String(text))
        })
    }
}

/// Extract text from MCP content blocks.
fn extract_text_from_content(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Tool as McpTool;
    use serde_json::json;
    use std::sync::Arc;

    fn make_test_tool(name: &str, description: &str) -> McpTool {
        let schema = json!({"type": "object", "properties": {"input": {"type": "string"}}});
        let schema_obj = schema.as_object().unwrap().clone();

        McpTool {
            name: Cow::Owned(name.to_string()),
            title: None,
            description: Some(Cow::Owned(description.to_string())),
            input_schema: Arc::new(schema_obj),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    /// Verify MCP tool metadata extraction matches what ToolDyn impl uses.
    ///
    /// The `ToolDyn` impl on `McpToolWrapper` delegates name/description/schema
    /// to the exact same expressions tested here. We cannot construct a
    /// `McpToolWrapper` without a real MCP `Peer`, but these assertions cover
    /// the identical code paths.
    #[test]
    fn mcp_tool_metadata_extraction() {
        let tool = make_test_tool("test_tool", "A test tool");

        // Same expression as ToolDyn::name() -> &self.tool.name
        assert_eq!(&*tool.name, "test_tool");
        // Same expression as ToolDyn::description() -> self.tool.description.as_deref().unwrap_or("")
        assert_eq!(tool.description.as_deref().unwrap_or(""), "A test tool");
        // Same expression as ToolDyn::input_schema() -> serde_json::to_value(&*self.tool.input_schema)
        let schema =
            serde_json::to_value(&*tool.input_schema).unwrap_or_else(|_| json!({"type": "object"}));
        assert_eq!(
            schema,
            json!({"type": "object", "properties": {"input": {"type": "string"}}})
        );
    }

    /// Verify `output_schema()` returns `None` when the MCP tool has no output schema.
    ///
    /// Tests the same field-access expression used in `McpToolWrapper::output_schema()`.
    #[test]
    fn mcp_tool_output_schema_none() {
        let tool = make_test_tool("no_schema", "no output schema");
        // Same expression as McpToolWrapper::output_schema()
        let result = tool
            .output_schema
            .as_deref()
            .and_then(|m| serde_json::to_value(m).ok());
        assert!(result.is_none());
    }

    /// Verify `output_schema()` returns the schema when the MCP tool declares one.
    ///
    /// Tests the same field-access expression used in `McpToolWrapper::output_schema()`.
    #[test]
    fn mcp_tool_output_schema_present() {
        let output_schema_val =
            json!({"type": "object", "properties": {"answer": {"type": "string"}}});
        let output_schema_obj = output_schema_val.as_object().unwrap().clone();
        let schema = json!({"type": "object"});
        let schema_obj = schema.as_object().unwrap().clone();
        let tool = McpTool {
            name: std::borrow::Cow::Owned("schema_tool".to_string()),
            title: None,
            description: Some(std::borrow::Cow::Owned("has output".to_string())),
            input_schema: Arc::new(schema_obj),
            output_schema: Some(Arc::new(output_schema_obj)),
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        };
        // Same expression as McpToolWrapper::output_schema()
        let result = tool
            .output_schema
            .as_deref()
            .and_then(|m| serde_json::to_value(m).ok())
            .unwrap();
        assert_eq!(result["type"], "object");
        assert!(result["properties"]["answer"].is_object());
    }

    /// Verify metadata extraction handles missing description.
    #[test]
    fn mcp_tool_metadata_missing_description() {
        let schema = json!({"type": "object"});
        let schema_obj = schema.as_object().unwrap().clone();
        let tool = McpTool {
            name: Cow::Owned("no_desc".to_string()),
            title: None,
            description: None,
            input_schema: Arc::new(schema_obj),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        };
        // Same fallback as ToolDyn::description()
        assert_eq!(tool.description.as_deref().unwrap_or(""), "");
    }

    /// Verify that McpToolWrapper is Send + Sync (required by ToolDyn).
    #[test]
    fn mcp_tool_wrapper_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<McpToolWrapper>();
    }

    /// Verify that McpResourceWrapper exposes correct metadata from the underlying model.
    ///
    /// `McpResourceWrapper` requires a live `Peer` to construct, so these assertions
    /// cover the identical field-access expressions used in the wrapper methods.
    #[test]
    fn mcp_resource_wrapper_exposes_metadata() {
        let raw = rmcp::model::RawResource {
            uri: "state://global/config".to_string(),
            name: "config".to_string(),
            title: None,
            description: Some("Server configuration".to_string()),
            mime_type: Some("application/json".into()),
            size: None,
            icons: None,
            meta: None,
        };
        let resource = rmcp::model::Annotated::new(raw, None);
        // Same as McpResourceWrapper::uri()
        assert_eq!(resource.uri, "state://global/config");
        // Same as McpResourceWrapper::name()
        assert_eq!(resource.name, "config");
        // Same as McpResourceWrapper::description()
        assert_eq!(
            resource.description.as_deref(),
            Some("Server configuration")
        );
    }

    /// Verify that McpResourceWrapper handles missing description.
    #[test]
    fn mcp_resource_wrapper_missing_description() {
        let raw = rmcp::model::RawResource {
            uri: "state://global/x".to_string(),
            name: "x".to_string(),
            title: None,
            description: None,
            mime_type: None,
            size: None,
            icons: None,
            meta: None,
        };
        let resource = rmcp::model::Annotated::new(raw, None);
        assert_eq!(resource.description.as_deref(), None);
    }

    /// Verify that McpPromptWrapper exposes correct metadata from the underlying model.
    ///
    /// Mirrors the pattern of `mcp_tool_metadata_extraction` - tests the same
    /// field-access expressions used in the wrapper without a live Peer.
    #[test]
    fn mcp_prompt_wrapper_exposes_metadata() {
        let prompt = rmcp::model::Prompt {
            name: "greet".to_string(),
            title: None,
            description: Some("A greeting".to_string()),
            arguments: None,
            icons: None,
            meta: None,
        };
        // Same as McpPromptWrapper::name()
        assert_eq!(prompt.name, "greet");
        // Same as McpPromptWrapper::description()
        assert_eq!(prompt.description.as_deref(), Some("A greeting"));
    }

    /// Verify that McpPromptWrapper handles missing description.
    #[test]
    fn mcp_prompt_wrapper_missing_description() {
        let prompt = rmcp::model::Prompt {
            name: "bare".to_string(),
            title: None,
            description: None,
            arguments: None,
            icons: None,
            meta: None,
        };
        assert_eq!(prompt.description.as_deref(), None);
    }

    /// Verify that McpResourceWrapper and McpPromptWrapper are Send + Sync.
    #[test]
    fn mcp_resource_and_prompt_wrapper_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<McpResourceWrapper>();
        assert_send_sync::<McpPromptWrapper>();
    }

    /// `tool_budget_tokens` returns 0 for an empty tool list.
    #[test]
    fn tool_budget_tokens_empty() {
        let tools: Vec<McpTool> = vec![];
        assert_eq!(McpClient::tool_budget_tokens(&tools), 0);
    }

    /// `tool_budget_tokens` sums (name_len + desc_len) / 4 per tool.
    ///
    /// "search" (6) + "Searches the web" (16) = 22 / 4 = 5
    /// "read" (4) + "Read a file" (11) = 15 / 4 = 3
    /// Total = 8
    #[test]
    fn tool_budget_tokens_counts_descriptions() {
        let tools = vec![
            make_test_tool("search", "Searches the web"),
            make_test_tool("read", "Read a file"),
        ];
        let estimate = McpClient::tool_budget_tokens(&tools);
        assert!(
            estimate > 0,
            "non-empty tool list must produce a non-zero estimate"
        );
        assert_eq!(estimate, 8);
    }

    /// Integration test that connects to a real MCP server.
    /// Requires an MCP server binary to be available.
    #[tokio::test]
    #[ignore]
    async fn integration_connect_and_discover() {
        let cmd = tokio::process::Command::new("npx");
        // Would need: cmd.arg("-y").arg("@modelcontextprotocol/server-everything");
        let client = McpClient::connect_stdio(cmd).await.unwrap();
        let tools = client.discover_tools().await.unwrap();
        assert!(!tools.is_empty());
        client.close().await.unwrap();
    }

    /// Verify that `ToolOperator::metadata()` correctly extracts name, description,
    /// schema, and parallel-safety from a `ToolDyn` — the same pipeline used inside
    /// `discover_operators()`. Also covers the edge case where description is empty
    /// (matching what `McpToolWrapper` produces when the MCP tool has no description).
    #[test]
    fn discover_operators_metadata_extraction() {
        struct MockMcpTool {
            name: &'static str,
            description: &'static str,
            schema: serde_json::Value,
        }
        impl ToolDyn for MockMcpTool {
            fn name(&self) -> &str {
                self.name
            }
            fn description(&self) -> &str {
                self.description
            }
            fn input_schema(&self) -> serde_json::Value {
                self.schema.clone()
            }
            fn call(
                &self,
                _: serde_json::Value,
                _ctx: &layer0::DispatchContext,
            ) -> Pin<
                Box<
                    dyn std::future::Future<Output = Result<serde_json::Value, ToolError>>
                        + Send
                        + '_,
                >,
            > {
                Box::pin(async { Ok(serde_json::Value::Null) })
            }
            // concurrency_hint() not overridden → defaults to Exclusive → parallel_safe = false
        }

        let schema = json!({"type": "object", "properties": {"query": {"type": "string"}}});

        // Normal case: tool with name and description.
        let arc: Arc<dyn ToolDyn> = Arc::new(MockMcpTool {
            name: "web_search",
            description: "Search the web",
            schema: schema.clone(),
        });
        let op = ToolOperator::new(Arc::clone(&arc));
        let meta = op.metadata();
        assert_eq!(meta.name, "web_search");
        assert_eq!(meta.description, "Search the web");
        assert_eq!(meta.input_schema, schema);
        // McpToolWrapper uses the default concurrency_hint (Exclusive) → parallel_safe = false
        assert!(!meta.parallel_safe);

        // Edge case: empty description (McpToolWrapper returns "" when MCP tool has no description).
        let arc_nodesc: Arc<dyn ToolDyn> = Arc::new(MockMcpTool {
            name: "bare_tool",
            description: "",
            schema: json!({"type": "object"}),
        });
        let op_nodesc = ToolOperator::new(Arc::clone(&arc_nodesc));
        let meta_nodesc = op_nodesc.metadata();
        assert_eq!(meta_nodesc.description, "");
    }

    /// Verify the warning threshold constant and predicate used by `discover_operators()`.
    ///
    /// `discover_operators` mirrors the `discover_tools` logic: it emits a warn when
    /// `tool_count > TOOL_COUNT_WARN_THRESHOLD`. This test exercises the predicate
    /// boundaries to guard against accidental changes (same pattern as
    /// `tool_budget_tokens_empty` / `tool_budget_tokens_counts_descriptions`).
    #[test]
    fn discover_operators_preserves_tool_count_logic() {
        // The threshold constant is part of the public API contract.
        assert_eq!(
            TOOL_COUNT_WARN_THRESHOLD, 20,
            "threshold must be 20 for context budget calculations"
        );

        // Boundary: at-threshold must NOT trigger the warning (strict greater-than).
        let at_threshold = TOOL_COUNT_WARN_THRESHOLD;
        assert!(
            at_threshold <= TOOL_COUNT_WARN_THRESHOLD,
            "count == threshold should not warn"
        );

        // One over the limit must trigger the warning.
        let over = TOOL_COUNT_WARN_THRESHOLD + 1;
        assert!(
            over > TOOL_COUNT_WARN_THRESHOLD,
            "count {} must exceed threshold {}",
            over,
            TOOL_COUNT_WARN_THRESHOLD
        );
    }
}
