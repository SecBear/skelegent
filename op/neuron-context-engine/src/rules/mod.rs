//! Reference rule implementations.

pub mod budget;
pub mod compaction;
pub mod telemetry;

pub use budget::BudgetGuard;
pub use compaction::{CompactionRule, CompactionStrategy};
pub use compaction::{SummarizeConfig, ExtractConfig, DEFAULT_SUMMARY_PROMPT, DEFAULT_EXTRACT_PROMPT_TEMPLATE, summarize_with, extract_cognitive_state_with};
pub use telemetry::{TelemetryConfig, TelemetryLevel, TelemetryRule, TelemetryVerbosity};