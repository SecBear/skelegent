//! Lifecycle-adjacent protocol types for Layer 0.
//!
//! Layer 0 stays protocol-only. Lifecycle coordination lives above this crate;
//! the only current cross-boundary lifecycle surface here is message-level
//! compaction policy metadata attached to [`crate::MessageMeta`].

use serde::{Deserialize, Serialize};

/// Policy controlling how a message should be treated by compaction-aware layers.
///
/// Stored in [`crate::MessageMeta`], which is attached to every message.
/// All variants are advisory when used with strategies that don't inspect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPolicy {
    /// Never compact this message. Architectural decisions, constraints, user instructions.
    Pinned,
    /// Subject to normal compaction. Default for all messages.
    #[default]
    Normal,
    /// Compress this message preferentially (verbose output, build logs).
    CompressFirst,
    /// Discard when the originating tool session or MCP session ends.
    DiscardWhenDone,
}
