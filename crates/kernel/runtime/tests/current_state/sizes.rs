use super::*;
use mfm_runtime::{Operation, RecordingFailure, Stage};
use mfm_values::{InvocationDiagnostic, SizeLimitExceeded, ValueError};
use mfm_values::{SizeResource, SizeViolation};

#[test]
fn serialization_stop_reports_a_lower_bound_and_preserves_the_native_encoding_cause() {
    let source = mfm_canonical::to_json_bounded(&"a value longer than the ceiling", 8).unwrap_err();
    let error = RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause: ValueError::Canonical(source).into_diagnostic("encode"),
    };
    assert!(
        matches!(error.size_limit(), Some(SizeViolation::SerializationBound {
        resource: SizeResource::CanonicalObject, observed_at_least, limit: 8,
    }) if observed_at_least > 8)
    );
    let projection = serde_json::to_value(error.size_limit().unwrap()).unwrap();
    assert!(projection.get("actual").is_none());
    assert!(
        serde_json::to_value(&error).unwrap()["native"]["cause"]["details"]["canonical"]
            ["serialization_limit"]["source"]["message"]
            .is_string()
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
        failure: Box::new(RecordingFailure::NotInserted {
            original: None,
            candidate: candidate.clone(),
            observation: None,
            reload_cause: Some(Box::new(secondary)),
        }),
    };
    assert!(error.size_limit().is_none());
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::Store {
            original: None,
            candidate,
            cause: mfm_store::StoreError::HistorySize(size),
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

static ORIGINAL_SERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OversizedOriginal {
    detail: String,
}
impl Serialize for OversizedOriginal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.detail == "encoding-task-panic" {
            ORIGINAL_SERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        }
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
    ) -> Result<(), InvocationDiagnostic> {
        Observation::bind_evidence(reference, intent, evidence)
    }
}
impl ReadState<LargeObservation> for Read {
    fn prepare(input: &Input) -> Result<Request, InvocationDiagnostic> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
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
// A panicking original serializer must run only once; report known execution context and
// unavailable original details without advancing history.
#[tokio::test]
async fn unrecordable_original_reports_known_slot_and_encoding_cause_without_append() {
    ORIGINAL_SERIALIZATIONS.store(0, Ordering::SeqCst);
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<Read, LargeObservation>().unwrap();
    builder
        .register_adapter::<LargeObservation, _, _>(NoParams, move |_, _| {
            Box::pin(async move {
                Err(AdapterError::Operational(OversizedOriginal {
                    detail: "encoding-task-panic".into(),
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
    let RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause,
    } = error
    else {
        panic!("encoding is an ordinary internal failure")
    };
    assert_eq!(cause.operation(), "encode_failure");
    assert_eq!(ORIGINAL_SERIALIZATIONS.load(Ordering::SeqCst), 1);
    let _rendered = serde_json::to_value(&cause).unwrap();
    assert_eq!(ORIGINAL_SERIALIZATIONS.load(Ordering::SeqCst), 1);
    let fields = cause.details().as_value();
    assert_eq!(fields["encoding_target"], "declared_failure");
    assert_eq!(fields["original_detail"], "unavailable");
    assert_eq!(fields["original_identity"], "unavailable");
    assert_eq!(
        fields["failure_contract"],
        serde_json::to_value(mfm_program::nominal_contract_ref::<OversizedOriginal>().unwrap())
            .unwrap()
    );
    assert!(fields["position"].is_object());
    assert_eq!(cause.code(), "task_failure");
    assert_eq!(fields["encoding"], "panicked");
    let loaded = store.load_run(&run, None).await.unwrap().unwrap();
    assert_eq!(loaded.head().head_sequence(), 1);
    assert_eq!(loaded.head().head_digest(), observed.head_digest());
}

// An original rejected by value admission must not leak its forbidden text through the error or
// retained history.
#[tokio::test]
async fn rejected_operational_original_reports_unavailable_detail_without_append() {
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
    let encoded = serde_json::to_string(&error).unwrap();
    assert!(!encoded.contains("must-not-escape"));
    let RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause,
    } = error
    else {
        panic!("actual admission boundary retained")
    };
    assert_eq!(cause.code(), "value_error");
    assert_eq!(
        cause.details().as_value()["encoding"],
        "schema_shape_mismatch"
    );
    assert_eq!(cause.details().as_value()["original_detail"], "unavailable");
    assert_eq!(
        cause.details().as_value()["original_identity"],
        "unavailable"
    );
    let retained = store.load_run(&run, None).await.unwrap().unwrap();
    assert_eq!(retained.head().head_digest(), observed.head_digest());
    assert!(!std::str::from_utf8(retained.latest())
        .unwrap()
        .contains("must-not-escape"));
}
