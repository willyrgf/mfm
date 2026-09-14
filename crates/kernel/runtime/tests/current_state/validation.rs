use super::*;
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_store::{AppendResult, LoadedRun, RunSummary, StoreError};
use std::{future::Future, pin::Pin};

struct SnapshotStore {
    head: RunSummary,
    admission: Arc<[u8]>,
    latest: Arc<[u8]>,
}
impl Store for SnapshotStore {
    fn load_run<'a>(
        &'a self,
        _: &'a RunId,
        _: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            LoadedRun::new(
                self.head.clone(),
                self.admission.clone(),
                self.latest.clone(),
                None,
            )
            .map(Some)
        })
    }
    fn append_run<'a>(
        &'a self,
        _: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        panic!("inspection cannot append")
    }
}

#[tokio::test]
async fn cold_inspection_rejects_locally_inconsistent_current_records_without_callbacks() {
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let live = runtime(
        store.clone(),
        Arc::new(AtomicBool::new(true)),
        calls.clone(),
    );
    let input = Input {
        value: 9,
        continuation: "current record validation".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-validation@1").unwrap(),
        &Flow,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([155; 32]));
    live.start(run.clone(), program, input).await.unwrap();
    let loaded = store.load_run(&run, None).await.unwrap().unwrap();
    let latest = decode_frame(loaded.latest()).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(latest.payload().as_bytes()).unwrap();
    for mutation in 0..13 {
        let mut payload = payload.clone();
        match mutation {
            0 => {
                payload["state"]["usage"].as_array_mut().unwrap().pop();
            }
            1 => {
                payload["state"]["usage"][1]["retries"] = 2.into();
            }
            2 => {
                payload["state"]["effect_barrier"] = 0.into();
            }
            3 => {
                payload["facts"] = serde_json::json!({"recovery_retry": {}});
            }
            4 => {
                payload["state"]["checkpoints"] = serde_json::json!([{
                    "position": 0,
                    "input": payload["facts"]["succeeded"]["call"]["read"]["call"]["input"],
                }]);
            }
            5 => {
                payload["facts"]["succeeded"]["output"] = serde_json::to_value(
                    mfm_values::Object::from_value(&Input {
                        value: 99,
                        continuation: "different completed output".into(),
                    })
                    .unwrap(),
                )
                .unwrap();
            }
            6 => {
                payload["facts"]["succeeded"]["call"]["read"]["call"]["input"] =
                    serde_json::to_value(
                        mfm_values::Object::from_value(&Request { value: 9 }).unwrap(),
                    )
                    .unwrap();
            }
            7 => {
                payload["program_ref"] =
                    serde_json::to_value(mfm_program::nominal_contract_ref::<Request>().unwrap())
                        .unwrap();
            }
            8 => {
                payload["facts"]["succeeded"]["unexpected"] = true.into();
            }
            9 => {
                payload["facts"]["succeeded"]["call"]["read"]["unexpected"] = true.into();
            }
            10 | 11 => {
                payload["facts"]["succeeded"]["call"]["read"]["call"]["input"]["canonical"]
                    ["value"] = 99.into();
                if mutation == 11 {
                    payload["facts"]["succeeded"]["call"]["read"]["call"]["zz_unexpected"] =
                        true.into();
                }
            }
            12 => {
                payload["facts"]["succeeded"]["call"]["read"]["call"]["position"]["state"] =
                    999.into();
            }
            _ => unreachable!(),
        }
        let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&payload).unwrap(),
        )
        .unwrap();
        let changed = seal_frame(
            &run,
            latest.run_sequence(),
            latest.previous_head_digest(),
            &payload,
        )
        .unwrap();
        let snapshot = SnapshotStore {
            head: RunSummary::new(
                run.clone(),
                latest.run_sequence(),
                changed.head_digest().clone(),
                loaded.head().total_bytes() - latest.canonical_bytes().len() as u64
                    + changed.canonical_bytes().len() as u64,
            )
            .unwrap(),
            admission: loaded.admission().clone(),
            latest: Arc::from(changed.canonical_bytes()),
        };
        let cold = runtime(
            Arc::new(snapshot),
            Arc::new(AtomicBool::new(true)),
            calls.clone(),
        );
        let InvocationFailure::Execution {
            error:
                RuntimeError::Native {
                    operation: mfm_runtime::Operation::Restore,
                    cause,
                    ..
                },
            ..
        } = cold.read(&run).await.err().unwrap()
        else {
            panic!("mutation {mutation} must retain a native restoration cause")
        };
        let projected: serde_json::Value =
            serde_json::from_str(cause.project().unwrap().get()).unwrap();
        match mutation {
            0 | 1 => assert_eq!(projected, "usage"),
            2 => assert_eq!(projected, "barrier"),
            3 | 8 | 9 => assert!(cause.downcast_ref::<mfm_canonical::JsonError>().is_some()),
            4 => assert_eq!(projected, "checkpoint"),
            5 => assert_eq!(projected, "facts"),
            6 | 7 => assert!(projected.get("identity").is_some()),
            10 | 11 => {
                assert!(cause.downcast_ref::<mfm_canonical::JsonError>().is_some());
                assert_eq!(projected["category"], "data");
                assert!(projected["message"]
                    .as_str()
                    .unwrap()
                    .contains("content_digest"));
                assert!(projected["line"].as_u64().unwrap() > 0);
                assert!(projected["column"].as_u64().unwrap() > 0);
            }
            12 => assert_eq!(projected, "position"),
            _ => unreachable!(),
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn admission_and_latest_cannot_disagree_when_the_head_is_admission() {
    let run = RunId::from_digest(DigestBytes::from_array([156; 32]));
    let first = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap();
    let second = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("[]").unwrap();
    let admission = seal_frame(&run, 1, None, &first).unwrap();
    let latest = seal_frame(&run, 1, None, &second).unwrap();
    let snapshot = SnapshotStore {
        head: RunSummary::new(
            run.clone(),
            1,
            latest.head_digest().clone(),
            latest.canonical_bytes().len() as u64,
        )
        .unwrap(),
        admission: Arc::from(admission.canonical_bytes()),
        latest: Arc::from(latest.canonical_bytes()),
    };
    let cold = runtime(
        Arc::new(snapshot),
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicUsize::new(0)),
    );
    assert!(matches!(
        cold.read(&run).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::InvalidHistory,
            ..
        })
    ));
}

