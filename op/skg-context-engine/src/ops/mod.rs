//! Reference context operations.

pub mod cognitive;

pub mod memory_note;
pub mod qualify;
pub mod compact;
pub mod inject;
pub mod response;
pub mod store;
pub mod tool;
pub mod procedural;

pub use cognitive::{
    CognitiveError, CognitiveState, CommitCognitiveState, CompressCognitiveStateConfig,
    DEFAULT_CCS_PROMPT, Entity, Relation,
};
pub use memory_note::{
    ConstructNoteConfig, DEFAULT_CONSTRUCT_NOTE_PROMPT, EvolveMemoryConfig,
    DEFAULT_EVOLVE_MEMORY_PROMPT, LinkGenerationConfig, DEFAULT_LINK_GENERATION_PROMPT,
    MemoryNote, NoteLink,
};
pub use qualify::{DEFAULT_QUALIFY_PROMPT, QualifyRecallConfig, RecalledArtifact};
pub use compact::{Compact, CompactResult};
pub use inject::{InjectMessage, InjectMessages, InjectSystem};
pub use response::AppendResponse;
pub use store::{
    FlushToStore, InjectFromStore, InjectSearchResults, InjectionPosition, LoadConversation,
    SaveConversation, fetch_search_results,
};
pub use tool::{ExecuteTool, format_tool_result};
pub use procedural::{
    DistillProcedureConfig, Procedure, ProcedureStep, RecallProcedureConfig,
    RefineProcedureConfig, DEFAULT_DISTILL_PROCEDURE_PROMPT,
};