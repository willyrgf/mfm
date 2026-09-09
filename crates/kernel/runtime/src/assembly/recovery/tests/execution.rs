use super::*;

struct RetryingRead;
impl mfm_program::Operation for RetryingRead {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, EvmFailure>,
    ) -> mfm_program::Result<()> {
        let mut classifiers = Classifiers::new();
        classifiers
            .bind::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
                NoParams, NoParams, NoParams,
            )?;
        scope.classifiers(classifiers)?;
        let mut handlers = Handlers::new();
        handlers.bind::<EvmIncident, RetryRead>(NoParams)?;
        scope.handlers(handlers)?;
        scope.allowances(RecoveryAllowances::new(1, 0))?;
        scope.read::<EvmRead, Observation, Identity<EvmFailure>>(
            &Offset { value: 1 },
            NoParams,
            mfm_program::Occurrence::new(),
            mfm_program::ConclusionBound::new(65536)?,
        )
    }
}

#[tokio::test]
async fn committed_read_recovery_yields_and_reconstructs_without_provider_calls() {
    use crate::{FailureCauseView, RunViewState, RunnableReason, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use mfm_program::{ProgramLimits, RecoveryLimit, StopReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    for operational in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_read::<EvmRead, Observation>().unwrap();
        builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
        builder
            .register_handler::<EvmIncident, RetryRead>()
            .unwrap();
        builder
            .register_adapter::<Observation, _, _>(Offset { value: 1 }, move |_, intent| {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if operational {
                        Err(AdapterError::Operational(ProviderError::Unavailable))
                    } else {
                        Ok(Offset {
                            value: intent.value,
                        })
                    }
                })
            })
            .unwrap();
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(builder.finish(), store);
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/retry@1").unwrap(),
            &RetryingRead,
            &Offset { value: 7 },
            ProgramLimits::new(1),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array(
            [if operational { 42 } else { 41 }; 32],
        ));
        let oversized = mfm_program::expand_program(
            EntryPointId::new("mfm.test/retry@1").unwrap(),
            &RetryingRead,
            &Offset { value: 7 },
            ProgramLimits::new(u32::MAX),
        )
        .unwrap();
        assert!(matches!(
            runtime
                .start(run.clone(), oversized, Offset { value: 7 })
                .await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::SizeLimit {
                    resource: crate::SizeResource::FrameCount,
                    ..
                },
                last_observed: None,
                ..
            })
        ));
        assert!(matches!(
            runtime.read(&run).await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::Absent,
                ..
            })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            runtime
                .start(run.clone(), program.clone(), Offset { value: 8 })
                .await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::Internal,
                last_observed: None,
                ..
            })
        ));
        assert!(matches!(
            runtime.read(&run).await,
            Err(crate::InvocationFailure::Execution {
                error: RuntimeError::Absent,
                ..
            })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let yielded = runtime
            .start(run.clone(), program, Offset { value: 7 })
            .await
            .unwrap();
        assert_eq!(yielded.head_sequence(), 2);
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
        let loaded = runtime.read(&run).await.unwrap();
        assert_eq!(loaded.head_digest(), yielded.head_digest());
        assert!(matches!(
            loaded.state(),
            RunViewState::Runnable {
                reason: RunnableReason::Retry,
                ..
            }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let terminal = runtime.resume(&run).await.unwrap();
        assert_eq!(terminal.head_sequence(), 3);
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
        match (operational, report.cause()) {
            (false, FailureCauseView::Domain { original, root }) => {
                assert_eq!(original.decode::<EvmFailure>().unwrap().source, 7);
                assert_eq!(root.decode::<EvmFailure>().unwrap().source, 7);
            }
            (true, FailureCauseView::Adapter(incident)) => {
                assert!(matches!(
                    incident.error.decode::<ProviderError>().unwrap(),
                    ProviderError::Unavailable
                ));
                assert_eq!(
                    incident
                        .state_context
                        .decode::<EvmContext>()
                        .unwrap()
                        .source,
                    7
                );
            }
            _ => panic!("cause alternative changed"),
        }
        let cold = runtime.read(&run).await.unwrap();
        let RunViewState::Failed(cold_report) = cold.state() else {
            panic!("cold failure")
        };
        assert_eq!(cold_report.value_ref(), report.value_ref());
        assert_eq!(cold_report.canonical_bytes(), report.canonical_bytes());
        assert_eq!(cold.head_digest(), terminal.head_digest());
        runtime.resume(&run).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

struct EvmSettlement;
impl mfm_program::State for EvmSettlement {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-settlement@1").map_err(|_| ProgramError::InvalidContract)
    }
}
struct Settlement;
impl mfm_capabilities::EffectCapabilityContract for Settlement {
    type Command = Offset;
    type Evidence = Offset;
    type OperationalError = ProviderError;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.settlement@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &mfm_ids::EffectId,
        command: &Offset,
        evidence: &Offset,
    ) -> mfm_capabilities::Result<()> {
        if command.value == evidence.value {
            Ok(())
        } else {
            Err(mfm_capabilities::CapabilityError::EvidenceBinding)
        }
    }
}
impl mfm_program::EffectState<Settlement> for EvmSettlement {
    type AdapterContext = EvmContext;
    fn prepare(input: &Offset) -> std::result::Result<Offset, mfm_program::PreparationError> {
        Ok(Offset { value: input.value })
    }
    fn adapter_context(
        input: &Offset,
        _: &Offset,
        _: &ProviderError,
    ) -> std::result::Result<EvmContext, StateExecutionError> {
        Ok(EvmContext {
            source: input.value,
        })
    }
    fn interpret(
        input: Offset,
        _: &Offset,
    ) -> std::result::Result<
        mfm_program::ProposedStateOutcome<Offset, EvmFailure>,
        StateExecutionError,
    > {
        Ok(mfm_program::ProposedStateOutcome::Success { output: input })
    }
}
impl mfm_program::CapabilityInjection<EvmSettlement> for Settlement {
    type Setup = Offset;
    type ExpandedInput = Offset;
    type ExpandedOutput = Offset;
    type ExpandedFailure = EvmFailure;
    type FailureMap = Identity<EvmFailure>;
    fn failure_map_params(_: &Offset) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &Offset) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct PendingSettlement;
impl mfm_program::Operation for PendingSettlement {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, EvmFailure>,
    ) -> mfm_program::Result<()> {
        scope.effect::<EvmSettlement, Settlement, Identity<EvmFailure>>(
            &Offset { value: 1 },
            NoParams,
            mfm_program::Occurrence::new(),
            mfm_program::EffectBounds::new(65536, 65536)?,
        )
    }
}

#[tokio::test]
async fn stopped_pending_effect_retains_exact_authority_until_explicit_settlement() {
    use crate::{EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use mfm_program::{ProgramLimits, StopReason};
    use std::sync::Mutex;

    let observed_ids = Arc::new(Mutex::new(Vec::new()));
    let observations = Arc::clone(&observed_ids);
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder
        .register_effect::<EvmSettlement, Settlement>()
        .unwrap();
    builder
        .register_effect_adapter::<Settlement, _, _>(
            Offset { value: 1 },
            move |effect, _, command| {
                let attempt = {
                    let mut observations = observations.lock().unwrap();
                    observations.push(effect.clone());
                    observations.len()
                };
                Box::pin(async move {
                    match attempt {
                        1 => Ok(EffectAdapterOutcome::Pending),
                        2 => Err(AdapterError::Operational(ProviderError::Unavailable)),
                        _ => Ok(EffectAdapterOutcome::Settled(Offset {
                            value: command.value,
                        })),
                    }
                })
            },
        )
        .unwrap();
    let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/pending@1").unwrap(),
        &PendingSettlement,
        &Offset { value: 9 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([43; 32]));
    let pending = runtime
        .start(run.clone(), program, Offset { value: 9 })
        .await
        .unwrap();
    assert_eq!(pending.head_sequence(), 2);
    let RunViewState::EffectPending {
        position,
        effect_id,
    } = pending.state()
    else {
        panic!("pending authority")
    };
    assert_eq!(position.visit.value(), 0);
    let stopped = match runtime.resume(&run).await {
        Err(InvocationFailure::RecoveryStopped {
            observed,
            incident,
            reason,
        }) => {
            assert_eq!(reason, StopReason::Nonrecoverable);
            assert_eq!(
                incident
                    .state_context
                    .decode::<EvmContext>()
                    .unwrap()
                    .source,
                9
            );
            assert!(matches!(
                incident.error.decode::<ProviderError>().unwrap(),
                ProviderError::Unavailable
            ));
            observed
        }
        _ => panic!("unresolved invocation stop"),
    };
    assert_eq!(stopped.head_digest(), pending.head_digest());
    let loaded = runtime.read(&run).await.unwrap();
    assert_eq!(loaded.head_digest(), pending.head_digest());
    assert!(
        matches!(loaded.state(), RunViewState::EffectPending { effect_id: retained, position: retained_position } if retained == effect_id && retained_position == position)
    );
    assert_eq!(observed_ids.lock().unwrap().len(), 2);
    let settled = runtime.resume(&run).await.unwrap();
    assert_eq!(settled.head_sequence(), 3);
    let RunViewState::Succeeded(output) = settled.state() else {
        panic!("settled success")
    };
    assert_eq!(output.decode::<Offset>().unwrap().value, 9);
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), settled.head_digest());
    let ids = observed_ids.lock().unwrap();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| id == effect_id));
}

