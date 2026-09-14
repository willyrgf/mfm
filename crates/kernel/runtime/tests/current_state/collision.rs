use super::*;
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_runtime::{AppendFailure, CandidatePresence, RecordingFailure};
use mfm_store::{AppendResult, LoadedRun, StoreError};
use std::{future::Future, pin::Pin};

#[derive(Clone, Copy)]
enum Collision {
    Present,
    Later,
    InvalidLatest,
    Excluded,
    Absent,
    ReloadFailure,
}
struct CompetingStore {
    inner: MemoryStore,
    collision: Collision,
    probes: std::sync::Mutex<Vec<Option<u64>>>,
}
impl Store for CompetingStore {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        self.probes.lock().unwrap().push(probe);
        if matches!(self.collision, Collision::ReloadFailure) {
            Box::pin(async { Err(StoreError::Unavailable) })
        } else {
            self.inner.load_run(run, probe)
        }
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            if frame.run_sequence() != 3 {
                return self.inner.append_run(frame).await;
            }
            match self.collision {
                Collision::Present | Collision::Later | Collision::InvalidLatest => {
                    self.inner.append_run(frame).await?;
                    if matches!(self.collision, Collision::Later | Collision::InvalidLatest) {
                        let invalid =
                            mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap();
                        let payload = if matches!(self.collision, Collision::InvalidLatest) {
                            &invalid
                        } else {
                            frame.payload()
                        };
                        let later =
                            seal_frame(frame.run_id(), 4, Some(frame.head_digest()), payload)
                                .unwrap();
                        self.inner.append_run(&later).await?;
                    }
                }
                Collision::Excluded => {
                    let mut payload: serde_json::Value =
                        serde_json::from_slice(frame.payload().as_bytes()).unwrap();
                    let other = serde_json::to_value(
                        mfm_values::Object::from_value(&Outage { deadline_ms: 732 }).unwrap(),
                    )
                    .unwrap();
                    payload["operation"]["failed"]["read"]["original"] = other;
                    let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
                        &serde_json::to_string(&payload).unwrap(),
                    )
                    .unwrap();
                    let other = seal_frame(
                        frame.run_id(),
                        frame.run_sequence(),
                        frame.previous_head_digest(),
                        &payload,
                    )
                    .unwrap();
                    self.inner.append_run(&other).await?;
                }
                Collision::Absent | Collision::ReloadFailure => {}
            }
            Ok(AppendResult::NotInserted)
        })
    }
}

