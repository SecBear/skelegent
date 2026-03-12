use async_trait::async_trait;
use serde_json::{Value, json};
use skg_run_core::{
    ResumeInput, RunControlError, RunController, RunId, RunOutcome, RunStarter, RunStatus, RunView,
    WaitPointId, WaitReason,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn serde_round_trips_public_nouns() {
    let run_id = RunId::new("run-123");
    let run_id_json = serde_json::to_string(&run_id).unwrap();
    assert_eq!(serde_json::from_str::<RunId>(&run_id_json).unwrap(), run_id);

    let status = RunStatus::Waiting;
    let status_json = serde_json::to_string(&status).unwrap();
    assert_eq!(
        serde_json::from_str::<RunStatus>(&status_json).unwrap(),
        status
    );

    let reason = WaitReason::Custom("human-approval".to_owned());
    let reason_json = serde_json::to_string(&reason).unwrap();
    assert_eq!(
        serde_json::from_str::<WaitReason>(&reason_json).unwrap(),
        reason
    );

    let resume =
        ResumeInput::new(json!({ "approved": true })).with_metadata("source", json!("ops"));
    let resume_json = serde_json::to_string(&resume).unwrap();
    assert_eq!(
        serde_json::from_str::<ResumeInput>(&resume_json).unwrap(),
        resume
    );

    let waiting = RunView::waiting(
        RunId::new("run-123"),
        WaitPointId::new("wait-1"),
        WaitReason::ExternalInput,
    );
    let waiting_json = serde_json::to_string(&waiting).unwrap();
    assert_eq!(
        serde_json::from_str::<RunView>(&waiting_json).unwrap(),
        waiting
    );

    let completed = RunView::terminal(
        RunId::new("run-123"),
        RunOutcome::Completed {
            result: json!({ "ok": true }),
        },
    );
    let completed_json = serde_json::to_string(&completed).unwrap();
    assert_eq!(
        serde_json::from_str::<RunView>(&completed_json).unwrap(),
        completed
    );
}

#[tokio::test]
async fn traits_are_object_safe_and_keep_signal_distinct_from_resume() {
    #[derive(Default)]
    struct RecordingControl {
        signals: Mutex<Vec<(String, Value)>>,
        resumes: Mutex<Vec<(String, String, ResumeInput)>>,
        cancels: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl RunStarter for RecordingControl {
        async fn start_run(&self, input: Value) -> Result<RunId, skg_run_core::RunControlError> {
            assert_eq!(input, json!({ "job": "demo" }));
            Ok(RunId::new("run-123"))
        }
    }

    #[async_trait]
    impl RunController for RecordingControl {
        async fn get_run(&self, run_id: &RunId) -> Result<RunView, skg_run_core::RunControlError> {
            Ok(RunView::waiting(
                run_id.clone(),
                WaitPointId::new("wait-1"),
                WaitReason::ExternalInput,
            ))
        }

        async fn signal_run(
            &self,
            run_id: &RunId,
            signal: Value,
        ) -> Result<(), skg_run_core::RunControlError> {
            self.signals.lock().await.push((run_id.to_string(), signal));
            Ok(())
        }

        async fn resume_run(
            &self,
            run_id: &RunId,
            wait_point: &WaitPointId,
            input: ResumeInput,
        ) -> Result<(), skg_run_core::RunControlError> {
            self.resumes
                .lock()
                .await
                .push((run_id.to_string(), wait_point.to_string(), input));
            Ok(())
        }

        async fn cancel_run(&self, run_id: &RunId) -> Result<(), skg_run_core::RunControlError> {
            self.cancels.lock().await.push(run_id.to_string());
            Ok(())
        }
    }

    let control = Arc::new(RecordingControl::default());
    let starter: Box<dyn RunStarter> = Box::new(RecordingControl::default());
    let run_id = starter.start_run(json!({ "job": "demo" })).await.unwrap();
    assert_eq!(run_id, RunId::new("run-123"));

    let controller: Arc<dyn RunController> = control.clone();
    let view = controller.get_run(&run_id).await.unwrap();
    match view {
        RunView::Waiting {
            run_id: returned_run_id,
            wait_point,
            wait_reason,
        } => {
            assert_eq!(returned_run_id, RunId::new("run-123"));
            assert_eq!(wait_point, WaitPointId::new("wait-1"));
            assert_eq!(wait_reason, WaitReason::ExternalInput);
        }
        other => panic!("expected waiting run view, got {other:?}"),
    }

    controller
        .signal_run(&run_id, json!({ "kind": "poke" }))
        .await
        .unwrap();
    controller
        .resume_run(
            &run_id,
            &WaitPointId::new("wait-1"),
            ResumeInput::new(json!({ "answer": 42 })),
        )
        .await
        .unwrap();
    controller.cancel_run(&run_id).await.unwrap();

    assert_eq!(
        control.signals.lock().await.as_slice(),
        &[("run-123".to_owned(), json!({ "kind": "poke" }))]
    );
    assert_eq!(control.resumes.lock().await.len(), 1);
    assert_eq!(
        control.cancels.lock().await.as_slice(),
        &["run-123".to_owned()]
    );
}

#[tokio::test]
async fn get_run_uses_run_not_found_error() {
    struct MissingRunControl;

    #[async_trait]
    impl RunController for MissingRunControl {
        async fn get_run(&self, run_id: &RunId) -> Result<RunView, RunControlError> {
            Err(RunControlError::RunNotFound(run_id.clone()))
        }

        async fn signal_run(&self, _run_id: &RunId, _signal: Value) -> Result<(), RunControlError> {
            Ok(())
        }

        async fn resume_run(
            &self,
            _run_id: &RunId,
            _wait_point: &WaitPointId,
            _input: ResumeInput,
        ) -> Result<(), RunControlError> {
            Ok(())
        }

        async fn cancel_run(&self, _run_id: &RunId) -> Result<(), RunControlError> {
            Ok(())
        }
    }

    let control = MissingRunControl;
    let run_id = RunId::new("missing");
    let error = control.get_run(&run_id).await.unwrap_err();
    assert!(matches!(error, RunControlError::RunNotFound(found) if found == run_id));
}
