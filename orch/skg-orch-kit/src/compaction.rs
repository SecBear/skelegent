use async_trait::async_trait;
use std::error::Error;
use std::fmt;

/// Boxed error type used by orchestration-local compaction steps.
pub type CompactionOperationError = Box<dyn Error + Send + Sync + 'static>;

/// Snapshot of orchestration-local compaction coordination state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionSnapshot {
    requested: bool,
    flush_required: bool,
}

impl CompactionSnapshot {
    /// Create a new compaction snapshot.
    pub fn new(requested: bool, flush_required: bool) -> Self {
        Self {
            requested,
            flush_required,
        }
    }

    /// Whether the runtime is currently asking orchestration to compact.
    pub fn requested(&self) -> bool {
        self.requested
    }

    /// Whether orchestration must flush pending state before compacting.
    pub fn flush_required(&self) -> bool {
        self.flush_required
    }

    /// Decide what orchestration should do next.
    pub fn decide(&self) -> CompactionDecision {
        match (self.requested, self.flush_required) {
            (false, _) => CompactionDecision::Skip,
            (true, false) => CompactionDecision::Compact,
            (true, true) => CompactionDecision::FlushThenCompact,
        }
    }
}

/// Minimal orchestration-local compaction actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionDecision {
    /// Leave the current execution shape unchanged.
    Skip,
    /// Run compaction directly in the current execution.
    Compact,
    /// Flush pending state or effects before compacting.
    FlushThenCompact,
}

/// Result of coordinating one compaction request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionOutcome {
    decision: CompactionDecision,
    flushed: bool,
    compacted: bool,
}

impl CompactionOutcome {
    fn skipped() -> Self {
        Self {
            decision: CompactionDecision::Skip,
            flushed: false,
            compacted: false,
        }
    }

    fn executed(flushed: bool) -> Self {
        Self {
            decision: if flushed {
                CompactionDecision::FlushThenCompact
            } else {
                CompactionDecision::Compact
            },
            flushed,
            compacted: true,
        }
    }

    /// The coordination decision that was executed.
    pub fn decision(&self) -> CompactionDecision {
        self.decision
    }

    /// Whether a flush step completed before compaction.
    pub fn flushed(&self) -> bool {
        self.flushed
    }

    /// Whether compaction completed.
    pub fn compacted(&self) -> bool {
        self.compacted
    }
}

/// Orchestration-local flush step that persists state before destructive compaction.
#[async_trait]
pub trait FlushBeforeCompact {
    /// Persist the state needed to survive the following compaction.
    async fn flush(&mut self) -> Result<(), CompactionOperationError>;
}

/// Orchestration-local compaction step.
#[async_trait]
pub trait CompactContext {
    /// Shrink the active context after any required flush succeeds.
    async fn compact(&mut self) -> Result<(), CompactionOperationError>;
}

/// Explicit failure stage for orchestration-local compaction coordination.
#[derive(Debug)]
pub enum CompactionCoordinationError {
    /// Flush failed, so compaction did not run.
    FlushFailed(CompactionOperationError),
    /// Compaction failed after any required flush completed.
    CompactionFailed(CompactionOperationError),
}

impl fmt::Display for CompactionCoordinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FlushFailed(_) => write!(f, "flush before compaction failed"),
            Self::CompactionFailed(_) => write!(f, "compaction failed"),
        }
    }
}

impl Error for CompactionCoordinationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FlushFailed(source) => Some(source.as_ref()),
            Self::CompactionFailed(source) => Some(source.as_ref()),
        }
    }
}

/// Small coordinator that turns a runtime compaction request into ordered async work.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompactionCoordinator;

impl CompactionCoordinator {
    /// Create a coordinator for orchestration-local compaction flow.
    pub fn new() -> Self {
        Self
    }

    /// Decide the next orchestration-local action for the given snapshot.
    pub fn decide(&self, snapshot: CompactionSnapshot) -> CompactionDecision {
        snapshot.decide()
    }

