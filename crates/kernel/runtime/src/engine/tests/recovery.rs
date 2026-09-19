use super::*;
use mfm_capabilities::{ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_program::{BindRead, InjectRead, Read, ReadSelection, ReadState, ResolveReadBinding};

#[allow(dead_code)]
#[path = "../../../tests/support/scripted_store.rs"]
mod scripted_store;
use scripted_store::{AppendAction, ScriptedStore};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Offset {
    value: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Observed {
    value: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct ReadFailure {
    source: u64,
}
static ACK_CLASSIFICATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
impl ClassifyError for ReadFailure {
    fn classify(&self) -> Classification {
        // Only the acknowledgement regression uses this source; other tests run concurrently.
        if self.source == 909 {
            ACK_CLASSIFICATIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Classification::Retryable
    }
}
#[derive(Debug, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("provider rejected observation with code {code}")]
struct ProviderCause {
    code: u32,
}
#[derive(Debug, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("observation unavailable during {operation}")]
struct ProviderError {
    operation: String,
    #[source]
    source: ProviderCause,
}
impl ClassifyError for ProviderError {
    fn classify(&self) -> Classification {
        Classification::Retryable
    }
}
struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = Offset;
    type Evidence = Offset;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.recovery/observation@1")?)
    }
    fn bind_evidence(
        _: &ContentRef,
        intent: &Offset,
        _: &ContentRef,
        evidence: &Offset,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        if intent.value != evidence.value {
            return Err(InvocationDiagnostic::from_fields(
                "evidence_binding",
                "bind_evidence",
                &(intent.value, evidence.value),
                None,
            ));
        }
        Ok(())
    }
}
struct Observe;
impl State for Observe {
    type Input = Offset;
    type Output = Offset;
    type Failure = ReadFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.recovery/observe@1")?)
    }
}
impl ReadState<Observation> for Observe {
    fn prepare(input: &Offset) -> std::result::Result<Offset, InvocationDiagnostic> {
        Ok(Offset { value: input.value })
    }
    fn interpret(
        _: Offset,
        evidence: &Offset,
    ) -> std::result::Result<ProposedStateOutcome<Offset, ReadFailure>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure {
            failure: ReadFailure {
                source: evidence.value,
            },
        })
    }
}
impl ReadSelection<Observation> for Observe {
    type ExpandedInput = Offset;
    type ExpandedOutput = Offset;
}
struct NativeRead;
impl ReadImplementation<Observation> for NativeRead {
    type Binding = NoParams;
    type NativeIntent = Offset;
    type NativeEvidence = Observed;
    type OperationalError = ProviderError;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.recovery/native-read@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &Offset,
    ) -> std::result::Result<Offset, mfm_capabilities::CallbackFailure> {
        Ok(Offset {
            value: intent.value,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &Offset,
        _: &ContentRef,
        _: &Offset,
        evidence: &Observed,
        _: &Object,
    ) -> std::result::Result<Offset, mfm_capabilities::CallbackFailure> {
        Ok(Offset {
            value: evidence.value,
        })
    }
}
impl InjectRead<Observe, Observation> for NativeRead {
    type Prefix = Identity<Offset>;
    type Suffix = Identity<Offset>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl ResolveReadBinding<Offset, Observation> for NativeRead {
    fn binding(_: &Offset) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}

// A scripted external observation boundary, shared by fresh and configuration-free cold binding.
type ReadJob = Pin<
    Box<dyn Future<Output = std::result::Result<Observed, AdapterError<ProviderError>>> + Send>,
>;
struct ReadResources<const COLD: bool>(Arc<dyn Fn(u64) -> ReadJob + Send + Sync>);
impl<const COLD: bool> ProgramEnvironment for ReadResources<COLD> {
    type Sources = (
        mfm_program::Operation<RetryingRead, RetryPolicy>,
        mfm_program::Operation<nested::NestedRegions>,
    );
}
impl<const COLD: bool> CapabilityFamily<Observation> for ReadResources<COLD> {
    type Implementations = (NativeRead,);
}
impl Resolve<Offset, Observation> for ReadResources<false> {
    fn implementation(_: &Offset) -> mfm_program::Result<StableId> {
        Ok(NativeRead::implementation_id()?)
    }
}
impl<const COLD: bool> BindRead<Observation, NativeRead> for ReadResources<COLD> {
    type Adapter = Self;
    fn bind_read(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self(Arc::clone(&self.0)))
    }
}
impl<const COLD: bool> ReadAdapter<Offset, Observed, ProviderError> for ReadResources<COLD> {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Offset,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Observed, AdapterError<ProviderError>>>
                + Send
                + 'a,
        >,
    > {
        (self.0)(intent.value)
    }
}
#[derive(Default)]
struct RetryingRead;
impl OperationDefinition for RetryingRead {
    type Body = Read<Observe, Observation>;
}
impl Plan<Offset> for RetryingRead {
    type Config = Offset;
    fn plan<'a>(&'a self, input: &'a Offset) -> mfm_program::Result<(&'a Offset, Self::Body)> {
        Ok((input, Read::default()))
    }
}
struct RetryPolicy;
impl OperationDefaults for RetryPolicy {
    type Handler = StandardRecovery;
    type Targets = ();
}
impl ResolveDefaults<Offset> for RetryPolicy {
    fn resolve(_: &Offset) -> mfm_program::Result<PolicyValues<StandardRecovery>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(1),
            restarts: Some(0),
        })
    }
}