struct InjectedObservation;
impl mfm_capabilities::ReadCapabilityContract for InjectedObservation {
    type Intent = Offset;
    type Evidence = Offset;
    type OperationalError = ProviderError;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.injected-observation@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        reference: &ContentRef,
        intent: &Offset,
        evidence: &Offset,
    ) -> mfm_capabilities::Result<()> {
        Observation::bind_evidence(reference, intent, evidence)
    }
}
impl mfm_program::ReadState<InjectedObservation> for EvmRead {
    type AdapterContext = EvmContext;
    fn prepare(input: &Offset) -> std::result::Result<Offset, mfm_program::PreparationError> {
        <Self as mfm_program::ReadState<Observation>>::prepare(input)
    }
    fn interpret(
        input: Offset,
        evidence: &Offset,
    ) -> std::result::Result<
        mfm_program::ProposedStateOutcome<Offset, EvmFailure>,
        StateExecutionError,
    > {
        <Self as mfm_program::ReadState<Observation>>::interpret(input, evidence)
    }
    fn adapter_context(
        input: &Offset,
        intent: &Offset,
        error: &ProviderError,
    ) -> std::result::Result<EvmContext, StateExecutionError> {
        <Self as mfm_program::ReadState<Observation>>::adapter_context(input, intent, error)
    }
}
impl mfm_program::CapabilityInjection<EvmRead> for InjectedObservation {
    type Setup = Offset;
    type ExpandedInput = Offset;
    type ExpandedOutput = Offset;
    type ExpandedFailure = EvmFailure;
    type FailureMap = Identity<EvmFailure>;
    fn failure_map_params(_: &Offset) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(_: &Offset) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(&Offset { value: 1 })
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn write_before(
        setup: &Offset,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, EvmFailure>,
    ) -> mfm_program::Result<()> {
        if setup.value == 1 {
            scope.effect::<EvmSettlement, Settlement, Identity<EvmFailure>>(
                &Offset { value: 1 },
                NoParams,
                mfm_program::Occurrence::new(),
                mfm_program::EffectBounds::new(65536, 65536)?,
            )?;
        }
        Ok(())
    }
}
struct RestartFirst;
impl Handler<EvmIncident> for RestartFirst {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.restart-first@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: &EvmIncident,
        _: Assessment,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(context
            .eligible_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .unwrap_or(RecoveryRequest::Stop))
    }
}
struct RestartRegion {
    before_checkpoint: bool,
    injected: bool,
}
impl mfm_program::Operation for RestartRegion {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, EvmFailure>,
    ) -> mfm_program::Result<()> {
        if self.before_checkpoint {
            scope.effect::<EvmSettlement, Settlement, Identity<EvmFailure>>(
                &Offset { value: 1 },
                NoParams,
                mfm_program::Occurrence::new(),
                mfm_program::EffectBounds::new(65536, 65536)?,
            )?;
        }
        let checkpoint = scope.checkpoint::<Offset>()?;
        let mut classifiers = Classifiers::new();
        classifiers
            .bind::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
                NoParams, NoParams, NoParams,
            )?;
        scope.classifiers(classifiers)?;
        let mut handlers = Handlers::new();
        handlers.bind::<EvmIncident, RestartFirst>(NoParams)?;
        handlers.checkpoint::<EvmIncident, Offset>(&checkpoint)?;
        scope.handlers(handlers)?;
        scope.allowances(RecoveryAllowances::new(0, 1))?;
        scope.read::<EvmRead, InjectedObservation, Identity<EvmFailure>>(
            &Offset {
                value: u64::from(self.injected),
            },
            NoParams,
            mfm_program::Occurrence::new(),
            mfm_program::ConclusionBound::new(65536)?,
        )
    }
}

