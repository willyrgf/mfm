use super::validation::SnapshotStore;
use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_store::{AppendResult, LoadedRun, RunSummary, StoreError};
use mfm_values::{DiagnosticEvidence, Object};
use std::{future::Future, pin::Pin};

#[tokio::test]
async fn document_bootstrap_needs_no_installed_code_and_does_not_validate_current_continuation() {
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let (live, resources) = runtime(
        store.clone(),
        Arc::new(AtomicBool::new(true)),
        calls.clone(),
    );
    let input = Input {
        value: 9,
        continuation: "bootstrap".into(),
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/bootstrap@1").unwrap(),
        &Flow::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let reference = program.content_ref().clone();
    let bytes = program.canonical_bytes().to_vec();
    let run = RunId::from_digest(DigestBytes::from_array([185; 32]));
    let completed = live.start(run.clone(), &program, &input).await.unwrap();
    let loaded = store.load_run(&run, None).await.unwrap().unwrap();
    let cold = Runtime::new(store.clone());
    let document = cold.program_document(&run).await.unwrap();
    assert_eq!(document.value_ref(), &reference);
    assert_eq!(document.canonical_bytes(), bytes);
    struct Uninstalled;
    impl mfm_program::ProgramEnvironment for Uninstalled {
        type Sources = Identity<Input>;
    }
    assert!(mfm_program::load(document.canonical_bytes(), &Uninstalled).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        live.read(&run, &program).await.unwrap().head_digest(),
        completed.head_digest()
    );

    // A valid latest Journal envelope is not a checked Runtime continuation.
    let latest = decode_frame(loaded.latest()).unwrap();
    let changed = seal_frame(
        &run,
        latest.run_sequence(),
        latest.previous_head_digest(),
        &PlainCanonicalJsonBytes::from_json_str("{\"unsupported_continuation\":true}").unwrap(),
    )
    .unwrap();
    let snapshot = Arc::new(SnapshotStore {
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
    });
    let cold = Runtime::new(snapshot.clone());
    assert_eq!(cold.program_document(&run).await.unwrap(), document);
    let (with_code, resources) = runtime(snapshot, Arc::new(AtomicBool::new(true)), calls.clone());
    let program = mfm_program::load(program.canonical_bytes(), &resources).unwrap();
    assert!(matches!(
        with_code.read(&run, &program).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Native {
                stage: mfm_runtime::Stage::Decode,
                ..
            },
            ..
        })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn document_bootstrap_qualifies_admission_and_preserves_decode_causes() {
    let store = Arc::new(MemoryStore::new());
    let (live, resources) = runtime(
        store.clone(),
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicUsize::new(0)),
    );
    let input = Input {
        value: 9,
        continuation: "hostile bootstrap".into(),
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/bootstrap-rejection@1").unwrap(),
        &Flow::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([186; 32]));
    live.start(run.clone(), &program, &input).await.unwrap();
    let loaded = store.load_run(&run, None).await.unwrap().unwrap();
    let admission = decode_frame(loaded.admission()).unwrap();
    let payload: serde_json::Value =
        serde_json::from_slice(admission.payload().as_bytes()).unwrap();
    for mutation in [
        "program_ref",
        "variant",
        "parser",
        "run_id",
        "sequence",
        "head",
        "object",
    ] {
        let mut payload = payload.clone();
        match mutation {
            "program_ref" => {
                payload["program_ref"] =
                    serde_json::to_value(mfm_program::nominal_contract_ref::<Request>().unwrap())
                        .unwrap()
            }
            "variant" => {
                let latest = decode_frame(loaded.latest()).unwrap();
                let latest: serde_json::Value =
                    serde_json::from_slice(latest.payload().as_bytes()).unwrap();
                payload["operation"] = latest["operation"].clone();
            }
            "parser" => payload["unreviewed_admission_field"] = true.into(),
            "object" => {
                payload["operation"]["admitted"]["program"]["canonical"] =
                    serde_json::json!({"changed":true})
            }
            _ => {}
        }
        let other_run = RunId::from_digest(DigestBytes::from_array([187; 32]));
        let changed = seal_frame(
            if mutation == "run_id" {
                &other_run
            } else {
                &run
            },
            if mutation == "sequence" { 2 } else { 1 },
            if mutation == "sequence" {
                Some(admission.head_digest())
            } else {
                None
            },
            &PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&payload).unwrap())
                .unwrap(),
        )
        .unwrap();
        let snapshot = SnapshotStore {
            head: RunSummary::new(
                run.clone(),
                loaded.head().head_sequence(),
                if mutation == "head" {
                    admission.head_digest().clone()
                } else {
                    loaded.head().head_digest().clone()
                },
                loaded.head().total_bytes() - admission.canonical_bytes().len() as u64
                    + changed.canonical_bytes().len() as u64,
            )
            .unwrap(),
            admission: Arc::from(changed.canonical_bytes()),
            latest: loaded.latest().clone(),
        };
        let cold = Runtime::new(Arc::new(snapshot));
        let Err(InvocationFailure::Execution {
            run_id,
            error,
            last_observed: None,
        }) = cold.program_document(&run).await
        else {
            panic!("bootstrap must reject {mutation} without a fabricated observation");
        };
        assert_eq!(run_id, run);
        match mutation {
            "parser" | "object" => {
                let RuntimeError::Native {
                    operation: mfm_runtime::Operation::Restore,
                    stage: mfm_runtime::Stage::Decode,
                    cause,
                } = error
                else {
                    panic!("retained parser cause");
                };
                assert_eq!(cause.code(), "json_error");
                assert_eq!(cause.details().as_value()["category"], "data");
                assert_eq!(cause.details().as_value()["line"], 1);
                assert!(cause.details().as_value()["column"].as_u64().unwrap() > 0);
                let rendered = serde_json::to_string(cause.details()).unwrap();
                assert!(rendered.contains(if mutation == "parser" {
                    "unreviewed_admission_field"
                } else {
                    "content"
                }));
            }
            "program_ref" => {
                let RuntimeError::Native { cause, .. } = error else {
                    panic!("retained identity mismatch");
                };
                assert_eq!(cause.operation(), "admitted_program");
                assert_eq!(
                    cause.details().as_value()["identity"]["field"],
                    "program_ref"
                );
            }
            _ => assert!(matches!(error, RuntimeError::InvalidHistory)),
        }
    }

    // Program's decoder, rather than the bootstrap, owns the Program format.
    let replacement = Object::from_value(&Request { value: 42 }).unwrap();
    let mut payload = payload;
    payload["program_ref"] = serde_json::to_value(replacement.value_ref()).unwrap();
    payload["operation"]["admitted"]["program"] = serde_json::to_value(&replacement).unwrap();
    let changed = seal_frame(
        &run,
        1,
        None,
        &PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&payload).unwrap()).unwrap(),
    )
    .unwrap();
    let cold = Runtime::new(Arc::new(SnapshotStore {
        head: RunSummary::new(
            run.clone(),
            loaded.head().head_sequence(),
            loaded.head().head_digest().clone(),
            loaded.head().total_bytes() - admission.canonical_bytes().len() as u64
                + changed.canonical_bytes().len() as u64,
        )
        .unwrap(),
        admission: Arc::from(changed.canonical_bytes()),
        latest: loaded.latest().clone(),
    }));
    let document = cold.program_document(&run).await.unwrap();
    assert_eq!(document, replacement);
    assert!(mfm_program::load(document.canonical_bytes(), &resources).is_err());
}