#[tokio::test]
async fn candidate_probe_yields_latest_without_recovery_and_preserves_exclusion_or_reload_failure()
{
    for (index, collision) in [
        Collision::Present,
        Collision::Later,
        Collision::InvalidLatest,
        Collision::Excluded,
        Collision::Absent,
        Collision::ReloadFailure,
    ]
    .into_iter()
    .enumerate()
    {
        let store = Arc::new(CompetingStore {
            inner: MemoryStore::new(),
            collision,
            probes: std::sync::Mutex::new(Vec::new()),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = runtime(
            store.clone(),
            Arc::new(AtomicBool::new(false)),
            calls.clone(),
        );
        let input = Input {
            value: 9,
            continuation: "candidate custody".into(),
        };
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/current-collision@1").unwrap(),
            &Flow,
            &input,
            ProgramLimits::new(1),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([130 + index as u8; 32]));
        let result = runtime.start(run.clone(), program.clone(), input).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*store.probes.lock().unwrap(), vec![Some(3)]);
        match collision {
            Collision::Present | Collision::Later => {
                let observed = result.unwrap();
                assert_eq!(
                    observed.head_sequence(),
                    if matches!(collision, Collision::Later) {
                        4
                    } else {
                        3
                    }
                );
                assert!(matches!(
                    observed.state(),
                    RunViewState::AwaitingRecovery { .. }
                ));
                let repeated = runtime
                    .start(
                        run.clone(),
                        program,
                        Input {
                            value: 9,
                            continuation: "candidate custody".into(),
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(repeated.head_digest(), observed.head_digest());
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let different = Input {
                    value: 10,
                    continuation: "different admission".into(),
                };
                let different_program = mfm_program::expand_program(
                    EntryPointId::new("mfm.test/current-collision@1").unwrap(),
                    &Flow,
                    &different,
                    ProgramLimits::new(1),
                )
                .unwrap();
                assert!(matches!(
                    runtime
                        .start(run.clone(), different_program, different)
                        .await,
                    Err(InvocationFailure::Execution {
                        error: RuntimeError::AdmissionConflict,
                        ..
                    })
                ));
                assert_eq!(
                    runtime.read(&run).await.unwrap().head_digest(),
                    observed.head_digest()
                );
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
            Collision::Excluded
            | Collision::Absent
            | Collision::ReloadFailure
            | Collision::InvalidLatest => {
                let InvocationFailure::Execution {
                    error: RuntimeError::Recording { failure, .. },
                    last_observed: Some(observed),
                    ..
                } = result.err().unwrap()
                else {
                    panic!("recording conflict")
                };
                let RecordingFailure::Append {
                    original: Some(original),
                    candidate,
                    outcome: AppendFailure::NotInserted,
                    observation,
                    reload_cause,
                } = failure.as_ref()
                else {
                    panic!("known attempt disposition")
                };
                assert_eq!(original.downcast_ref::<Outage>().unwrap().deadline_ms, 731);
                assert_eq!(
                    decode_frame(candidate.canonical_bytes())
                        .unwrap()
                        .run_sequence(),
                    3
                );
                if matches!(collision, Collision::Excluded) {
                    assert!(matches!(
                        observation,
                        Some((_, CandidatePresence::Excluded))
                    ));
                    assert!(reload_cause.is_none());
                    assert_eq!(observed.head_sequence(), 3);
                    let RunViewState::AwaitingRecovery { failure } = observed.state() else {
                        panic!("winning original")
                    };
                    assert_eq!(
                        failure.original().decode::<Outage>().unwrap().deadline_ms,
                        732
                    );
                } else if matches!(collision, Collision::InvalidLatest) {
                    let Some((head, CandidatePresence::Present)) = observation else {
                        panic!("exact candidate survives failed current-record decoding")
                    };
                    assert_eq!(head.head_sequence(), 4);
                    assert_eq!(observed.head_sequence(), 2);
                    let RuntimeError::Native {
                        operation: mfm_runtime::Operation::Restore,
                        stage: mfm_runtime::Stage::Decode,
                        cause,
                    } = reload_cause
                        .as_ref()
                        .unwrap()
                        .downcast_ref::<RuntimeError>()
                        .unwrap()
                    else {
                        panic!("native reload failure retains its operation and stage")
                    };
                    let json = cause.downcast_ref::<mfm_canonical::JsonError>().unwrap();
                    let source = std::error::Error::source(json)
                        .unwrap()
                        .downcast_ref::<serde_json::Error>()
                        .unwrap();
                    assert!(source.is_data());
                    assert_eq!(source.line(), 1);
                    assert_eq!(source.column(), 2);
                } else if matches!(collision, Collision::Absent) {
                    let Some((head, CandidatePresence::Absent)) = observation else {
                        panic!("bound snapshot proves candidate absence")
                    };
                    assert_eq!(head.head_sequence(), 2);
                    assert!(reload_cause.is_none());
                    assert_eq!(observed.head_sequence(), 2);
                    assert!(matches!(observed.state(), RunViewState::Runnable { .. }));
                } else {
                    assert!(observation.is_none());
                    assert!(matches!(
                        reload_cause
                            .as_ref()
                            .unwrap()
                            .downcast_ref::<RuntimeError>(),
                        Some(RuntimeError::Store(StoreError::Unavailable))
                    ));
                    assert_eq!(observed.head_sequence(), 2);
                }
            }
        }
    }
}
