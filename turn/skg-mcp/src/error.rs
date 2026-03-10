//! Error types for MCP operations.

/// Errors from MCP client and server operations.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Connection to the MCP server or client failed.
    #[error("connection failed: {0}")]
    Connection(String),

    /// MCP protocol-level error.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Error related to tool operations.
    #[error("tool error: {0}")]
    Tool(String),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_error_display() {
        assert_eq!(
            McpError::Connection("timeout".into()).to_string(),
            "connection failed: timeout"
        );
        assert_eq!(
            McpError::Protocol("bad frame".into()).to_string(),
            "protocol error: bad frame"
        );
        assert_eq!(
            McpError::Tool("not found".into()).to_string(),
            "tool error: not found"
        );
    }

    #[test]
    fn mcp_error_from_boxed() {
        let err: Box<dyn std::error::Error + Send + Sync> = "some error".into();
        let mcp_err = McpError::from(err);
        assert!(matches!(mcp_err, McpError::Other(_)));
        assert_eq!(mcp_err.to_string(), "some error");
    }

    #[test]
    fn mcp_error_is_debug() {
        let err = McpError::Connection("test".into());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Connection"));
    }
}
