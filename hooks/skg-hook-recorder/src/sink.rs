//! Built-in [`RecordSink`] implementations.

use crate::{RecordEntry, RecordSink};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// IN-MEMORY SINK
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A [`RecordSink`] that stores all entries in memory.
///
/// Useful for testing, short-lived pipelines, and replay scenarios.
/// All entries are accumulated in a `Vec` protected by a `RwLock`.
///
/// # Example
///
/// ```rust
/// # tokio_test::block_on(async {
/// use skg_hook_recorder::{InMemorySink, RecordEntry, Boundary, Phase, RecordContext};
/// use skg_hook_recorder::RecordSink;
/// use std::sync::Arc;
///
/// let sink = Arc::new(InMemorySink::new());
/// let entry = RecordEntry::pre(
///     Boundary::Dispatch,
///     RecordContext::empty(),
///     serde_json::json!({"test": true}),
/// );
/// sink.record(entry).await;
///
/// let entries = sink.entries().await;
/// assert_eq!(entries.len(), 1);
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct InMemorySink {
    entries: Arc<RwLock<Vec<RecordEntry>>>,
}

impl InMemorySink {
    /// Create a new empty in-memory sink.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Return a snapshot of all recorded entries.
    pub async fn entries(&self) -> Vec<RecordEntry> {
        self.entries.read().await.clone()
    }

    /// Return the number of entries currently stored.
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Return `true` if no entries have been recorded yet.
    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }
}

impl Default for InMemorySink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RecordSink for InMemorySink {
    async fn record(&self, entry: RecordEntry) {
        self.entries.write().await.push(entry);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Boundary, RecordContext, RecordEntry, SCHEMA_VERSION};

    fn make_entry() -> RecordEntry {
        RecordEntry::pre(
            Boundary::Dispatch,
            RecordContext::empty(),
            serde_json::json!({"test": true}),
        )
    }

    #[tokio::test]
    async fn in_memory_sink_stores_entries() {
        let sink = InMemorySink::new();
        assert!(sink.is_empty().await);

        sink.record(make_entry()).await;
        assert_eq!(sink.len().await, 1);

        sink.record(make_entry()).await;
        assert_eq!(sink.len().await, 2);

        let entries = sink.entries().await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].version, SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn in_memory_sink_concurrent() {
        use std::sync::Arc;

        let sink = Arc::new(InMemorySink::new());
        let mut handles = Vec::new();

        for i in 0..10 {
            let s = sink.clone();
            handles.push(tokio::spawn(async move {
                let entry = RecordEntry::pre(
                    Boundary::Dispatch,
                    RecordContext::empty(),
                    serde_json::json!({"task": i}),
                );
                s.record(entry).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(sink.len().await, 10);
    }
}