// Recording a retry must yield before the next provider call; reading retained domain or
// operational failures must preserve the cause and spent allowance.
#[tokio::test]
async fn committed_read_recovery_yields_and_reconstructs_without_provider_calls() {
    use crate::{RunViewState, RunnableReason, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use mfm_program::{ProgramLimits, RecoveryLimit, StopReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    for operational in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let resources = ReadResources::<false>(Arc::new(move |value| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if operational {
                    Err(AdapterError::Operational(ProviderError {
                        operation: "read_balance".into(),
                        source: ProviderCause { code: 73 },
                    }))
                } else {
                    Ok(Observed { value })
                }
            })
        }));
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/retry@1").unwrap(),
            &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
            &Offset { value: 7 },
            &resources,
            ProgramLimits::new(u32::MAX),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array(
            [if operational { 42 } else { 41 }; 32],
        ));
        assert!(matches!(
            admit(
                store.clone(),
                run.clone(),
                program.clone(),
                Object::from_value(&Offset { value: 8 }).unwrap(),
                Advancement::Manual
            )
            .await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::Native {
                    operation: crate::Operation::Admission,
                    ..
                },
                last_observed: None,
                ..
            })
        ));
        assert!(matches!(
            runtime.read(&run, &program).await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::Absent,
                ..
            })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let yielded = runtime
            .start(run.clone(), &program, &Offset { value: 7 })
            .await
            .unwrap();
        assert_eq!(yielded.head_sequence(), 3);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let RunViewState::Runnable {
            position,
            reason: RunnableReason::Retry,
        } = yielded.state()
        else {
            panic!("committed retry must yield")
        };
        assert_eq!(position.visit.value(), 1);
        assert_eq!(position.state.index(), 0);
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let cold_resources = ReadResources::<true>(Arc::clone(&resources.0));
        drop(resources);
        let program = mfm_program::load(document.canonical_bytes(), &cold_resources).unwrap();
        let runtime = Runtime::new(store.clone());
        let loaded = runtime.read(&run, &program).await.unwrap();
        assert_eq!(loaded.head_digest(), yielded.head_digest());
        assert!(matches!(
            loaded.state(),
            RunViewState::Runnable {
                reason: RunnableReason::Retry,
                ..
            }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let terminal = runtime.resume(&run, &program).await.unwrap();
        assert_eq!(terminal.head_sequence(), 5);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let RunViewState::Failed(report) = terminal.state() else {
            panic!("exhausted retry")
        };
        assert_eq!(
            report.reason(),
            &StopReason::Exhausted(RecoveryLimit::StateRetry)
        );
        assert_eq!(report.usage().state_retries, 1);
        assert_eq!(report.usage().run_decisions, 1);
        match (operational, report.failure()) {
            (
                false,
                crate::Failure::Domain {
                    call:
                        StateCall::Read {
                            call,
                            intent,
                            evidence,
                        },
                    original,
                },
            ) => {
                assert_eq!(original.decode::<ReadFailure>().unwrap().source, 7);
                assert_eq!(call.input().decode::<Offset>().unwrap().value, 7);
                assert_eq!(intent.decode::<Offset>().unwrap().value, 7);
                assert_eq!(evidence.decode::<Observed>().unwrap().value, 7);
                assert!(evidence.decode::<Offset>().is_err());
            }
            (true, incident @ crate::Failure::Read { .. }) => {
                assert!(matches!(
                    incident.original().decode::<ProviderError>().unwrap(),
                    ProviderError { operation, source: ProviderCause { code: 73 } } if operation == "read_balance"
                ));
                assert_eq!(incident.call().input().decode::<Offset>().unwrap().value, 7);
            }
            _ => panic!("cause alternative changed"),
        }
        let cold = runtime.read(&run, &program).await.unwrap();
        let RunViewState::Failed(cold_report) = cold.state() else {
            panic!("cold failure")
        };
        assert_eq!(cold_report.value_ref(), report.value_ref());
        assert_eq!(cold_report.canonical_bytes(), report.canonical_bytes());
        assert_eq!(cold.head_digest(), terminal.head_digest());
        runtime.resume(&run, &program).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

// A lost recovery-append acknowledgement must return the prior observation and Store cause;
// later inspection may discover the committed retry.
#[tokio::test]
async fn ambiguous_recovery_append_stops_with_historical_observation_and_preserved_source() {
    use crate::{InvocationFailure, RunViewState, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let resources = ReadResources::<false>(Arc::new(move |value| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(Observed { value }) })
    }));
    let store = Arc::new(ScriptedStore::new([(
        3,
        AppendAction::RetainThenIndeterminate,
    )]));
    let runtime = Runtime::new(store.clone());
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/ambiguous-recovery@1").unwrap(),
        &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
        &Offset { value: 7 },
        &resources,
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([61; 32]));
    let failure = admit(
        store.clone(),
        run.clone(),
        program.clone(),
        Object::from_value(&Offset { value: 7 }).unwrap(),
        Advancement::Manual,
    )
    .await
    .err()
    .expect("ambiguous acknowledgement");
    let InvocationFailure::Execution {
        run_id,
        error: RuntimeError::Recording { failure, .. },
        last_observed: Some(observed),
    } = failure
    else {
        panic!("source-preserving historical observation")
    };
    let crate::RecordingFailure::Store {
        original: Some(original),
        cause: mfm_store::StoreError::Indeterminate(cause),
        ..
    } = failure.as_ref()
    else {
        panic!("retain the original and ambiguous recording cause")
    };
    assert_eq!(
        original.original().decode::<ReadFailure>().unwrap().source,
        7
    );
    assert_eq!(
        cause.as_value(),
        &serde_json::json!({"operation":"test.store", "injected":"Indeterminate"})
    );
    assert_eq!(run_id, run);
    assert_eq!(observed.head_sequence(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let recovered = runtime.read(&run, &program).await.unwrap();
    assert_eq!(recovered.head_sequence(), 3);
    assert!(matches!(
        recovered.state(),
        RunViewState::Runnable {
            reason: crate::RunnableReason::Retry,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let terminal = runtime.resume(&run, &program).await.unwrap();
    assert_eq!(terminal.head_sequence(), 5);
    assert!(matches!(terminal.state(), RunViewState::Failed(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// Concurrent Read failures must converge on retained failure or recovery records and yield
// before either caller executes the granted retry.
#[tokio::test]
async fn competing_original_appends_yield_a_checked_observation_without_executing_the_retry() {
    use crate::{RunViewState, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&entered);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let resources = ReadResources::<false>(Arc::new(move |value| {
        let attempt = counter.fetch_add(1, Ordering::SeqCst);
        let barrier = Arc::clone(&barrier);
        notify.notify_one();
        Box::pin(async move {
            if attempt < 2 {
                barrier.wait().await;
            }
            Ok(Observed { value })
        })
    }));
    let store = Arc::new(mfm_store::MemoryStore::new());
    let runtime = Arc::new(Runtime::new(store.clone()));
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/competing-recovery@1").unwrap(),
        &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
        &Offset { value: 7 },
        &resources,
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([62; 32]));
    let first_store = store.clone();
    let first_program = program.clone();
    let first_run = run.clone();
    let first = tokio::spawn(async move {
        admit(
            first_store,
            first_run,
            first_program,
            Object::from_value(&Offset { value: 7 }).unwrap(),
            Advancement::Manual,
        )
        .await
    });
    entered.notified().await;
    let second = runtime.resume(&run, &program).await.unwrap();
    let first = first.await.unwrap().unwrap();
    assert!(first.head_sequence() == 3 || second.head_sequence() == 3);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for view in [first, second] {
        match view.state() {
            RunViewState::Runnable {
                position,
                reason: crate::RunnableReason::Retry,
            } => {
                assert_eq!(view.head_sequence(), 3);
                assert_eq!(position.visit.value(), 1);
            }
            RunViewState::AwaitingRecovery { failure } => {
                assert_eq!(view.head_sequence(), 2);
                assert_eq!(
                    failure.original().decode::<ReadFailure>().unwrap().source,
                    7
                );
            }
            _ => panic!("original or recovered winner; no retry execution"),
        }
    }
    let cold = runtime.read(&run, &program).await.unwrap();
    assert_eq!(cold.head_sequence(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// An interrupted observation has no recorded failure, so resuming it must retain the visit and
// its unused retry allowance.
#[tokio::test]
async fn cancelled_read_preserves_visit_and_spends_no_recovery_allowance() {
    use crate::{RunViewState, RunnableReason, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&entered);
    let resources = ReadResources::<false>(Arc::new(move |value| {
        let attempt = counter.fetch_add(1, Ordering::SeqCst);
        notify.notify_one();
        Box::pin(async move {
            if attempt == 0 {
                std::future::pending::<()>().await;
            }
            Ok(Observed { value })
        })
    }));
    let store = Arc::new(mfm_store::MemoryStore::new());
    let runtime = Arc::new(Runtime::new(store.clone()));
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/cancelled-read@1").unwrap(),
        &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
        &Offset { value: 7 },
        &resources,
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([63; 32]));
    let first_store = store.clone();
    let first_program = program.clone();
    let first_run = run.clone();
    let first = tokio::spawn(async move {
        admit(
            first_store,
            first_run,
            first_program,
            Object::from_value(&Offset { value: 7 }).unwrap(),
            Advancement::Manual,
        )
        .await
    });
    entered.notified().await;
    first.abort();
    assert!(matches!(first.await, Err(error) if error.is_cancelled()));
    let loaded = runtime.read(&run, &program).await.unwrap();
    assert_eq!(loaded.head_sequence(), 1);
    let RunViewState::Runnable {
        position,
        reason: RunnableReason::Advance,
    } = loaded.state()
    else {
        panic!("unconcluded visit")
    };
    assert_eq!(position.visit.value(), 0);
    let retry = runtime.resume(&run, &program).await.unwrap();
    assert_eq!(retry.head_sequence(), 3);
    let RunViewState::Runnable {
        position,
        reason: RunnableReason::Retry,
    } = retry.state()
    else {
        panic!("first committed decision")
    };
    assert_eq!(position.visit.value(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

mod nested;

// Cold rebinding changes the scripted provider response while the admitted Program and command
// remain exact. A Stop ends automatic recovery, not the caller's retained reconciliation authority.
#[tokio::test]
async fn stopped_pending_effect_retains_exact_authority_until_explicit_settlement() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(MemoryStore::new());
    let resources = Resources {
        calls: Arc::clone(&calls),
        operational: false,
        settled: false,
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/pending-authority@1").unwrap(),
        &mfm_program::Operation::new(Flow::<false>),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([43; 32]));
    let pending = Runtime::new(store.clone())
        .start(run.clone(), &program, &NoParams)
        .await
        .unwrap();
    let RunViewState::EffectPending {
        effect,
        latest_failure: None,
    } = pending.state()
    else {
        panic!("expected prepared authority")
    };
    assert_eq!(pending.head_sequence(), 2);
    let runtime = Runtime::new(store.clone());
    let document = runtime.program_document(&run).await.unwrap();
    let failing = Resources {
        calls: Arc::clone(&calls),
        operational: true,
        settled: false,
    };
    let program = mfm_program::load(document.canonical_bytes(), &failing).unwrap();
    let Err(InvocationFailure::RecoveryStopped { observed }) = runtime.resume(&run, &program).await
    else {
        panic!("expected invocation stop with retained authority")
    };
    let RunViewState::EffectPending {
        effect: retained,
        latest_failure: Some((original, crate::RecoveryOutcome::Stop { reason })),
    } = observed.state()
    else {
        panic!("expected acknowledged pending failure")
    };
    assert_eq!(reason, &mfm_program::StopReason::Requested);
    assert_eq!(retained, effect);
    original.decode::<Outage>().unwrap();
    assert_eq!(observed.head_sequence(), 4);
    let settlement = Resources {
        calls: Arc::clone(&calls),
        operational: false,
        settled: true,
    };
    let program = mfm_program::load(document.canonical_bytes(), &settlement).unwrap();
    let cold = Runtime::new(store.clone())
        .read(&run, &program)
        .await
        .unwrap();
    assert_eq!(cold.head_digest(), observed.head_digest());
    assert!(
        matches!(cold.state(), RunViewState::EffectPending { effect: retained, .. } if retained == effect)
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    let completed = runtime.resume(&run, &program).await.unwrap();
    assert_eq!(completed.head_sequence(), 6);
    let RunViewState::Succeeded(output) = completed.state() else {
        panic!("expected reconciled success")
    };
    output.decode::<NoParams>().unwrap();
    assert_eq!(
        runtime.read(&run, &program).await.unwrap().head_digest(),
        completed.head_digest()
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        &[
            effect.effect_id().clone(),
            effect.effect_id().clone(),
            effect.effect_id().clone()
        ]
    );
}

#[tokio::test]
async fn ambiguous_settlement_is_cold_interpreted_without_resubmitting_the_effect() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resources = Resources {
        calls: Arc::clone(&calls),
        operational: false,
        settled: true,
    };
    let store = Arc::new(ScriptedStore::new([(
        3,
        AppendAction::RetainThenIndeterminate,
    )]));
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/ambiguous-settlement@1").unwrap(),
        &mfm_program::Operation::new(Flow::<false>),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([64; 32]));
    let Err(InvocationFailure::Execution {
        error: RuntimeError::Recording { failure, .. },
        last_observed: Some(previous),
        ..
    }) = admit(
        store.clone(),
        run.clone(),
        program,
        Object::from_value(&NoParams).unwrap(),
        Advancement::Manual,
    )
    .await
    else {
        panic!("expected ambiguous settlement acknowledgement")
    };
    assert!(matches!(
        failure.as_ref(),
        crate::RecordingFailure::Store {
            cause: mfm_store::StoreError::Indeterminate(_),
            ..
        }
    ));
    assert_eq!(previous.head_sequence(), 2);
    assert!(matches!(
        previous.state(),
        RunViewState::EffectPending { .. }
    ));
    let runtime = Runtime::new(store);
    let document = runtime.program_document(&run).await.unwrap();
    let program = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
    let RunViewState::AwaitingInterpretation { settlement } = cold.state() else {
        panic!("expected retained native settlement")
    };
    assert_eq!(cold.head_sequence(), 3);
    assert!(
        settlement
            .evidence()
            .decode::<NativeReceipt>()
            .unwrap()
            .accepted
    );
    let completed = runtime.resume(&run, &program).await.unwrap();
    assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
    assert_eq!(completed.head_sequence(), 4);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn borrowed_execute_returns_terminal_success_or_original_failure_across_retries() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let resources = ReadResources::<false>(Arc::new(move |value| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(Observed { value }) })
    }));
    // Offset intentionally does not implement Clone; execute must retain only its encoding.
    let input = Offset { value: 7 };
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store.clone());
    let pure = mfm_program::compile(
        EntryPointId::new("mfm.test/borrowed-success@1").unwrap(),
        &mfm_program::Pure::<nested::IncrementOffset>::default(),
        &input,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([65; 32]));
    let success = runtime.execute(run.clone(), &pure, &input).await.unwrap();
    assert_eq!(input.value, 7);
    assert_eq!(success.run_id(), &run);
    assert_eq!(
        success.success().unwrap().decode::<Offset>().unwrap().value,
        8
    );
    assert!(success.failure().is_none());
    assert!(success.success().unwrap().decode::<Observed>().is_err());
    let cold = runtime.read(&run, &pure).await.unwrap();
    assert_eq!(cold.success(), success.success());
    let failed = mfm_program::compile(
        EntryPointId::new("mfm.test/borrowed-recovery@1").unwrap(),
        &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([66; 32]));
    let result = runtime.execute(run.clone(), &failed, &input).await.unwrap();
    assert_eq!(input.value, 7);
    assert_eq!(result.run_id(), &run);
    assert!(result.success().is_none());
    let report = result.failure().unwrap();
    assert_eq!(report.usage().state_retries, 1);
    assert_eq!(
        report
            .failure()
            .original()
            .decode::<ReadFailure>()
            .unwrap()
            .source,
        7
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let document = runtime.program_document(&run).await.unwrap();
    let cold_program = mfm_program::load(
        document.canonical_bytes(),
        &ReadResources::<true>(resources.0),
    )
    .unwrap();
    let cold = runtime.resume(&run, &cold_program).await.unwrap();
    assert_eq!(
        cold.failure().unwrap().canonical_bytes(),
        report.canonical_bytes()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn terminal_execution_stops_on_a_reconciled_pending_stop_without_an_extra_attempt() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resources = Resources {
        calls: Arc::clone(&calls),
        operational: true,
        settled: false,
    };
    let store = Arc::new(ScriptedStore::new([(
        4,
        AppendAction::RetainThenNotInserted,
    )]));
    let runtime = Runtime::new(store.clone());
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/reconciled-stop@1").unwrap(),
        &mfm_program::Operation::new(Flow::<false>),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([67; 32]));
    let Err(InvocationFailure::RecoveryStopped { observed }) =
        runtime.execute(run, &program, &NoParams).await
    else {
        panic!("reconciled Stop must end this invocation")
    };
    assert_eq!(observed.head_sequence(), 4);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(matches!(
        observed.state(),
        RunViewState::EffectPending {
            latest_failure: Some((_, crate::RecoveryOutcome::Stop { .. })),
            ..
        }
    ));
}

#[tokio::test]
async fn original_acknowledgement_loss_defers_classification_until_cold_recovery() {
    use std::sync::atomic::Ordering;
    for retained in [false, true] {
        ACK_CLASSIFICATIONS.store(0, Ordering::SeqCst);
        let resources = ReadResources::<false>(Arc::new(|value| {
            Box::pin(async move { Ok(Observed { value }) })
        }));
        let store = Arc::new(ScriptedStore::new([(
            2,
            if retained {
                AppendAction::RetainThenIndeterminate
            } else {
                AppendAction::Indeterminate
            },
        )]));
        let runtime = Runtime::new(store.clone());
        let input = Offset { value: 909 };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/original-acknowledgement@1").unwrap(),
            &mfm_program::Operation::<RetryingRead, RetryPolicy>::default(),
            &input,
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([160 + u8::from(retained); 32]));
        let Err(InvocationFailure::Execution {
            error: RuntimeError::Recording { failure, .. },
            last_observed: Some(observed),
            ..
        }) = runtime.execute(run.clone(), &program, &input).await
        else {
            panic!("lost original acknowledgement must stop automatic recovery")
        };
        let crate::RecordingFailure::Store {
            original: Some(original),
            cause: mfm_store::StoreError::Indeterminate(_),
            ..
        } = failure.as_ref()
        else {
            panic!("retain original and ambiguous Store cause")
        };
        assert_eq!(
            original.original().decode::<ReadFailure>().unwrap().source,
            909
        );
        assert_eq!(observed.head_sequence(), 1);
        assert_eq!(ACK_CLASSIFICATIONS.load(Ordering::SeqCst), 0);
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let cold = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
        let view = runtime.read(&run, &cold).await.unwrap();
        assert_eq!(ACK_CLASSIFICATIONS.load(Ordering::SeqCst), 0);
        if retained {
            assert_eq!(view.head_sequence(), 2);
            assert!(matches!(
                view.state(),
                RunViewState::AwaitingRecovery { .. }
            ));
            let terminal = runtime.resume(&run, &cold).await.unwrap();
            assert!(terminal.failure().is_some());
            assert!(ACK_CLASSIFICATIONS.load(Ordering::SeqCst) > 0);
        } else {
            assert_eq!(view.head_digest(), observed.head_digest());
            assert!(matches!(view.state(), RunViewState::Runnable { .. }));
        }
    }
}
