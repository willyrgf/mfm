use crate::*;
use mfm_runtime::{AppendFailure, Operation, RecordingFailure};
use mfm_values::NativeCause;

#[derive(Debug, thiserror::Error)]
#[error("reviewed original")]
struct Original {
    code: u64,
}
#[derive(Debug, Serialize, thiserror::Error)]
#[error("reviewed projection failure")]
struct ProjectionFailure {
    code: u64,
}

#[test]
fn indeterminate_conversion_retains_original_candidate_and_typed_projection_failure() {
    let run_id = RunId::from_digest(mfm_ids::DigestBytes::from_array([77; 32]));
    let candidate = mfm_journal::seal_frame(
        &run_id,
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let expected_bytes = candidate.canonical_bytes().to_vec();
    let failure = RunRequestError::from_invocation(
        InvocationFailure::Execution {
            run_id: run_id.clone(),
            last_observed: None,
            error: RuntimeError::Recording {
                operation: Operation::ReadAdapter,
                failure: Box::new(RecordingFailure::Append {
                    original: Some(NativeCause::from_error_with(
                        Original { code: 71 },
                        |original| {
                            Err(NativeCause::from_error(ProjectionFailure {
                                code: original.code + 1,
                            }))
                        },
                    )),
                    candidate,
                    outcome: AppendFailure::Store(mfm_store::StoreError::Indeterminate),
                    observation: None,
                    reload_cause: None,
                }),
            },
        },
        RunRecovery::Progress {
            run_id: run_id.clone(),
        },
    );
    assert_eq!(failure.code(), "run_append_indeterminate");
    let projection_failure =
        SerializableClientError::for_run(&failure, "run append outcome is indeterminate")
            .err()
            .unwrap();
    assert_eq!(
        projection_failure
            .downcast_ref::<ProjectionFailure>()
            .unwrap()
            .code,
        72
    );
    let RunRequestError::AppendIndeterminate {
        recovery: RunRecovery::Progress {
            run_id: retained_id,
        },
        invocation:
            InvocationFailure::Execution {
                error: RuntimeError::Recording { failure, .. },
                ..
            },
    } = &failure
    else {
        panic!("complete ambiguous invocation")
    };
    assert_eq!(retained_id, &run_id);
    let RecordingFailure::Append {
        original: Some(original),
        candidate,
        outcome: AppendFailure::Store(mfm_store::StoreError::Indeterminate),
        ..
    } = failure.as_ref()
    else {
        panic!("original and physical outcome")
    };
    assert_eq!(original.downcast_ref::<Original>().unwrap().code, 71);
    assert_eq!(candidate.canonical_bytes(), expected_bytes);
}

#[test]
fn noninsertion_keeps_its_category_when_the_secondary_probe_has_a_different_failure() {
    let run_id = RunId::from_digest(mfm_ids::DigestBytes::from_array([78; 32]));
    let candidate = mfm_journal::seal_frame(
        &run_id,
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let failure = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id,
        last_observed: None,
        error: RuntimeError::Recording {
            operation: Operation::Record,
            failure: Box::new(RecordingFailure::Append {
                original: None,
                candidate,
                outcome: AppendFailure::NotInserted,
                observation: None,
                reload_cause: Some(
                    RuntimeError::Store(mfm_store::StoreError::Unavailable).into_native(),
                ),
            }),
        },
    });
    assert_eq!(
        failure.request_error(),
        Some(RequestError::RunAppendNotInserted)
    );
    let model = SerializableClientError::for_run(&failure, "run append was not inserted").unwrap();
    let json = serde_json::to_value(model).unwrap();
    assert_eq!(json["code"], "run_append_not_inserted");
    assert_eq!(
        json["invocation"]["cause"]["recording"]["failure"]["append"]["reload_cause"]["store"],
        "unavailable"
    );
}

#[test]
fn incomplete_report_retains_both_failures_and_does_not_retry_failed_projection() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[derive(Debug, thiserror::Error)]
    #[error("reviewed reporting failure")]
    struct Unprojectable(Arc<AtomicUsize>);

    let attempts = Arc::new(AtomicUsize::new(0));
    let cause = NativeCause::from_error_with(Unprojectable(attempts.clone()), |owner| {
        owner.0.fetch_add(1, Ordering::SeqCst);
        Err(NativeCause::from_error(ProjectionFailure { code: 91 }))
    });
    let original = RunRequestError::Request(RequestError::RunAppendNotInserted);
    let mut failure = ReportFailure::new(original, ReportStage::Prepare, cause);
    for _ in 0..2 {
        let bytes = encode_response(&failure.incomplete()).unwrap();
        let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["code"], "run_append_not_inserted");
        assert_eq!(report["report_failure"]["stage"], "prepare");
        assert!(report["report_failure"].get("cause").is_none());
        assert_eq!(
            report["report_failure"]["omissions"],
            serde_json::json!([
                {"field": "original_detail", "reason": "projection_failed"},
                {"field": "report_failure.cause", "reason": "projection_failed"}
            ])
        );
        assert!(report.get("acknowledged").is_none());
        assert!(report.get("last_observed").is_none());
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(failure.original().code(), "run_append_not_inserted");
    assert!(failure.cause().downcast_ref::<Unprojectable>().is_some());
    assert_eq!(
        failure
            .projection_failure()
            .unwrap()
            .downcast_ref::<ProjectionFailure>()
            .unwrap()
            .code,
        91
    );
}

