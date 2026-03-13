//! Reference context operations.

pub mod cognitive;

pub mod compact;
pub mod inject;
pub mod memory_note;
pub mod procedural;
pub mod qualify;
pub mod response;
pub mod store;
pub mod tool;

pub use cognitive::{
    CognitiveError, CognitiveState, CommitCognitiveState, CompressCognitiveStateConfig,
    DEFAULT_CCS_PROMPT, Entity, Relation,
};
pub use compact::{Compact, CompactResult};
pub use inject::{InjectMessage, InjectMessages, InjectSystem};
pub use memory_note::{
    ConstructNoteConfig, DEFAULT_CONSTRUCT_NOTE_PROMPT, DEFAULT_EVOLVE_MEMORY_PROMPT,
    DEFAULT_LINK_GENERATION_PROMPT, EvolveMemoryConfig, LinkGenerationConfig, MemoryNote, NoteLink,
};
pub use procedural::{
    DEFAULT_DISTILL_PROCEDURE_PROMPT, DistillProcedureConfig, Procedure, ProcedureStep,
    RecallProcedureConfig, RefineProcedureConfig,
};
pub use qualify::{DEFAULT_QUALIFY_PROMPT, QualifyRecallConfig, RecalledArtifact};
pub use response::AppendResponse;
pub use store::{
    FlushToStore, InjectFromStore, InjectSearchResults, InjectionPosition, LoadConversation,
    SaveConversation, fetch_search_results,
};
pub use tool::{ExecuteTool, format_tool_result};
