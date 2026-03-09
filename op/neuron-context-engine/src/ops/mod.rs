//! Reference context operations.

pub mod compact;
pub mod inject;
pub mod response;
pub mod store;
pub mod tool;

pub use compact::{Compact, CompactResult};
pub use inject::{InjectMessage, InjectMessages, InjectSystem};
pub use response::AppendResponse;
pub use store::{FlushToStore, InjectFromStore, InjectionPosition};
pub use tool::{ExecuteTool, format_tool_result};