#[tokio::test]
async fn injected_effect_blocks_prior_checkpoint_but_preserves_post_effect_restart() {
    use crate::{EffectAdapterOutcome, RunViewState, RunnableReason, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use mfm_program::{ProgramLimits, RecoveryLimit, StopReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    for (case, before_checkpoint, injected) in
        [(0, false, false), (1, false, true), (2, true, false)]
    {
        let effects = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&effects);
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_read::<EvmRead, InjectedObservation>()
            .unwrap();
        builder
            .register_effect::<EvmSettlement, Settlement>()
            .unwrap();
        builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
        builder
            .register_handler::<EvmIncident, RestartFirst>()
            .unwrap();
        builder
            .register_adapter::<InjectedObservation, _, _>(Offset { value: 1 }, |_, intent| {
                Box::pin(async move {
                    Ok(Offset {
                        value: intent.value,
                    })
                })
            })
            .unwrap();
        builder
            .register_effect_adapter::<Settlement, _, _>(
                Offset { value: 1 },
                move |_, _, command| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        Ok(EffectAdapterOutcome::Settled(Offset {
                            value: command.value,
                        }))
                    })
                },
            )
            .unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/restart-region@1").unwrap(),
            &RestartRegion {
                before_checkpoint,
                injected,
            },
            &Offset { value: 7 },
            ProgramLimits::new(1),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([50 + case; 32]));
        let first = runtime
            .start(run.clone(), program, Offset { value: 7 })
            .await
            .unwrap();
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_digest(), first.head_digest());
        if injected {
            let RunViewState::Failed(report) = first.state() else {
                panic!("injected authority removes earlier checkpoint eligibility")
            };
            assert_eq!(report.reason(), &StopReason::Requested);
            assert_eq!(report.usage().run_decisions, 0);
            assert_eq!(effects.load(Ordering::SeqCst), 1);
        } else {
            let RunViewState::Runnable {
                position,
                reason: RunnableReason::Restart { checkpoint },
            } = first.state()
            else {
                panic!("eligible Read checkpoint")
            };
            assert_eq!(checkpoint.index(), usize::from(before_checkpoint));
            assert_eq!(position.state, *checkpoint);
            assert_eq!(
                position.visit.value(),
                if before_checkpoint { 2 } else { 1 }
            );
            let exhausted = runtime.resume(&run).await.unwrap();
            let RunViewState::Failed(report) = exhausted.state() else {
                panic!("restart counters survive restored input")
            };
            assert_eq!(
                report.reason(),
                &StopReason::Exhausted(RecoveryLimit::StateRestart)
            );
            assert_eq!(report.usage().state_restarts, 1);
            assert_eq!(
                effects.load(Ordering::SeqCst),
                usize::from(before_checkpoint)
            );
        }
    }
}

