//! Context operations — thin wrappers around Context mutation.

pub mod compact;
pub mod inject;
pub mod response;
pub mod tool;

pub use compact::{Compact, CompactResult};
pub use inject::{InjectMessage, InjectMessages, InjectSystem};
pub use response::AppendResponse;
pub use tool::{ExecuteTool, format_tool_result};
