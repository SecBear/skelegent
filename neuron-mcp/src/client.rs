//! MCP client that discovers remote tools and wraps them as [`ToolDyn`].
//!
//! [`McpClient`] connects to an MCP server (via stdio child process or
//! streamable HTTP), discovers its tools, and wraps each as a [`ToolDyn`]
//! implementation so they can be registered in a
//! [`ToolRegistry`](neuron_tool::ToolRegistry).

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use neuron_tool::{AliasedTool, ToolDyn, ToolError};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, RawContent, Tool as McpTool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;

use crate::error::McpError;

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

    /// Connect to an MCP server via streamable HTTP (supersedes SSE).
    ///
    /// The URL should point to the MCP server's HTTP endpoint
    /// (e.g., `http://localhost:8080/mcp`).
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Connection`] if the HTTP connection or MCP
    /// handshake fails.
    pub async fn connect_sse(url: &str) -> Result<Self, McpError> {
        let transport = StreamableHttpClientTransport::from_uri(url);
        let service: RunningService<RoleClient, ()> = ()
            .serve(transport)
            .await
            .map_err(|e| McpError::Connection(e.to_string()))?;
        Ok(Self { service })
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

        let peer = self.service.peer().clone();
        let peer = Arc::new(peer);

        let tools: Vec<Arc<dyn ToolDyn>> = result
            .into_iter()
            .map(|tool| Arc::new(McpToolWrapper::new(tool, Arc::clone(&peer))) as Arc<dyn ToolDyn>)
            .collect();

        Ok(tools)
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

    fn call(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let name: Cow<'static, str> = self.tool.name.clone();
        let arguments = input.as_object().cloned();
        let peer = Arc::clone(&self.peer);

        Box::pin(async move {
            let params = CallToolRequestParams {
                meta: None,
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
}