#[tokio::test]
async fn ambiguous_recovery_append_stops_with_historical_observation_and_preserved_source() {
    use crate::{InvocationFailure, RunViewState, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<EvmRead, Observation>().unwrap();
    builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
    builder
        .register_handler::<EvmIncident, RetryRead>()
        .unwrap();
    builder
        .register_adapter::<Observation, _, _>(Offset { value: 1 }, move |_, intent| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(Offset {
                    value: intent.value,
                })
            })
        })
        .unwrap();
    let store = Arc::new(ScriptedStore::new([(
        2,
        AppendAction::RetainThenIndeterminate,
    )]));
    let runtime = Runtime::new(builder.finish(), store);
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/ambiguous-recovery@1").unwrap(),
        &RetryingRead,
        &Offset { value: 7 },
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([61; 32]));
    let failure = runtime
        .start(run.clone(), program, Offset { value: 7 })
        .await
        .err()
        .expect("ambiguous acknowledgement");
    let InvocationFailure::Execution {
        run_id,
        error: RuntimeError::Store(mfm_store::StoreError::Indeterminate),
        last_observed: Some(observed),
    } = failure
    else {
        panic!("source-preserving historical observation")
    };
    assert_eq!(run_id, run);
    assert_eq!(observed.head_sequence(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let recovered = runtime.read(&run).await.unwrap();
    assert_eq!(recovered.head_sequence(), 2);
    assert!(matches!(
        recovered.state(),
        RunViewState::Runnable {
            reason: crate::RunnableReason::Retry,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let terminal = runtime.resume(&run).await.unwrap();
    assert_eq!(terminal.head_sequence(), 3);
    assert!(matches!(terminal.state(), RunViewState::Failed(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn competing_recovery_appends_return_the_winner_without_executing_its_new_visit() {
    use crate::{RunViewState, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&entered);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<EvmRead, Observation>().unwrap();
    builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
    builder
        .register_handler::<EvmIncident, RetryRead>()
        .unwrap();
    builder
        .register_adapter::<Observation, _, _>(Offset { value: 1 }, move |_, intent| {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            let barrier = Arc::clone(&barrier);
            notify.notify_one();
            Box::pin(async move {
                if attempt < 2 {
                    barrier.wait().await;
                }
                Ok(Offset {
                    value: intent.value,
                })
            })
        })
        .unwrap();
    let runtime = Arc::new(Runtime::new(
        builder.finish(),
        Arc::new(mfm_store::MemoryStore::new()),
    ));
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/competing-recovery@1").unwrap(),
        &RetryingRead,
        &Offset { value: 7 },
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([62; 32]));
    let first_runtime = Arc::clone(&runtime);
    let first_run = run.clone();
    let first = tokio::spawn(async move {
        first_runtime
            .start(first_run, program, Offset { value: 7 })
            .await
    });
    entered.notified().await;
    let second = runtime.resume(&run).await.unwrap();
    let first = first.await.unwrap().unwrap();
    assert_eq!(first.head_sequence(), 2);
    assert_eq!(second.head_digest(), first.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for view in [first, second] {
        let RunViewState::Runnable {
            position,
            reason: crate::RunnableReason::Retry,
        } = view.state()
        else {
            panic!("winning recovery decision")
        };
        assert_eq!(position.visit.value(), 1);
    }
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_sequence(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_read_preserves_visit_and_spends_no_recovery_allowance() {
    use crate::{RunViewState, RunnableReason, Runtime};
    use mfm_ids::{DigestBytes, EntryPointId, RunId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&entered);
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<EvmRead, Observation>().unwrap();
    builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
    builder
        .register_handler::<EvmIncident, RetryRead>()
        .unwrap();
    builder
        .register_adapter::<Observation, _, _>(Offset { value: 1 }, move |_, intent| {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            notify.notify_one();
            Box::pin(async move {
                if attempt == 0 {
                    std::future::pending::<()>().await;
                }
                Ok(Offset {
                    value: intent.value,
                })
            })
        })
        .unwrap();
    let runtime = Arc::new(Runtime::new(
        builder.finish(),
        Arc::new(mfm_store::MemoryStore::new()),
    ));
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/cancelled-read@1").unwrap(),
        &RetryingRead,
        &Offset { value: 7 },
        mfm_program::ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([63; 32]));
    let first_runtime = Arc::clone(&runtime);
    let first_run = run.clone();
    let first = tokio::spawn(async move {
        first_runtime
            .start(first_run, program, Offset { value: 7 })
            .await
    });
    entered.notified().await;
    first.abort();
    assert!(matches!(first.await, Err(error) if error.is_cancelled()));
    let loaded = runtime.read(&run).await.unwrap();
    assert_eq!(loaded.head_sequence(), 1);
    let RunViewState::Runnable {
        position,
        reason: RunnableReason::Advance,
    } = loaded.state()
    else {
        panic!("unconcluded visit")
    };
    assert_eq!(position.visit.value(), 0);
    let retry = runtime.resume(&run).await.unwrap();
    assert_eq!(retry.head_sequence(), 2);
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