struct WideAllowances;
impl Operation for WideAllowances {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, Never>,
    ) -> mfm_program::Result<()> {
        body.allowances(RecoveryAllowances::new(u32::MAX, u32::MAX))?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())
    }
}

#[tokio::test]
async fn current_usage_checks_the_derived_sum_without_reconstructing_historical_grants() {
    for (index, limit, retries, restarts, accepted) in [
        (0, 3, 1, 2, true),
        (1, 3, 2, 2, false),
        (2, u32::MAX, u32::MAX - 1, 1, true),
        (3, u32::MAX, u32::MAX, 1, false),
    ] {
        let store = Arc::new(MemoryStore::new());
        let assembly = || {
            let mut builder = RuntimeAssemblyBuilder::new().unwrap();
            builder.register_pure::<Increment>().unwrap();
            builder.finish()
        };
        let input = Input {
            value: 9,
            continuation: "current usage bounds".into(),
        };
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/current-usage@1").unwrap(),
            &WideAllowances,
            &input,
            ProgramLimits::new(limit),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([170 + index; 32]));
        let live = Runtime::new(assembly(), store.clone());
        let completed = live.start(run.clone(), program, input).await.unwrap();
        let loaded = store.load_run(&run, None).await.unwrap().unwrap();
        let latest = decode_frame(loaded.latest()).unwrap();
        let mut payload: serde_json::Value =
            serde_json::from_slice(latest.payload().as_bytes()).unwrap();
        payload["state"]["usage"][0]["retries"] = retries.into();
        payload["state"]["usage"][1]["restarts"] = restarts.into();
        let canonical =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&payload.to_string()).unwrap();
        let changed = seal_frame(
            &run,
            latest.run_sequence(),
            latest.previous_head_digest(),
            &canonical,
        )
        .unwrap();
        let snapshot = SnapshotStore {
            head: RunSummary::new(
                run.clone(),
                latest.run_sequence(),
                changed.head_digest().clone(),
                loaded.head().total_bytes() - latest.canonical_bytes().len() as u64
                    + changed.canonical_bytes().len() as u64,
            )
            .unwrap(),
            admission: loaded.admission().clone(),
            latest: Arc::from(changed.canonical_bytes()),
        };
        let cold = Runtime::new(assembly(), Arc::new(snapshot));
        let result = cold.read(&run).await;
        if accepted {
            let observed = result.unwrap();
            let (RunViewState::Succeeded(actual), RunViewState::Succeeded(expected)) =
                (observed.state(), completed.state())
            else {
                panic!("locally valid usage does not change the completed output")
            };
            assert_eq!(actual.value_ref(), expected.value_ref());
        } else {
            let InvocationFailure::Execution {
                error:
                    RuntimeError::Native {
                        operation: mfm_runtime::Operation::Restore,
                        stage: mfm_runtime::Stage::Execute,
                        cause,
                    },
                ..
            } = result.err().unwrap()
            else {
                panic!("derived usage limit retains its validation cause")
            };
            assert_eq!(cause.project().unwrap().get(), "\"usage\"");
        }
    }
}
