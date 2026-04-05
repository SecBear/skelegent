//! MCP server that exposes context compaction operations as MCP tools.
//!
//! Three tools are provided:
//! - `compact_sliding_window`: keep the first message and recent messages by token budget
//! - `compact_tiered`: zone-partition messages by recency
//! - `compact_salience`: MMR-based salience-aware compaction
//!
//! The server communicates over stdio for use with Claude Code or any
//! MCP-compatible client.

use std::borrow::Cow;
use std::sync::Arc;

use layer0::Content as L0Content;
use layer0::context::{Message, Role};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    ProtocolVersion, ServerCapabilities, ServerInfo, Tool as McpTool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::io::stdio;
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use skg_context::{SaliencePackingConfig, TieredConfig};

// --- Wire types --------------------------------------------------------------

/// JSON-serialisable message exchanged with MCP clients.
#[derive(Debug, Serialize, Deserialize)]
struct CompactMessage {
    role: String,
    content: String,
}

/// Input for the `compact_sliding_window` tool.
#[derive(Debug, Deserialize)]
struct SlidingWindowInput {
    messages: Vec<CompactMessage>,
    // token_budget is part of the advertised schema for API consistency, but
    // sliding_window_compactor derives its own budget from the message set.
    #[allow(dead_code)]
    token_budget: Option<usize>,
}

/// Input for the `compact_tiered` tool.
#[derive(Debug, Deserialize)]
struct TieredInput {
    messages: Vec<CompactMessage>,
    active_zone_size: Option<usize>,
}

/// Input for the `compact_salience` tool.
#[derive(Debug, Deserialize)]
struct SalienceInput {
    messages: Vec<CompactMessage>,
    token_budget: Option<usize>,
    lambda: Option<f64>,
}

// --- Message conversion -------------------------------------------------------

/// Convert a wire message into a `layer0` [`Message`].
fn to_layer0(m: &CompactMessage) -> Message {
    let role = match m.role.as_str() {
        "assistant" => Role::Assistant,
        "system" => Role::System,
        _ => Role::User,
    };
    Message::new(role, L0Content::text(m.content.clone()))
}

/// Convert a `layer0` [`Message`] back into a wire message.
fn from_layer0(m: &Message) -> CompactMessage {
    CompactMessage {
        role: match &m.role {
            Role::System => "system".to_string(),
            Role::User => "user".to_string(),
            Role::Assistant => "assistant".to_string(),
            Role::Tool { .. } => "tool".to_string(),
            // Non-exhaustive enum: map any future variant to a safe default.
            _ => "unknown".to_string(),
        },
        content: m.text_content(),
    }
}

// --- Tool dispatch ------------------------------------------------------------

fn call_sliding_window(args: Value) -> Result<CallToolResult, ErrorData> {
    let input: SlidingWindowInput =
        serde_json::from_value(args).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
    let messages: Vec<Message> = input.messages.iter().map(to_layer0).collect();
    let mut compactor = skg_context::sliding_window_compactor();
    let result: Vec<CompactMessage> = compactor(&messages).iter().map(from_layer0).collect();
    let text = serde_json::to_string(&result)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn call_tiered(args: Value) -> Result<CallToolResult, ErrorData> {
    let input: TieredInput =
        serde_json::from_value(args).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
    let messages: Vec<Message> = input.messages.iter().map(to_layer0).collect();
    let config = TieredConfig {
        active_zone_size: input.active_zone_size.unwrap_or(10),
    };
    let mut compactor = skg_context::tiered_compactor(config);
    let result: Vec<CompactMessage> = compactor(&messages).iter().map(from_layer0).collect();
    let text = serde_json::to_string(&result)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn call_salience(args: Value) -> Result<CallToolResult, ErrorData> {
    let input: SalienceInput =
        serde_json::from_value(args).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
    let messages: Vec<Message> = input.messages.iter().map(to_layer0).collect();
    let config = SaliencePackingConfig {
        token_budget: input.token_budget.unwrap_or(4000),
        lambda: input.lambda.unwrap_or(0.7),
        ..Default::default()
    };
    let mut compactor = skg_context::salience_packing_compactor(config);
    let result: Vec<CompactMessage> = compactor(&messages).iter().map(from_layer0).collect();
    let text = serde_json::to_string(&result)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// --- Schema helpers ----------------------------------------------------------

/// Build a minimal JSON Schema object for a tool's `messages` array input.
fn messages_property() -> Value {
    serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "role": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["role", "content"]
        }
    })
}

fn sliding_window_schema() -> Map<String, Value> {
    let v = serde_json::json!({
        "type": "object",
        "properties": {
            "messages": messages_property(),
            "token_budget": { "type": "integer", "default": 4000 }
        },
        "required": ["messages"]
    });
    v.as_object().cloned().unwrap_or_default()
}

fn tiered_schema() -> Map<String, Value> {
    let v = serde_json::json!({
        "type": "object",
        "properties": {
            "messages": messages_property(),
            "active_zone_size": { "type": "integer", "default": 10 }
        },
        "required": ["messages"]
    });
    v.as_object().cloned().unwrap_or_default()
}

fn salience_schema() -> Map<String, Value> {
    let v = serde_json::json!({
        "type": "object",
        "properties": {
            "messages": messages_property(),
            "token_budget": { "type": "integer", "default": 4000 },
            "lambda": { "type": "number", "default": 0.7 }
        },
        "required": ["messages"]
    });
    v.as_object().cloned().unwrap_or_default()
}

fn make_tool(name: &str, description: &str, schema: Map<String, Value>) -> McpTool {
    McpTool {
        name: Cow::Owned(name.to_string()),
        title: None,
        description: Some(Cow::Owned(description.to_string())),
        input_schema: Arc::new(schema),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

fn tool_list() -> Vec<McpTool> {
    vec![
        make_tool(
            "compact_sliding_window",
            "Keep the first message and the most recent messages within a token budget. \
             Pinned messages always survive.",
            sliding_window_schema(),
        ),
        make_tool(
            "compact_tiered",
            "Zone-partition messages: pinned always kept, N most-recent normal messages \
             kept, older messages dropped.",
            tiered_schema(),
        ),
        make_tool(
            "compact_salience",
            "Salience-aware MMR compaction: maximize coverage while minimizing redundancy \
             within a token budget.",
            salience_schema(),
        ),
    ]
}

// --- Server ------------------------------------------------------------------

/// MCP server that exposes the three context compaction tools.
struct CompactionServer;

impl CompactionServer {
    fn new() -> Self {
        Self
    }
}

impl ServerHandler for CompactionServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities {
                tools: Some(rmcp::model::ToolsCapability::default()),
                ..Default::default()
            },
            server_info: Implementation {
                name: "skg-compaction-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
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
        Ok(ListToolsResult::with_all_items(tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = match request.arguments {
            Some(map) => Value::Object(map),
            None => Value::Object(Map::new()),
        };

        match request.name.as_ref() {
            "compact_sliding_window" => call_sliding_window(args),
            "compact_tiered" => call_tiered(args),
            "compact_salience" => call_salience(args),
            name => Err(ErrorData::invalid_params(
                format!("unknown tool: {name}"),
                None,
            )),
        }
    }
}

// --- Entry point -------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = CompactionServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
