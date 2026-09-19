use super::*;
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_runtime::{CandidatePresence, RecordingFailure};
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
            Box::pin(async {
                Err(StoreError::Unavailable(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.store", "injected": "Unavailable"}),
                    ),
                ))
            })
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

// After a rejected append, distinguish the exact candidate from a competing record and preserve
// reload errors without running recovery.
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
        let (runtime, resources) = runtime(
            store.clone(),
            Arc::new(AtomicBool::new(false)),
            calls.clone(),
        );
        let input = Input {
            value: 9,
            continuation: "candidate custody".into(),
        };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/current-collision@1").unwrap(),
            &Flow::default(),
            &input,
            &resources,
            ProgramLimits::new(1),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([130 + index as u8; 32]));
        let result = runtime.start(run.clone(), &program, &input).await;
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
                        &program,
                        &Input {
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
                let different_program = mfm_program::compile(
                    EntryPointId::new("mfm.test/current-collision@1").unwrap(),
                    &Flow::default(),
                    &different,
                    &resources,
                    ProgramLimits::new(1),
                )
                .unwrap();
                let conflict = runtime
                    .start(run.clone(), &different_program, &different)
                    .await
                    .err()
                    .unwrap();
                let InvocationFailure::Execution {
                    error:
                        RuntimeError::Native {
                            operation: mfm_runtime::Operation::Restore,
                            stage: mfm_runtime::Stage::Decode,
                            cause,
                        },
                    last_observed: None,
                    ..
                } = conflict
                else {
                    panic!("different complete Program must reject before observation")
                };
                assert_eq!(cause.operation(), "restore_frames");
                let identity = &cause.details().as_value()["identity"];
                assert_eq!(identity["field"], "program_ref");
                assert_eq!(
                    identity["expected"],
                    serde_json::to_value(different_program.content_ref()).unwrap()
                );
                assert_eq!(
                    identity["actual"],
                    serde_json::to_value(program.content_ref()).unwrap()
                );
                assert_eq!(
                    runtime.read(&run, &program).await.unwrap().head_digest(),
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
                let RecordingFailure::NotInserted {
                    original: Some(original),
                    candidate,
                    observation,
                    reload_cause,
                } = failure.as_ref()
                else {
                    panic!("known attempt disposition")
                };
                assert_eq!(
                    original.original().decode::<Outage>().unwrap().deadline_ms,
                    731
                );
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
                    } = reload_cause.as_deref().unwrap()
                    else {
                        panic!("native reload failure retains its operation and stage")
                    };
                    assert_eq!(cause.code(), "json_error");
                    assert_eq!(cause.details().as_value()["category"], "data");
                    assert_eq!(cause.details().as_value()["line"], 1);
                    assert_eq!(cause.details().as_value()["column"], 2);
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
                        reload_cause.as_deref(),
                        Some(RuntimeError::Store(StoreError::Unavailable(_)))
                    ));
                    assert_eq!(observed.head_sequence(), 2);
                }
            }
        }
    }
}
