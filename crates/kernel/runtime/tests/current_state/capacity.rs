use super::*;
use mfm_journal::{decode_frame, seal_frame};
use mfm_runtime::{RecordingFailure, SizeResource, SizeViolation};
use mfm_store::{AppendResult, LoadedRun, StoreError};
use std::{future::Future, pin::Pin};

struct PauseSecond(MemoryStore);
impl Store for PauseSecond {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        self.0.load_run(run, probe)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.0.append_run(frame).await?;
            Ok(if frame.run_sequence() == 2 {
                AppendResult::NotInserted
            } else {
                result
            })
        })
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct CapacityFailure {
    code: u64,
}
impl ClassifyError for CapacityFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
struct RejectAtCapacity;
impl State for RejectAtCapacity {
    type Input = Input;
    type Output = Input;
    type Failure = CapacityFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.capacity-reject@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for RejectAtCapacity {
    fn evaluate(_: Input) -> Result<ProposedStateOutcome<Input, CapacityFailure>, NativeCause> {
        Ok(ProposedStateOutcome::Failure {
            failure: CapacityFailure { code: 91 },
        })
    }
}
struct RejectFlow;
impl Operation for RejectFlow {
    type Input = Input;
    type Output = Input;
    type Failure = CapacityFailure;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, CapacityFailure>,
    ) -> mfm_program::Result<()> {
        body.pure::<RejectAtCapacity, Identity<CapacityFailure>>(NoParams, Occurrence::new())
    }
}

// Store owns only the physical prefix. These opaque intervening frames deliberately make no
// historical transition claim; Runtime qualifies the admission and the final current record.
async fn fill_frame_count(store: &MemoryStore, run: &RunId) {
    let loaded = store.load_run(run, None).await.unwrap().unwrap();
    let latest = decode_frame(loaded.latest()).unwrap();
    let filler = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap();
    let mut head = loaded.head().head_digest().clone();
    for sequence in loaded.head().head_sequence() + 1..=65_536 {
        let payload = if sequence == 65_536 {
            latest.payload()
        } else {
            &filler
        };
        let frame = seal_frame(run, sequence, Some(&head), payload).unwrap();
        assert_eq!(
            store.append_run(&frame).await.unwrap(),
            AppendResult::Inserted
        );
        head = frame.head_digest().clone();
    }
}

#[tokio::test]
async fn full_history_preserves_committed_original_when_recovery_cannot_fit() {
    let store = Arc::new(PauseSecond(MemoryStore::new()));
    let assembly = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_pure::<RejectAtCapacity>().unwrap();
        builder.finish()
    };
    let input = Input {
        value: 7,
        continuation: "capacity original".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/capacity-original@1").unwrap(),
        &RejectFlow,
        &input,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([160; 32]));
    let original = Runtime::new(assembly(), store.clone())
        .start(run.clone(), program, input)
        .await
        .unwrap();
    assert!(matches!(
        original.state(),
        RunViewState::AwaitingRecovery { .. }
    ));
    fill_frame_count(&store.0, &run).await;
    let runtime = Runtime::new(assembly(), store.clone());
    let before = runtime.read(&run).await.unwrap();
    assert_eq!(before.head_sequence(), 65_536);
    let InvocationFailure::Execution {
        error,
        last_observed: Some(observed),
        ..
    } = runtime.resume(&run).await.err().unwrap()
    else {
        panic!("recovery exceeds physical frame count")
    };
    assert_eq!(observed.head_digest(), before.head_digest());
    assert!(matches!(
        error.size_limit(),
        Some(SizeViolation::Measured {
            resource: SizeResource::FrameCount,
            actual: 65_537,
            limit: 65_536
        })
    ));
    let RuntimeError::Recording { failure, .. } = error else {
        panic!("recording cause")
    };
    assert!(matches!(
        failure.as_ref(),
        RecordingFailure::BeforeAppend {
            candidate: None,
            ..
        }
    ));
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), before.head_digest());
    let RunViewState::AwaitingRecovery { failure } = cold.state() else {
        panic!("original remains durable")
    };
    assert_eq!(
        failure.original().decode::<CapacityFailure>().unwrap().code,
        91
    );
}

struct CapacityEffect;
impl State for CapacityEffect {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.capacity-effect@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_program::EffectState<Submit> for CapacityEffect {
    fn prepare(input: &Input) -> Result<Request, NativeCause> {
        Ok(Request { value: input.value })
    }
    fn interpret(_: Input, _: &Request) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        panic!("uncommitted settlement cannot enter interpretation")
    }
}
impl mfm_program::CapabilityInjection<CapacityEffect> for Submit {
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
struct CapacityEffectFlow;
impl Operation for CapacityEffectFlow {
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
        body.effect::<CapacityEffect, Submit, Identity<Never>>(
            &NoParams,
            NoParams,
            Occurrence::new(),
        )
    }
}