#[test]
fn incomplete_report_projects_reviewed_cause_without_replacing_primary_category() {
    let mut failure = ReportFailure::new(
        RunRequestError::Request(RequestError::RunAppendNotInserted),
        ReportStage::Encode,
        NativeCause::from_error(ProjectionFailure { code: 92 }),
    );
    let bytes = encode_response(&failure.incomplete()).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["code"], "run_append_not_inserted");
    assert_eq!(report["report_failure"]["stage"], "encode");
    assert_eq!(report["report_failure"]["cause"]["code"], 92);
    assert_eq!(
        report["report_failure"]["omissions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(failure.projection_failure().is_none());
}

#[test]
fn incomplete_report_labels_a_stopped_serializer_measurement_as_a_lower_bound() {
    let cause = mfm_canonical::to_json_bounded(&"abc", 4).unwrap_err();
    let mut failure = ReportFailure::new(
        RunRequestError::Request(RequestError::RunAppendNotInserted),
        ReportStage::Prepare,
        NativeCause::from_error(cause),
    );
    let bytes = encode_response(&failure.incomplete()).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["code"], "run_append_not_inserted");
    assert_eq!(
        report["report_failure"]["omissions"][0],
        serde_json::json!({
            "field": "original_detail",
            "reason": "bound_reached",
            "limit": 4,
            "observed_at_least": 5
        })
    );
    assert!(failure
        .cause()
        .downcast_ref::<mfm_canonical::CanonicalError>()
        .is_some());
    assert!(failure.projection_failure().is_none());
}

#[tokio::test]
async fn panicking_original_projection_retains_invocation_and_last_observed_without_retry() {
    use mfm_program::{Never, NoParams, OperationExpansion, ProgramLimits};
    use mfm_runtime::{Runtime, RuntimeAssemblyBuilder, Stage, TaskFailure};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Empty;
    impl mfm_program::Operation for Empty {
        type Input = NoParams;
        type Output = NoParams;
        type Failure = Never;
        fn validate_input(&self, _: &NoParams) -> mfm_program::Result<()> {
            Ok(())
        }
        fn expand(
            &self,
            _: &mut OperationExpansion<NoParams, NoParams, Never>,
        ) -> mfm_program::Result<()> {
            Ok(())
        }
    }
    #[derive(Debug, thiserror::Error)]
    #[error("original encoder failed")]
    struct PanickingOriginal {
        code: u64,
        attempts: Arc<AtomicUsize>,
    }

    let run_id = RunId::from_digest(mfm_ids::DigestBytes::from_array([81; 32]));
    let store = Arc::new(mfm_store::MemoryStore::new());
    let runtime = Runtime::new(RuntimeAssemblyBuilder::new().unwrap().finish(), store);
    let program = mfm_program::expand_program(
        mfm_ids::EntryPointId::new("mfm.test/report-projection-panic@1").unwrap(),
        &Empty,
        &NoParams,
        ProgramLimits::new(0),
    )
    .unwrap();
    let observed = runtime
        .start(run_id.clone(), program, NoParams)
        .await
        .unwrap();
    let head_digest = observed.head_digest().clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let original = NativeCause::from_error_with(
        PanickingOriginal {
            code: 93,
            attempts: attempts.clone(),
        },
        |owner| {
            owner.attempts.fetch_add(1, Ordering::SeqCst);
            panic!("injected native projection panic");
        },
    );
    let original = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: run_id.clone(),
        last_observed: Some(observed),
        error: RuntimeError::Recording {
            operation: Operation::ReadAdapter,
            failure: Box::new(RecordingFailure::BeforeAppend {
                original: Some(original),
                candidate: None,
                cause: RuntimeError::Native {
                    operation: Operation::ReadAdapter,
                    stage: Stage::Execute,
                    cause: NativeCause::from_error(TaskFailure::Panicked),
                }
                .into_native(),
            }),
        },
    });
    let cause = SerializableClientError::for_run(&original, "execution stopped")
        .err()
        .unwrap();
    let mut failure = ReportFailure::new(original, ReportStage::Prepare, cause);
    for _ in 0..2 {
        let bytes = encode_response(&failure.incomplete()).unwrap();
        let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["code"], "internal");
        assert_eq!(
            report["last_observed"]["head_digest"],
            serde_json::to_value(&head_digest).unwrap()
        );
        assert!(report.get("acknowledged").is_none());
        assert_eq!(
            report["report_failure"]["cause"],
            serde_json::json!({
                "operation": "native_projection", "outcome": "panicked",
                "omissions": [{"field": "panic_payload", "reason": "withheld"}],
            })
        );
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("injected native projection panic"));
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: retained_id,
        last_observed: Some(observed),
        error: RuntimeError::Recording { failure, .. },
    }) = failure.original()
    else {
        panic!("complete original invocation remains in custody");
    };
    assert_eq!(retained_id, &run_id);
    assert_eq!(observed.head_digest(), &head_digest);
    let RecordingFailure::BeforeAppend {
        original: Some(original),
        candidate: None,
        ..
    } = failure.as_ref()
    else {
        panic!("unsealed original remains available");
    };
    assert_eq!(
        original.downcast_ref::<PanickingOriginal>().unwrap().code,
        93
    );
}
