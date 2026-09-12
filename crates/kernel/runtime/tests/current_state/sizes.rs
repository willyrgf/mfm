use super::*;
use mfm_runtime::{AppendFailure, Operation, RecordingFailure, SizeResource, SizeViolation, Stage};
use mfm_values::{NativeCause, SizeLimitExceeded, ValueError};

#[test]
fn serialization_stop_reports_a_lower_bound_and_preserves_the_native_encoding_cause() {
    let source = mfm_canonical::to_json_bounded(&"a value longer than the ceiling", 8).unwrap_err();
    let error = RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause: NativeCause::from_error(ValueError::Canonical(source)),
    };
    assert!(
        matches!(error.size_limit(), Some(SizeViolation::SerializationBound {
        resource: SizeResource::CanonicalObject, observed_at_least, limit: 8,
    }) if observed_at_least > 8)
    );
    let projection = serde_json::to_value(error.size_limit().unwrap()).unwrap();
    assert!(projection.get("actual").is_none());
    let RuntimeError::Native { cause, .. } = &error else {
        unreachable!()
    };
    let ValueError::Canonical(source) = cause.downcast_ref::<ValueError>().unwrap() else {
        unreachable!()
    };
    assert!(std::error::Error::source(source).is_some());
    assert!(
        serde_json::from_str::<serde_json::Value>(error.project().unwrap().get()).unwrap()
            ["native"]["cause"]["canonical"]["serialization_limit"]
            .is_object()
    );
}

#[test]
fn append_size_projection_uses_the_physical_outcome_and_ignores_secondary_reload_errors() {
    let run = RunId::from_digest(DigestBytes::from_array([157; 32]));
    let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap();
    let candidate = mfm_journal::seal_frame(&run, 1, None, &payload).unwrap();
    let size = SizeLimitExceeded::check(11, 10).unwrap_err();
    let secondary = RuntimeError::SizeLimit {
        resource: SizeResource::Frame,
        size,
    };
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::Append {
            original: None,
            candidate: candidate.clone(),
            outcome: AppendFailure::NotInserted,
            observation: None,
            reload_cause: Some(secondary.into_native()),
        }),
    };
    assert!(error.size_limit().is_none());
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::Append {
            original: None,
            candidate,
            outcome: AppendFailure::Store(mfm_store::StoreError::HistorySize(size)),
            observation: None,
            reload_cause: None,
        }),
    };
    assert_eq!(
        error.size_limit(),
        Some(SizeViolation::Measured {
            resource: SizeResource::HistoryBytes,
            actual: 11,
            limit: 10,
        })
    );
}

