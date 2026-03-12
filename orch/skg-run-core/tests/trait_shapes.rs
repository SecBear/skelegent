use async_trait::async_trait;
use serde_json::json;
use skg_run_core::{
    BackendRunRef, DispatchPayload, DriverError, DriverRequest, DriverResponse, LeaseClaim,
    LeaseGrant, LeaseStore, PendingResume, PendingSignal, PortableWakeDeadline, RunDriver,
    RunEvent, RunId, RunStore, RunStoreError, RunView, ScheduledTimer, StoreRunRecord, TimerStore,
    TimerStoreError, WaitPointId, WaitReason, WaitStore, WaitStoreError,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
struct RecordingBackend {
    runs: Mutex<HashMap<String, StoreRunRecord>>,
    resumes: Mutex<HashMap<(String, String), PendingResume>>,
    signals: Mutex<HashMap<String, Vec<PendingSignal>>>,
    timers: Mutex<Vec<ScheduledTimer>>,
    leases: Mutex<HashMap<String, LeaseGrant>>,
    driver_requests: Mutex<Vec<DriverRequest>>,
}

#[async_trait]
impl RunStore for RecordingBackend {
    async fn insert_run(&self, run: StoreRunRecord) -> Result<(), RunStoreError> {
        self.runs
            .lock()
            .await
            .insert(run.view.run_id().to_string(), run);
        Ok(())
    }

    async fn get_run(&self, run_id: &RunId) -> Result<Option<StoreRunRecord>, RunStoreError> {
        Ok(self.runs.lock().await.get(run_id.as_str()).cloned())
    }

    async fn put_run(&self, run: StoreRunRecord) -> Result<(), RunStoreError> {
        self.runs
            .lock()
            .await
            .insert(run.view.run_id().to_string(), run);
        Ok(())
    }
}

#[async_trait]
impl WaitStore for RecordingBackend {
    async fn save_resume(&self, resume: PendingResume) -> Result<(), WaitStoreError> {
        self.resumes.lock().await.insert(
            (resume.run_id.to_string(), resume.wait_point.to_string()),
            resume,
        );
        Ok(())
    }

    async fn take_resume(
        &self,
        run_id: &RunId,
        wait_point: &WaitPointId,
    ) -> Result<Option<PendingResume>, WaitStoreError> {
        Ok(self
            .resumes
            .lock()
            .await
            .remove(&(run_id.to_string(), wait_point.to_string())))
    }

    async fn push_signal(&self, signal: PendingSignal) -> Result<(), WaitStoreError> {
        self.signals
            .lock()
            .await
            .entry(signal.run_id.to_string())
            .or_default()
            .push(signal);
        Ok(())
    }

    async fn drain_signals(&self, run_id: &RunId) -> Result<Vec<PendingSignal>, WaitStoreError> {
        Ok(self
            .signals
            .lock()
            .await
            .remove(run_id.as_str())
            .unwrap_or_default())
    }
}

#[async_trait]
impl TimerStore for RecordingBackend {
    async fn schedule_timer(&self, timer: ScheduledTimer) -> Result<(), TimerStoreError> {
        self.timers.lock().await.push(timer);
        Ok(())
    }

    async fn cancel_timer(
        &self,
        run_id: &RunId,
        wait_point: &WaitPointId,
    ) -> Result<(), TimerStoreError> {
        self.timers
            .lock()
            .await
            .retain(|timer| timer.run_id != *run_id || timer.wait_point != *wait_point);
        Ok(())
    }

    async fn due_timers(
        &self,
        not_after: &PortableWakeDeadline,
        limit: usize,
    ) -> Result<Vec<ScheduledTimer>, TimerStoreError> {
        let timers = self.timers.lock().await;
        Ok(timers
            .iter()
            .filter(|timer| timer.wake_at.as_str() <= not_after.as_str())
            .take(limit)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl LeaseStore for RecordingBackend {
    async fn try_acquire_lease(
        &self,
        claim: LeaseClaim,
    ) -> Result<Option<LeaseGrant>, skg_run_core::LeaseError> {
        let mut leases = self.leases.lock().await;
        if leases.contains_key(claim.run_id.as_str()) {
            return Ok(None);
        }

        let grant = LeaseGrant::new(
            claim.run_id.clone(),
            claim.holder,
            claim.lease_until,
            "lease-1",
        );
        leases.insert(claim.run_id.to_string(), grant.clone());
        Ok(Some(grant))
    }

    async fn renew_lease(
        &self,
        grant: &LeaseGrant,
        lease_until: PortableWakeDeadline,
    ) -> Result<LeaseGrant, skg_run_core::LeaseError> {
        let mut leases = self.leases.lock().await;
        let Some(current) = leases.get_mut(grant.run_id.as_str()) else {
            return Err(skg_run_core::LeaseError::LeaseNotFound {
                run_id: grant.run_id.clone(),
            });
        };
        current.lease_until = lease_until.clone();
        Ok(current.clone())
    }

    async fn release_lease(&self, grant: &LeaseGrant) -> Result<(), skg_run_core::LeaseError> {
        self.leases.lock().await.remove(grant.run_id.as_str());
        Ok(())
    }
}

#[async_trait]
impl RunDriver for RecordingBackend {
    async fn drive_run(&self, request: DriverRequest) -> Result<DriverResponse, DriverError> {
        self.driver_requests.lock().await.push(request.clone());
        Ok(DriverResponse::new(
            RunEvent::Wait {
                wait_point: WaitPointId::new("wait-1"),
                reason: WaitReason::ExternalInput,
                wake_at: None,
            },
            Some(BackendRunRef::new("opaque-ref-2")),
        ))
    }
}

#[tokio::test]
async fn trait_shapes_cover_lower_level_backend_traits_and_keep_signal_resume_distinct() {
    let backend = Arc::new(RecordingBackend::default());
    let run_store: Arc<dyn RunStore> = backend.clone();
    let wait_store: Arc<dyn WaitStore> = backend.clone();
    let timer_store: Arc<dyn TimerStore> = backend.clone();
    let lease_store: Arc<dyn LeaseStore> = backend.clone();
    let driver: Arc<dyn RunDriver> = backend.clone();

    let run_id = RunId::new("run-123");
    let wait_point = WaitPointId::new("wait-1");
    let wake_at = PortableWakeDeadline::parse("2026-03-12T08:15:30Z").unwrap();
    let run = StoreRunRecord::new(
        RunView::waiting(
            run_id.clone(),
            wait_point.clone(),
            WaitReason::ExternalInput,
        ),
        Some(BackendRunRef::new("opaque-ref-1")),
    );

    run_store.insert_run(run.clone()).await.unwrap();
    let fetched = run_store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(fetched, run);
    assert!(
        run_store
            .get_run(&RunId::new("run-missing"))
            .await
            .unwrap()
            .is_none()
    );

    wait_store
        .save_resume(PendingResume::new(
            run_id.clone(),
            wait_point.clone(),
            skg_run_core::ResumeInput::new(json!({ "approved": true })),
        ))
        .await
        .unwrap();
    wait_store
        .push_signal(PendingSignal::new(
            run_id.clone(),
            json!({ "kind": "poke" }),
        ))
        .await
        .unwrap();

    let resume = wait_store.take_resume(&run_id, &wait_point).await.unwrap();
    assert!(resume.is_some());
    assert!(
        wait_store
            .take_resume(&run_id, &wait_point)
            .await
            .unwrap()
            .is_none()
    );
    let drained_signals = wait_store.drain_signals(&run_id).await.unwrap();
    assert_eq!(drained_signals.len(), 1);
    assert_eq!(drained_signals[0].signal, json!({ "kind": "poke" }));

    timer_store
        .schedule_timer(ScheduledTimer::new(
            run_id.clone(),
            wait_point.clone(),
            wake_at.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(timer_store.due_timers(&wake_at, 8).await.unwrap().len(), 1);
    timer_store
        .cancel_timer(&run_id, &wait_point)
        .await
        .unwrap();
    assert!(
        timer_store
            .due_timers(&wake_at, 8)
            .await
            .unwrap()
            .is_empty()
    );

    let grant = lease_store
        .try_acquire_lease(LeaseClaim::new(
            run_id.clone(),
            "worker-a",
            PortableWakeDeadline::parse("2026-03-12T08:20:30Z").unwrap(),
        ))
        .await
        .unwrap()
        .unwrap();
    let renewed = lease_store
        .renew_lease(
            &grant,
            PortableWakeDeadline::parse("2026-03-12T08:25:30Z").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(renewed.run_id, run_id);
    lease_store.release_lease(&renewed).await.unwrap();
    let renew_error = lease_store
        .renew_lease(
            &renewed,
            PortableWakeDeadline::parse("2026-03-12T08:30:30Z").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        renew_error,
        skg_run_core::LeaseError::LeaseNotFound { ref run_id } if *run_id == renewed.run_id
    ));

    let response = driver
        .drive_run(DriverRequest::new(
            run_id.clone(),
            DispatchPayload::Resume {
                wait_point: wait_point.clone(),
                input: skg_run_core::ResumeInput::new(json!({ "approved": true })),
            },
            run.backend_ref.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.backend_ref,
        Some(BackendRunRef::new("opaque-ref-2"))
    );
    assert!(matches!(
        response.next_event,
        RunEvent::Wait {
            wait_point: ref found,
            ..
        } if *found == wait_point
    ));
}
