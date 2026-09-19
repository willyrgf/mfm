use super::*;
use mfm_store::{AppendResult, LoadedRun, StoreError};
use mfm_values::{SizeLimitExceeded, SizeResource, SizeViolation};

// A small physical Store ceiling exercises the invocation boundary without allocating the
// production maximum or claiming that this fixture verifies MemoryStore's configured limit.
struct LimitedStore(MemoryStore);
impl Store for LimitedStore {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Option<LoadedRun>, StoreError>> + Send + 'a>>
    {
        self.0.load_run(run, probe)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            SizeLimitExceeded::check(frame.run_sequence(), 2).map_err(StoreError::FrameCount)?;
            self.0.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn physical_capacity_rejection_preserves_original_or_prepared_authority_without_terminal_success(
) {
    for effect in [false, true] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resources = Resources {
            calls: calls.clone(),
            operational: false,
            settled: true,
        };
        let entry = EntryPointId::new("mfm.test/small-capacity@1").unwrap();
        let program = if effect {
            mfm_program::compile(
                entry,
                &mfm_program::Operation::new(Flow::<false>),
                &NoParams,
                &resources,
                ProgramLimits::new(0),
            )
        } else {
            mfm_program::compile(
                entry,
                &mfm_program::Pure::<Fail>::default(),
                &NoParams,
                &resources,
                ProgramLimits::new(0),
            )
        }
        .unwrap();
        let store = Arc::new(LimitedStore(MemoryStore::new()));
        let runtime = Runtime::new(store);
        let run = RunId::from_digest(DigestBytes::from_array([130 + u8::from(effect); 32]));
        let Err(InvocationFailure::Execution {
            run_id,
            error,
            last_observed: Some(observed),
        }) = runtime.execute(run.clone(), &program, &NoParams).await
        else {
            panic!("capacity rejection must preserve the acknowledged prefix")
        };
        assert_eq!(run_id, run);
        assert_eq!(observed.head_sequence(), 2);
        assert_eq!(
            error.size_limit(),
            Some(SizeViolation::Measured {
                resource: SizeResource::FrameCount,
                actual: 3,
                limit: 2
            })
        );
        let RuntimeError::Recording { failure, .. } = error else {
            panic!("retain recording rejection")
        };
        let crate::RecordingFailure::Store {
            candidate,
            cause: StoreError::FrameCount(size),
            ..
        } = failure.as_ref()
        else {
            panic!("retain exact Store size cause")
        };
        assert_eq!(candidate.run_sequence(), 3);
        assert_eq!(*size, SizeLimitExceeded::check(3, 2).unwrap_err());
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let program = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
        let cold = runtime.read(&run, &program).await.unwrap();
        assert_eq!(cold.head_digest(), observed.head_digest());
        assert!(cold.success().is_none());
        assert!(cold.failure().is_none());
        if effect {
            let RunViewState::EffectPending {
                effect,
                latest_failure: None,
            } = cold.state()
            else {
                panic!("unrecorded settlement cannot erase command authority")
            };
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[effect.effect_id().clone()]
            );
        } else {
            let RunViewState::AwaitingRecovery { failure } = cold.state() else {
                panic!("original remains durable when recovery cannot fit")
            };
            failure.original().decode::<Outage>().unwrap();
            assert!(calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn report_encoding_bound_preserves_the_durable_original_before_terminal_append() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resources = Resources {
        calls: calls.clone(),
        operational: false,
        settled: true,
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/report-capacity@1").unwrap(),
        &mfm_program::Pure::<Fail>::default(),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([132; 32]));
    let runtime = Runtime::new(Arc::new(MemoryStore::new()));
    // Exercise the actual report encoder and pre-append check with a small policy value.
    *crate::report::REPORT_ENCODING_LIMIT.lock().unwrap() = Some((run.clone(), 128));
    let result = runtime.execute(run.clone(), &program, &NoParams).await;
    *crate::report::REPORT_ENCODING_LIMIT.lock().unwrap() = None;
    let Err(InvocationFailure::Execution {
        run_id,
        error,
        last_observed: Some(observed),
    }) = result
    else {
        panic!("oversized report must stop before terminal append")
    };
    assert_eq!(run_id, run);
    assert_eq!(observed.head_sequence(), 2);
    assert!(
        matches!(error.size_limit(), Some(SizeViolation::SerializationBound {
        resource: SizeResource::FailureReport, limit: 128, observed_at_least,
    }) if observed_at_least > 128)
    );
    let RuntimeError::Recording { failure, .. } = error else {
        panic!("retain the failed recording boundary")
    };
    let crate::RecordingFailure::BeforeAppend { original, cause } = *failure else {
        panic!("report rejection must precede Store append")
    };
    original.original().decode::<Outage>().unwrap();
    let RuntimeError::Native {
        operation: Operation::Project,
        stage: Stage::Encode,
        cause,
    } = *cause
    else {
        panic!("retain the report encoding cause")
    };
    assert_eq!(cause.operation(), "new");
    let document = runtime.program_document(&run).await.unwrap();
    drop(program);
    let cold = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
    let view = runtime.read(&run, &cold).await.unwrap();
    assert_eq!(view.head_digest(), observed.head_digest());
    let RunViewState::AwaitingRecovery { failure } = view.state() else {
        panic!("report failure cannot erase the acknowledged original")
    };
    failure.original().decode::<Outage>().unwrap();
    assert!(view.failure().is_none());
    assert!(view.success().is_none());
    assert!(calls.lock().unwrap().is_empty());
    // With ordinary policy restored, cold recovery can finish from that same original.
    let terminal = runtime.resume(&run, &cold).await.unwrap();
    assert!(terminal.failure().is_some());
}