#[tokio::test]
async fn full_history_cannot_acknowledge_external_settlement_or_replace_pending_command() {
    let store = Arc::new(PauseSecond(MemoryStore::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_effect::<CapacityEffect, Submit>().unwrap();
    let adapter_calls = calls.clone();
    builder
        .register_effect_adapter::<Submit, _, _>(NoParams, move |_, _, command| {
            adapter_calls.fetch_add(1, Ordering::SeqCst);
            let value = command.value;
            Box::pin(async move {
                Ok(mfm_runtime::EffectAdapterOutcome::Settled(Request {
                    value,
                }))
            })
        })
        .unwrap();
    let runtime = Runtime::new(builder.finish(), store.clone());
    let input = Input {
        value: 17,
        continuation: "capacity settlement".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/capacity-settlement@1").unwrap(),
        &CapacityEffectFlow,
        &input,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([161; 32]));
    let pending = runtime.start(run.clone(), program, input).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let RunViewState::EffectPending { effect_id, .. } = pending.state() else {
        panic!("prepared command")
    };
    let effect_id = effect_id.clone();
    fill_frame_count(&store.0, &run).await;
    let before = runtime.read(&run).await.unwrap();
    let InvocationFailure::Execution {
        error,
        last_observed: Some(observed),
        ..
    } = runtime.resume(&run).await.err().unwrap()
    else {
        panic!("settlement exceeds physical frame count")
    };
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(observed.head_digest(), before.head_digest());
    assert!(matches!(
        error.size_limit(),
        Some(SizeViolation::Measured {
            resource: SizeResource::FrameCount,
            actual: 65_537,
            limit: 65_536
        })
    ));
    let RuntimeError::Recording { failure, .. } = error else {
        panic!("settlement recording cause")
    };
    assert!(matches!(
        failure.as_ref(),
        RecordingFailure::BeforeAppend {
            original: None,
            candidate: None,
            ..
        }
    ));
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), before.head_digest());
    let RunViewState::EffectPending {
        effect_id: cold_effect_id,
        ..
    } = cold.state()
    else {
        panic!("settlement was not acknowledged")
    };
    assert_eq!(cold_effect_id, &effect_id);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct LargeReportFailure {
    detail: String,
}
impl ClassifyError for LargeReportFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
struct RejectWithLargeReport;
impl State for RejectWithLargeReport {
    type Input = Input;
    type Output = Input;
    type Failure = LargeReportFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.report-capacity-reject@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for RejectWithLargeReport {
    fn evaluate(_: Input) -> Result<ProposedStateOutcome<Input, LargeReportFailure>, NativeCause> {
        Ok(ProposedStateOutcome::Failure {
            failure: LargeReportFailure {
                detail: "a".repeat(16 * 1024 * 1024),
            },
        })
    }
}
struct LargeReportFlow;
impl Operation for LargeReportFlow {
    type Input = Input;
    type Output = Input;
    type Failure = LargeReportFailure;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, LargeReportFailure>,
    ) -> mfm_program::Result<()> {
        body.pure::<RejectWithLargeReport, Identity<LargeReportFailure>>(
            NoParams,
            Occurrence::new(),
        )
    }
}

#[tokio::test]
async fn individually_admitted_causes_cannot_append_an_oversized_terminal_report() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_pure::<RejectWithLargeReport>().unwrap();
    let runtime = Runtime::new(builder.finish(), store.clone());
    let input = Input {
        value: 7,
        continuation: "actual derived report limit".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/report-capacity@1").unwrap(),
        &LargeReportFlow,
        &input,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([162; 32]));
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
        panic!("combined report must fail before terminal insertion")
    };
    assert_eq!(observed.head_sequence(), 2);
    assert!(
        matches!(error.size_limit(), Some(SizeViolation::SerializationBound {
        resource: SizeResource::FailureReport, limit: 33_554_432, observed_at_least,
    }) if observed_at_least > 33_554_432)
    );
    let RuntimeError::Recording { failure, .. } = error else {
        panic!("report recording cause")
    };
    let RecordingFailure::BeforeAppend {
        candidate: None,
        cause,
        ..
    } = failure.as_ref()
    else {
        panic!("report rejected before sealing a terminal candidate")
    };
    let RuntimeError::Native {
        operation: mfm_runtime::Operation::Project,
        stage: mfm_runtime::Stage::Encode,
        cause,
    } = cause.downcast_ref::<RuntimeError>().unwrap()
    else {
        panic!("report encoding retains its actual boundary")
    };
    let bound = cause
        .downcast_ref::<mfm_canonical::CanonicalError>()
        .unwrap()
        .serialization_bound()
        .unwrap();
    assert_eq!(bound.0, 33_554_432);
    assert!(bound.1 > bound.0);
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), observed.head_digest());
    let RunViewState::AwaitingRecovery { failure } = cold.state() else {
        panic!("original remains readable")
    };
    assert_eq!(
        failure
            .original()
            .decode::<LargeReportFailure>()
            .unwrap()
            .detail
            .len(),
        16 * 1024 * 1024
    );
    assert_eq!(
        store
            .load_run(&run, None)
            .await
            .unwrap()
            .unwrap()
            .head()
            .head_sequence(),
        2
    );
}