    /// Execute flush and compaction in the required order for one snapshot.
    pub async fn coordinate<F, C>(
        &self,
        snapshot: CompactionSnapshot,
        flusher: &mut F,
        compactor: &mut C,
    ) -> Result<CompactionOutcome, CompactionCoordinationError>
    where
        F: FlushBeforeCompact + ?Sized,
        C: CompactContext + ?Sized,
    {
        match self.decide(snapshot) {
            CompactionDecision::Skip => Ok(CompactionOutcome::skipped()),
            CompactionDecision::Compact => {
                compactor
                    .compact()
                    .await
                    .map_err(CompactionCoordinationError::CompactionFailed)?;
                Ok(CompactionOutcome::executed(false))
            }
            CompactionDecision::FlushThenCompact => {
                flusher
                    .flush()
                    .await
                    .map_err(CompactionCoordinationError::FlushFailed)?;
                compactor
                    .compact()
                    .await
                    .map_err(CompactionCoordinationError::CompactionFailed)?;
                Ok(CompactionOutcome::executed(true))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompactContext, CompactionCoordinationError, CompactionCoordinator, CompactionDecision,
        CompactionSnapshot, FlushBeforeCompact,
    };
    use async_trait::async_trait;
    use std::io::Error;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct RecorderState {
        events: Vec<&'static str>,
        fail_flush: bool,
        fail_compact: bool,
    }

    #[derive(Clone, Debug, Default)]
    struct SharedRecorder(Arc<Mutex<RecorderState>>);

    impl SharedRecorder {
        fn with_failures(fail_flush: bool, fail_compact: bool) -> Self {
            Self(Arc::new(Mutex::new(RecorderState {
                events: Vec::new(),
                fail_flush,
                fail_compact,
            })))
        }

        fn events(&self) -> Vec<&'static str> {
            self.0.lock().unwrap().events.clone()
        }
    }

    struct RecordingFlusher {
        recorder: SharedRecorder,
    }

    struct RecordingCompactor {
        recorder: SharedRecorder,
    }

    #[async_trait]
    impl FlushBeforeCompact for RecordingFlusher {
        async fn flush(&mut self) -> Result<(), super::CompactionOperationError> {
            let mut state = self.recorder.0.lock().unwrap();
            state.events.push("flush");
            if state.fail_flush {
                return Err(Box::new(Error::other("flush failed")));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl CompactContext for RecordingCompactor {
        async fn compact(&mut self) -> Result<(), super::CompactionOperationError> {
            let mut state = self.recorder.0.lock().unwrap();
            state.events.push("compact");
            if state.fail_compact {
                return Err(Box::new(Error::other("compact failed")));
            }
            Ok(())
        }
    }

    #[test]
    fn skip_when_compaction_not_requested() {
        let snapshot = CompactionSnapshot::new(false, false);

        assert_eq!(snapshot.decide(), CompactionDecision::Skip);
    }

    #[test]
    fn compact_when_requested_and_no_flush_needed() {
        let snapshot = CompactionSnapshot::new(true, false);

        assert_eq!(snapshot.decide(), CompactionDecision::Compact);
    }

    #[test]
    fn flush_then_compact_when_requested_and_flush_needed() {
        let snapshot = CompactionSnapshot::new(true, true);

        assert_eq!(snapshot.decide(), CompactionDecision::FlushThenCompact);
    }

    #[test]
    fn skip_even_if_flush_flag_is_set_without_request() {
        let snapshot = CompactionSnapshot::new(false, true);

        assert_eq!(snapshot.decide(), CompactionDecision::Skip);
    }

    #[tokio::test]
    async fn flush_runs_before_compaction_when_required() {
        let recorder = SharedRecorder::default();
        let mut flusher = RecordingFlusher {
            recorder: recorder.clone(),
        };
        let mut compactor = RecordingCompactor {
            recorder: recorder.clone(),
        };

        let outcome = CompactionCoordinator::new()
            .coordinate(CompactionSnapshot::new(true, true), &mut flusher, &mut compactor)
            .await
            .unwrap();

        assert_eq!(outcome.decision(), CompactionDecision::FlushThenCompact);
        assert!(outcome.flushed());
        assert!(outcome.compacted());
        assert_eq!(recorder.events(), vec!["flush", "compact"]);
    }

    #[tokio::test]
    async fn flush_failure_prevents_compaction_and_is_explicit() {
        let recorder = SharedRecorder::with_failures(true, false);
        let mut flusher = RecordingFlusher {
            recorder: recorder.clone(),
        };
        let mut compactor = RecordingCompactor {
            recorder: recorder.clone(),
        };

        let error = CompactionCoordinator::new()
            .coordinate(CompactionSnapshot::new(true, true), &mut flusher, &mut compactor)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            CompactionCoordinationError::FlushFailed(_)
        ));
        assert_eq!(recorder.events(), vec!["flush"]);
    }

    #[tokio::test]
    async fn compact_only_path_skips_flush() {
        let recorder = SharedRecorder::default();
        let mut flusher = RecordingFlusher {
            recorder: recorder.clone(),
        };
        let mut compactor = RecordingCompactor {
            recorder: recorder.clone(),
        };

        let outcome = CompactionCoordinator::new()
            .coordinate(CompactionSnapshot::new(true, false), &mut flusher, &mut compactor)
            .await
            .unwrap();

        assert_eq!(outcome.decision(), CompactionDecision::Compact);
        assert!(!outcome.flushed());
        assert!(outcome.compacted());
        assert_eq!(recorder.events(), vec!["compact"]);
    }

    #[tokio::test]
    async fn no_request_path_skips_both() {
        let recorder = SharedRecorder::default();
        let mut flusher = RecordingFlusher {
            recorder: recorder.clone(),
        };
        let mut compactor = RecordingCompactor {
            recorder: recorder.clone(),
        };

        let outcome = CompactionCoordinator::new()
            .coordinate(CompactionSnapshot::new(false, true), &mut flusher, &mut compactor)
            .await
            .unwrap();

        assert_eq!(outcome.decision(), CompactionDecision::Skip);
        assert!(!outcome.flushed());
        assert!(!outcome.compacted());
        assert!(recorder.events().is_empty());
    }
}
