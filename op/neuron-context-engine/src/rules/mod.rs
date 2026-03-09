//! Reference rule implementations.

pub mod budget;
pub mod compaction;
pub mod telemetry;

pub use budget::BudgetGuard;
pub use compaction::{CompactionRule, CompactionStrategy};
pub use compaction::{
    DEFAULT_EXTRACT_PROMPT_TEMPLATE, DEFAULT_SUMMARY_PROMPT, ExtractConfig, SummarizeConfig,
    extract_cognitive_state_with, summarize_with,
};
pub use compaction::strip_json_fences;
pub use telemetry::{TelemetryConfig, TelemetryLevel, TelemetryRule, TelemetryVerbosity};
