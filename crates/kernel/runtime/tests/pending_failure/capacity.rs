use super::*;
use mfm_journal::{decode_frame, seal_frame};
use mfm_runtime::RecordingFailure;
use mfm_store::{AppendResult, LoadedRun, StoreError};
use mfm_values::{SizeResource, SizeViolation};
use std::result::Result;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{future::Future, pin::Pin};

// Supplies the real admission/current payload with a boundary-valued physical summary. This
// exercises Runtime's pre-append count comparison without allocating 65,536 history frames;
// MemoryStore's physical limit is independently tested at its owner.
struct PauseSecond {
    inner: MemoryStore,
    full: AtomicBool,
}
impl Store for PauseSecond {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let loaded = self.inner.load_run(run, probe).await?;
            if !self.full.load(Ordering::SeqCst) {
                return Ok(loaded);
            }
            assert!(
                probe.is_none(),
                "pre-append capacity rejection must not probe"
            );
            let loaded = loaded.unwrap();
            let current = decode_frame(loaded.latest()).unwrap();
            let latest =
                seal_frame(run, 65_536, Some(current.head_digest()), current.payload()).unwrap();
            LoadedRun::new(
                mfm_store::RunSummary::new(
                    run.clone(),
                    65_536,
                    latest.head_digest().clone(),
                    loaded.head().total_bytes() + 65_536 * 512,
                )
                .unwrap(),
                loaded.admission().clone(),
                Arc::from(latest.canonical_bytes()),
                None,
            )
            .map(Some)
        })
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            assert!(
                !self.full.load(Ordering::SeqCst),
                "capacity must reject before Store append"
            );
            let result = self.inner.append_run(frame).await?;
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
    type Input = Number;
    type Output = Number;
    type Failure = CapacityFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.capacity-reject@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for RejectAtCapacity {
    fn evaluate(
        _: Number,
    ) -> Result<ProposedStateOutcome<Number, CapacityFailure>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure {
            failure: CapacityFailure { code: 91 },
        })
    }
}
// Exhausting frame capacity must leave the original failure readable even when its recovery
// decision cannot be recorded.
#[tokio::test]
async fn full_history_preserves_committed_original_when_recovery_cannot_fit() {
    let store = Arc::new(PauseSecond {
        inner: MemoryStore::new(),
        full: AtomicBool::new(false),
    });
    let resources = Resources::<Pure<RejectAtCapacity>>::new(|_, _, _| {
        panic!("pure failure cannot enter an adapter")
    });
    let input = Number { value: 7 };
    let program = compile(
        EntryPointId::new("mfm.test/capacity-original@1").unwrap(),
        &Pure::<RejectAtCapacity>::default(),
        &input,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([160; 32]));
    let original = Runtime::new(store.clone())
        .start(run.clone(), &program, &input)
        .await
        .unwrap();
    assert!(matches!(
        original.state(),
        RunViewState::AwaitingRecovery { .. }
    ));
    store.full.store(true, Ordering::SeqCst);
    let runtime = Runtime::new(store.clone());
    let before = runtime.read(&run, &program).await.unwrap();
    assert_eq!(before.head_sequence(), 65_536);
    let InvocationFailure::Execution {
        error,
        last_observed: Some(observed),
        ..
    } = runtime.resume(&run, &program).await.err().unwrap()
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
        RecordingFailure::BeforeAppend { .. }
    ));
    let document = runtime.program_document(&run).await.unwrap();
    let program = load(document.canonical_bytes(), &resources).unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
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
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.capacity-effect@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_program::EffectState<Submit> for CapacityEffect {
    fn prepare(input: &Number) -> Result<Number, InvocationDiagnostic> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        _: Number,
        _: &Number,
    ) -> Result<ProposedStateOutcome<Number, Never>, InvocationDiagnostic> {
        panic!("uncommitted settlement cannot enter interpretation")
    }
}
impl EffectSelection<Submit> for CapacityEffect {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}
type CapacityEffectFlow = ResolvedEffect<CapacityEffect, Submit, Native<Cause>>;

// An external settlement that cannot fit in history must leave the existing command pending and
// must not be acknowledged as durable.
#[tokio::test]
async fn full_history_cannot_acknowledge_external_settlement_or_replace_pending_command() {
    let store = Arc::new(PauseSecond {
        inner: MemoryStore::new(),
        full: AtomicBool::new(false),
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter_calls = calls.clone();
    let resources = Resources::<CapacityEffectFlow>::new(move |_, _, command| {
        adapter_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(EffectAdapterOutcome::Settled(command)) })
    });
    let runtime = Runtime::new(store.clone());
    let input = Number { value: 17 };
    let program = compile(
        EntryPointId::new("mfm.test/capacity-settlement@1").unwrap(),
        &CapacityEffectFlow::new(Number { value: 1 }),
        &input,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([161; 32]));
    let pending = runtime.start(run.clone(), &program, &input).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let RunViewState::EffectPending { effect, .. } = pending.state() else {
        panic!("prepared command")
    };
    let effect_id = effect.effect_id().clone();
    store.full.store(true, Ordering::SeqCst);
    let before = runtime.read(&run, &program).await.unwrap();
    let InvocationFailure::Execution {
        error,
        last_observed: Some(observed),
        ..
    } = runtime.resume(&run, &program).await.err().unwrap()
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
    assert!(matches!(error, RuntimeError::Native { .. }));
    let document = runtime.program_document(&run).await.unwrap();
    let program = load(document.canonical_bytes(), &resources).unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
    assert_eq!(cold.head_digest(), before.head_digest());
    let RunViewState::EffectPending {
        effect: cold_effect,
        ..
    } = cold.state()
    else {
        panic!("settlement was not acknowledged")
    };
    assert_eq!(cold_effect.effect_id(), &effect_id);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
