//! Reference context operations.

pub mod cognitive;

pub mod compact;
pub mod inject;
pub mod response;
pub mod store;
pub mod tool;

pub use cognitive::{
    CognitiveError, CognitiveState, CommitCognitiveState, CompressCognitiveStateConfig,
    DEFAULT_CCS_PROMPT, Entity, Relation,
};
pub use compact::{Compact, CompactResult};
pub use inject::{InjectMessage, InjectMessages, InjectSystem};
pub use response::AppendResponse;
pub use store::{
    FlushToStore, InjectFromStore, InjectSearchResults, InjectionPosition, LoadConversation,
    SaveConversation, fetch_search_results,
};
pub use tool::{ExecuteTool, format_tool_result};