#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OversizedOriginal {
    detail: String,
}
impl Serialize for OversizedOriginal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        assert_ne!(
            self.detail, "encoding-task-panic",
            "injected encoding task failure"
        );
        let mut value = serializer.serialize_struct("OversizedOriginal", 1)?;
        value.serialize_field("detail", &self.detail)?;
        value.end()
    }
}
impl ClassifyError for OversizedOriginal {
    fn classify(&self) -> Classification {
        panic!("unrecorded original cannot enter policy")
    }
}
struct LargeObservation;
impl ReadCapabilityContract for LargeObservation {
    type Intent = Request;
    type Evidence = Request;
    type OperationalError = OversizedOriginal;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-large-observation@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        reference: &ContentRef,
        intent: &Request,
        evidence: &Request,
    ) -> Result<(), NativeCause> {
        Observation::bind_evidence(reference, intent, evidence)
    }
}
impl ReadState<LargeObservation> for Read {
    fn prepare(input: &Input) -> Result<Request, NativeCause> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl mfm_program::CapabilityInjection<Read> for LargeObservation {
    type Setup = NoParams;
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct LargeFlow;
impl mfm_program::Operation for LargeFlow {
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
        body.read::<Read, LargeObservation, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}
#[tokio::test]
async fn unrecordable_original_survives_encoding_bound_and_task_failure_without_append() {
    for panics in [false, true] {
        let store = Arc::new(MemoryStore::new());
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_read::<Read, LargeObservation>().unwrap();
        builder
            .register_adapter::<LargeObservation, _, _>(NoParams, move |_, _| {
                Box::pin(async move {
                    Err(AdapterError::Operational(OversizedOriginal {
                        detail: if panics {
                            "encoding-task-panic".into()
                        } else {
                            "a".repeat(mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES)
                        },
                    }))
                })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), store.clone());
        let input = Input {
            value: 9,
            continuation: "actual size rejection".into(),
        };
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/current-oversize@1").unwrap(),
            &LargeFlow,
            &input,
            ProgramLimits::new(1),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([159; 32]));
        let InvocationFailure::Execution {
            error,
            last_observed: Some(observed),
            ..
        } = runtime
            .start(run.clone(), program, input)
            .await
            .err()
            .unwrap()
        else {
            panic!("execution stopped")
        };
        assert_eq!(observed.head_sequence(), 1);
        if !panics {
            assert!(matches!(
                error.size_limit(),
                Some(SizeViolation::SerializationBound {
                    resource: SizeResource::CanonicalObject,
                    ..
                })
            ));
        }
        let RuntimeError::Recording { failure, .. } = error else {
            panic!("original custody")
        };
        let RecordingFailure::BeforeAppend {
            original: Some(original),
            candidate: None,
            cause,
        } = failure.as_ref()
        else {
            panic!("no fabricated candidate")
        };
        if panics {
            assert_eq!(
                original.downcast_ref::<OversizedOriginal>().unwrap().detail,
                "encoding-task-panic"
            );
            let Some(RuntimeError::Native {
                operation: mfm_runtime::Operation::ReadAdapter,
                stage: mfm_runtime::Stage::Execute,
                cause,
            }) = cause.downcast_ref::<RuntimeError>()
            else {
                panic!("encoding task failure retains its operation and reviewed outcome");
            };
            assert!(matches!(
                cause.downcast_ref::<mfm_runtime::TaskFailure>(),
                Some(mfm_runtime::TaskFailure::Panicked)
            ));
            let projection = original.project().unwrap_err();
            let projection: serde_json::Value =
                serde_json::from_str(projection.project().unwrap().get()).unwrap();
            assert_eq!(projection["operation"], "native_projection");
            assert_eq!(projection["outcome"], "panicked");
            assert_eq!(
                projection["omissions"],
                serde_json::json!([
                    {"field": "panic_payload", "reason": "withheld"}
                ])
            );
        } else {
            assert_eq!(
                original
                    .downcast_ref::<OversizedOriginal>()
                    .unwrap()
                    .detail
                    .len(),
                mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES
            );
            assert!(cause.downcast_ref::<RuntimeError>().is_some());
            let projection_error = original.project().unwrap_err();
            let ValueError::Canonical(cause) =
                projection_error.downcast_ref::<ValueError>().unwrap()
            else {
                panic!("original projection preserves Values' encoding boundary")
            };
            assert!(cause.serialization_bound().is_some());
            assert_eq!(
                original
                    .downcast_ref::<OversizedOriginal>()
                    .unwrap()
                    .detail
                    .len(),
                mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES
            );
        }
        let loaded = store.load_run(&run, None).await.unwrap().unwrap();
        assert_eq!(loaded.head().head_sequence(), 1);
        assert_eq!(loaded.head().head_digest(), observed.head_digest());
    }
}

#[derive(Debug, thiserror::Error)]
#[error("reviewed owner projection stopped")]
struct UnprojectableOwner {
    reviewed_code: u64,
}
#[derive(Debug, Serialize, thiserror::Error)]
#[error("reviewed projection failure")]
struct OwnerProjectionFailure {
    reviewed_code: u64,
}

#[test]
fn nested_runtime_projection_returns_the_native_child_failure_without_losing_its_owner() {
    let nested = RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Execute,
        cause: NativeCause::from_error_with(UnprojectableOwner { reviewed_code: 41 }, |owner| {
            Err(NativeCause::from_error(OwnerProjectionFailure {
                reviewed_code: owner.reviewed_code + 1,
            }))
        }),
    };
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::BeforeAppend {
            original: None,
            candidate: None,
            cause: nested.into_native(),
        }),
    };
    let projection_failure = error.project().unwrap_err();
    assert_eq!(
        projection_failure
            .downcast_ref::<OwnerProjectionFailure>()
            .unwrap()
            .reviewed_code,
        42
    );
    let RuntimeError::Recording { failure, .. } = &error else {
        panic!("recording custody")
    };
    let RecordingFailure::BeforeAppend { cause, .. } = failure.as_ref() else {
        panic!("before append")
    };
    let RuntimeError::Native { cause, .. } = cause.downcast_ref::<RuntimeError>().unwrap() else {
        panic!("nested runtime")
    };
    assert_eq!(
        cause
            .downcast_ref::<UnprojectableOwner>()
            .unwrap()
            .reviewed_code,
        41
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(projection_failure.project().unwrap().get())
            .unwrap()["reviewed_code"],
        42
    );
}

#[tokio::test]
async fn rejected_operational_original_cannot_escape_through_recording_failure_projection() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<Read, LargeObservation>().unwrap();
    builder
        .register_adapter::<LargeObservation, _, _>(NoParams, |_, _| {
            Box::pin(async {
                Err(AdapterError::Operational(OversizedOriginal {
                    detail: "api_key=must-not-escape".into(),
                }))
            })
        })
        .unwrap();
    let runtime = Runtime::new(builder.finish(), store.clone());
    let input = Input {
        value: 9,
        continuation: "rejected original custody".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-rejected-original@1").unwrap(),
        &LargeFlow,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([163; 32]));
    let InvocationFailure::Execution {
        error,
        last_observed: Some(observed),
        ..
    } = runtime
        .start(run.clone(), program, input)
        .await
        .err()
        .unwrap()
    else {
        panic!("original violates value admission")
    };
    assert_eq!(observed.head_sequence(), 1);
    let projection_error = error.project().unwrap_err();
    assert!(matches!(
        projection_error.downcast_ref::<ValueError>(),
        Some(ValueError::SchemaShapeMismatch)
    ));
    assert_eq!(
        projection_error.project().unwrap().get(),
        "\"schema_shape_mismatch\""
    );
    let RuntimeError::Recording { failure, .. } = error else {
        panic!("native original remains in recording custody")
    };
    let RecordingFailure::BeforeAppend {
        original: Some(original),
        candidate: None,
        cause,
    } = failure.as_ref()
    else {
        panic!("no candidate or policy decision")
    };
    assert_eq!(
        original.downcast_ref::<OversizedOriginal>().unwrap().detail,
        "api_key=must-not-escape"
    );
    let RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause,
    } = cause.downcast_ref::<RuntimeError>().unwrap()
    else {
        panic!("actual admission boundary retained")
    };
    assert!(matches!(
        cause.downcast_ref::<ValueError>(),
        Some(ValueError::SchemaShapeMismatch)
    ));
    let retained = store.load_run(&run, None).await.unwrap().unwrap();
    assert_eq!(retained.head().head_digest(), observed.head_digest());
    assert!(!std::str::from_utf8(retained.latest())
        .unwrap()
        .contains("must-not-escape"));
}