struct FailedLoad;
impl Store for FailedLoad {
    fn load_run<'a>(
        &'a self,
        _: &'a RunId,
        _: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        Box::pin(async {
            Err(StoreError::Unavailable(DiagnosticEvidence::from_value(
                serde_json::json!({
                    "operation":"load_snapshot", "cause":{"operation":"read", "os_code":5}
                }),
            )))
        })
    }
    fn append_run<'a>(
        &'a self,
        _: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        panic!("bootstrap cannot append");
    }
}

#[tokio::test]
async fn missing_or_failed_bootstrap_retains_identity_and_store_cause_without_observation() {
    let run = RunId::from_digest(DigestBytes::from_array([188; 32]));
    let empty = Runtime::new(Arc::new(MemoryStore::new()));
    let Err(InvocationFailure::Execution {
        run_id,
        error: RuntimeError::Absent,
        last_observed: None,
    }) = empty.program_document(&run).await
    else {
        panic!("absent admission has no observed run");
    };
    assert_eq!(run_id, run);
    let failed = Runtime::new(Arc::new(FailedLoad));
    let Err(InvocationFailure::Execution {
        run_id,
        error: RuntimeError::Store(StoreError::Unavailable(cause)),
        last_observed: None,
    }) = failed.program_document(&run).await
    else {
        panic!("failed Store has no observed run");
    };
    assert_eq!(run_id, run);
    assert_eq!(
        cause.as_value(),
        &serde_json::json!({
            "operation":"load_snapshot", "cause":{"operation":"read", "os_code":5}
        })
    );
}
