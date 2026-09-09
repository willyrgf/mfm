use mfm_capabilities::AdapterError;
use mfm_evm::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_portfolio::{MapEvmBalanceFailure, PortfolioSnapshotFailure};
use mfm_program::*;
use mfm_runtime::{
    FailureCauseView, RunViewState, RunnableReason, Runtime, RuntimeAssemblyBuilder,
};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

type Context = EvmBalanceContext<NoContext>;
type EvmIncident = Incident<EvmBalanceFailure, EvmOperationalError, EvmBalanceAdapterContext>;
type PortfolioIncident = Incident<PortfolioSnapshotFailure, EvmOperationalError, NoContext>;

struct IgnoreContext;
impl ValueMap for IgnoreContext {
    type Input = EvmBalanceAdapterContext;
    type Output = NoContext;
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.portfolio.ignore-context@1").unwrap())
    }
    fn apply(_: &NoParams, _: Self::Input) -> std::result::Result<NoContext, StateExecutionError> {
        Ok(NoContext)
    }
}
struct RetryRead;
impl Handler<EvmIncident> for RetryRead {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.portfolio.retry-read@1").unwrap())
    }
    fn handle(
        _: &NoParams,
        incident: &EvmIncident,
        _: Assessment,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(match incident {
            Incident::Adapter {
                original: EvmOperationalError::Timeout,
                ..
            } => RecoveryRequest::RetryState,
            _ => RecoveryRequest::Stop,
        })
    }
}
struct Child {
    target: EvmPhysicalTarget,
    classify: bool,
    retry: bool,
    bound: ConclusionBound,
}
impl Operation for Child {
    type Input = Context;
    type Output = Context;
    type Failure = EvmBalanceFailure;
    fn validate_input(&self, _: &Context) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Context, Context, EvmBalanceFailure>,
    ) -> mfm_program::Result<()> {
        if self.classify {
            let mut family = Classifiers::new();
            family.bind::<EvmOperationalError, Identity<EvmBalanceFailure>, Identity<EvmBalanceAdapterContext>, EvmBalanceClassifier>(NoParams, NoParams, NoParams)?;
            body.classifiers(family)?;
        }
        let first = if self.retry {
            let mut handlers = Handlers::new();
            handlers.bind::<EvmIncident, RetryRead>(NoParams)?;
            Occurrence::new().handlers(handlers)
        } else {
            Occurrence::new()
        };
        body.read::<CheckChainIdentity<NoContext>, EvmChainIdentityRead, Identity<EvmBalanceFailure>>(&self.target.binding_ref().map_err(|_| ProgramError::InvalidContract)?, NoParams, first, self.bound)?;
        body.read::<ReadInitialAnchor<NoContext>, EvmAnchorRead, Identity<EvmBalanceFailure>>(
            &self
                .target
                .binding_ref()
                .map_err(|_| ProgramError::InvalidContract)?,
            NoParams,
            Occurrence::new(),
            self.bound,
        )
    }
}
struct Parent {
    child: Child,
    direct: bool,
}
impl Operation for Parent {
    type Input = Context;
    type Output = Context;
    type Failure = PortfolioSnapshotFailure;
    fn validate_input(&self, _: &Context) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Context, Context, PortfolioSnapshotFailure>,
    ) -> mfm_program::Result<()> {
        let mut classifiers = Classifiers::new();
        classifiers.bind::<EvmOperationalError, MapEvmBalanceFailure, IgnoreContext, NoRecovery>(
            NoParams, NoParams, NoParams,
        )?;
        body.classifiers(classifiers)?;
        let mut handlers = Handlers::new();
        handlers.bind::<PortfolioIncident, Stop>(NoParams)?;
        handlers.bind::<EvmIncident, Stop>(NoParams)?;
        body.handlers(handlers)?;
        body.allowances(RecoveryAllowances::new(1, 0))?;
        if self.direct {
            body.read::<CheckChainIdentity<NoContext>, EvmChainIdentityRead, MapEvmBalanceFailure>(
                &self
                    .child
                    .target
                    .binding_ref()
                    .map_err(|_| ProgramError::InvalidContract)?,
                NoParams,
                Occurrence::new(),
                self.child.bound,
            )
        } else {
            body.operation::<Child, MapEvmBalanceFailure>(&self.child, NoParams)
        }
    }
}

#[tokio::test]
async fn actual_domain_policies_select_parent_child_and_occurrence_bindings() {
    for scenario in 0..4 {
        let target = EvmPhysicalTarget {
            chain_id: NonZeroU64::new(1).unwrap(),
            endpoint_ref: EvmEndpoint::new("policy-fixture")
                .unwrap()
                .endpoint_ref()
                .unwrap(),
        };
        let request = EvmBalanceRequest::new(
            vec![EvmBalanceSource::new(
                "wallet",
                target.chain_id,
                EvmAddress::from_bytes([1; 20]),
                None,
            )
            .unwrap()],
            18,
        )
        .unwrap();
        let bound = request.conclusion_bound(2, 2048).unwrap();
        let input = Context::new(
            request,
            NoContext,
            0,
            "collection".into(),
            target.binding_ref().unwrap(),
        )
        .unwrap();
        let parent = Parent {
            direct: scenario == 0,
            child: Child {
                target: target.clone(),
                classify: scenario >= 2,
                retry: scenario == 3,
                bound,
            },
        };
        let program = expand_program(
            EntryPointId::new("mfm.test.portfolio/policy-selection@1").unwrap(),
            &parent,
            &input,
            ProgramLimits::new(1),
        )
        .unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_read::<CheckChainIdentity<NoContext>, EvmChainIdentityRead>()
            .unwrap();
        builder
            .register_read::<ReadInitialAnchor<NoContext>, EvmAnchorRead>()
            .unwrap();
        builder.register_classifier::<EvmOperationalError, MapEvmBalanceFailure, IgnoreContext, NoRecovery>().unwrap();
        builder.register_classifier::<EvmOperationalError, Identity<EvmBalanceFailure>, Identity<EvmBalanceAdapterContext>, EvmBalanceClassifier>().unwrap();
        builder
            .register_handler::<PortfolioIncident, Stop>()
            .unwrap();
        builder.register_handler::<EvmIncident, Stop>().unwrap();
        builder
            .register_handler::<EvmIncident, RetryRead>()
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        builder
            .register_adapter::<EvmChainIdentityRead, _, _>(
                target.clone(),
                move |reference, intent| {
                    let call = counted.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        if scenario == 0 {
                            Ok(EvmReadEvidence::rejected(reference.clone()))
                        } else if scenario == 3 && call == 1 {
                            Ok(EvmReadEvidence::returned(
                                reference.clone(),
                                EvmReadValue::ChainId(intent.chain_id()),
                            ))
                        } else {
                            Err(AdapterError::Operational(EvmOperationalError::Timeout))
                        }
                    })
                },
            )
            .unwrap();
        builder
            .register_adapter::<EvmAnchorRead, _, _>(target, |_, _| {
                Box::pin(async { Err(AdapterError::Operational(EvmOperationalError::Timeout)) })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([scenario + 1; 32]));
        let first = runtime.start(run.clone(), program, input).await.unwrap();
        let terminal = if scenario == 3 {
            assert!(matches!(
                first.state(),
                RunViewState::Runnable {
                    reason: RunnableReason::Retry,
                    ..
                }
            ));
            assert_eq!(first.head_sequence(), 2);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            runtime.resume(&run).await.unwrap()
        } else {
            first
        };
        let RunViewState::Failed(report) = terminal.state() else {
            panic!("selected stop policy")
        };
        assert_eq!(
            report.reason(),
            &if scenario < 2 {
                StopReason::Nonrecoverable
            } else {
                StopReason::Requested
            }
        );
        if scenario == 0 {
            let FailureCauseView::Domain { original, root } = report.cause() else {
                panic!("mapped domain failure")
            };
            assert!(matches!(
                original.decode::<EvmBalanceFailure>().unwrap(),
                EvmBalanceFailure::SourceUnavailable {
                    collection_ordinal: 0,
                    ..
                }
            ));
            assert!(matches!(
                root.decode::<PortfolioSnapshotFailure>().unwrap(),
                PortfolioSnapshotFailure::CollectionFailed { ordinal: 0, .. }
            ));
        } else {
            let FailureCauseView::Adapter(incident) = report.cause() else {
                panic!("original adapter context")
            };
            assert_eq!(
                incident
                    .state_context
                    .decode::<EvmBalanceAdapterContext>()
                    .unwrap()
                    .collection_ordinal(),
                0
            );
        }
        let cold = runtime.read(&run).await.unwrap();
        let RunViewState::Failed(cold) = cold.state() else {
            panic!("cold stop")
        };
        assert_eq!(cold.canonical_bytes(), report.canonical_bytes());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if scenario == 3 { 2 } else { 1 }
        );
    }
}

struct RestartCollection;
impl Handler<EvmIncident> for RestartCollection {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.portfolio.restart-collection@1").unwrap())
    }
    fn handle(
        _: &NoParams,
        incident: &EvmIncident,
        _: Assessment,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(match incident {
            Incident::Domain(EvmBalanceFailure::AnchorChanged { .. }) => context
                .eligible_restart_targets()
                .first()
                .copied()
                .map(RecoveryRequest::Restart)
                .unwrap_or(RecoveryRequest::Stop),
            _ => RecoveryRequest::Stop,
        })
    }
}
struct RecoveringCollection(CollectEvmBalances<NoContext>);
impl Operation for RecoveringCollection {
    type Input = Context;
    type Output = EvmBalanceCollectionCompletion<NoContext>;
    type Failure = PortfolioSnapshotFailure;
    fn validate_input(&self, input: &Context) -> mfm_program::Result<()> {
        self.0.validate_input(input)
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        let checkpoint = body.checkpoint::<Context>()?;
        let mut classifiers = Classifiers::new();
        classifiers.bind::<EvmOperationalError, Identity<EvmBalanceFailure>, Identity<EvmBalanceAdapterContext>, EvmBalanceClassifier>(NoParams, NoParams, NoParams)?;
        classifiers.bind::<Never, MapEvmBalanceFailure, Identity<NoContext>, NoRecovery>(
            NoParams, NoParams, NoParams,
        )?;
        body.classifiers(classifiers)?;
        let mut handlers = Handlers::new();
        handlers.bind::<EvmIncident, RestartCollection>(NoParams)?;
        handlers.checkpoint::<EvmIncident, Context>(&checkpoint)?;
        handlers.bind::<Incident<PortfolioSnapshotFailure, Never, NoContext>, Stop>(NoParams)?;
        body.handlers(handlers)?;
        body.allowances(RecoveryAllowances::new(0, 1))?;
        body.operation::<CollectEvmBalances<NoContext>, MapEvmBalanceFailure>(&self.0, NoParams)
    }
}

#[tokio::test]
async fn changed_anchor_restarts_the_real_collection_and_preserves_its_acknowledged_prefix() {
    use mfm_store::Store;
    // Cover a changed confirmation and a changed initial anchor after a completed source.
    for changed_call in [1, 2] {
        let target = EvmPhysicalTarget {
            chain_id: NonZeroU64::new(1).unwrap(),
            endpoint_ref: EvmEndpoint::new("restart-fixture")
                .unwrap()
                .endpoint_ref()
                .unwrap(),
        };
        let request = EvmBalanceRequest::new(
            (0..2)
                .map(|index| {
                    EvmBalanceSource::new(
                        format!("wallet-{index}"),
                        target.chain_id,
                        EvmAddress::from_bytes([index; 20]),
                        None,
                    )
                    .unwrap()
                })
                .collect(),
            18,
        )
        .unwrap();
        let input = Context::new(
            request.clone(),
            NoContext,
            0,
            "collection".into(),
            target.binding_ref().unwrap(),
        )
        .unwrap();
        let operation = RecoveringCollection(
            CollectEvmBalances::new(
                target.binding_ref().unwrap(),
                request.clone(),
                request.conclusion_bound(2, 2048).unwrap(),
            )
            .unwrap(),
        );
        let program = expand_program(
            EntryPointId::new("mfm.test.portfolio/restart@1").unwrap(),
            &operation,
            &input,
            ProgramLimits::new(1),
        )
        .unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_read::<CheckChainIdentity<NoContext>, EvmChainIdentityRead>()
            .unwrap();
        builder
            .register_read::<ReadInitialAnchor<NoContext>, EvmAnchorRead>()
            .unwrap();
        builder
            .register_read::<ConfirmBalanceAnchor<NoContext>, EvmAnchorRead>()
            .unwrap();
        builder
            .register_read::<ReadNativeBalance<NoContext>, EvmBalanceRead>()
            .unwrap();
        builder
            .register_pure::<ConsolidateBalanceCollection<NoContext>>()
            .unwrap();
        builder.register_classifier::<EvmOperationalError, Identity<EvmBalanceFailure>, Identity<EvmBalanceAdapterContext>, EvmBalanceClassifier>().unwrap();
        builder
            .register_classifier::<Never, MapEvmBalanceFailure, Identity<NoContext>, NoRecovery>()
            .unwrap();
        builder
            .register_handler::<EvmIncident, RestartCollection>()
            .unwrap();
        builder
            .register_handler::<Incident<PortfolioSnapshotFailure, Never, NoContext>, Stop>()
            .unwrap();
        let anchor_calls = Arc::new(AtomicUsize::new(0));
        let counted = anchor_calls.clone();
        builder
            .register_adapter::<EvmChainIdentityRead, _, _>(target.clone(), |reference, intent| {
                Box::pin(async move {
                    Ok(EvmReadEvidence::returned(
                        reference.clone(),
                        EvmReadValue::ChainId(intent.chain_id()),
                    ))
                })
            })
            .unwrap();
        builder
            .register_adapter::<EvmAnchorRead, _, _>(target.clone(), move |reference, _| {
                let call = counted.fetch_add(1, Ordering::SeqCst);
                let height = if call < changed_call { 7 } else { 8 };
                Box::pin(async move {
                    Ok(EvmReadEvidence::returned(
                        reference.clone(),
                        EvmReadValue::Anchor(EvmBlockAnchor {
                            number: EvmU256::from_u64(height),
                            hash: EvmHash::from_bytes([height as u8; 32]),
                        }),
                    ))
                })
            })
            .unwrap();
        let balance_calls = Arc::new(AtomicUsize::new(0));
        let counted = balance_calls.clone();
        builder
            .register_adapter::<EvmBalanceRead, _, _>(target, move |reference, _| {
                let call = counted.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(EvmReadEvidence::returned(
                        reference.clone(),
                        EvmReadValue::RawUnits(EvmU256::from_u64(if call == 0 {
                            100
                        } else {
                            200
                        })),
                    ))
                })
            })
            .unwrap();
        let assembly = builder.finish();
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(assembly, store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([changed_call as u8 + 10; 32]));
        let yielded = runtime.start(run.clone(), program, input).await.unwrap();
        assert!(matches!(
            yielded.state(),
            RunViewState::Runnable {
                reason: RunnableReason::Restart { .. },
                ..
            }
        ));
        let prefix = mfm_journal::JournalHistory::qualify(
            &run,
            store.load_run(&run).await.unwrap().unwrap(),
        )
        .unwrap();
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_digest(), yielded.head_digest());
        assert_eq!(anchor_calls.load(Ordering::SeqCst), changed_call + 1);
        let terminal = runtime.resume(&run).await.unwrap();
        let RunViewState::Succeeded(value) = terminal.state() else {
            panic!("coherent restarted collection")
        };
        let (_, _, _, number, hash, balances, total) = value
            .decode::<EvmBalanceCollectionCompletion<NoContext>>()
            .unwrap()
            .into_parts();
        assert_eq!(number, "8");
        assert_eq!(hash, EvmHash::from_bytes([8; 32]).to_string());
        assert_eq!(balances.len(), 2);
        assert_eq!(total, "400");
        let history = mfm_journal::JournalHistory::qualify(
            &run,
            store.load_run(&run).await.unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(terminal.head_sequence(), yielded.head_sequence() + 9);
        use mfm_journal::{DomainConclusion, DomainDecision, JournalRecord, ReadConclusion};
        let prefix_len = prefix.records().len();
        assert!(history.records().len() >= prefix_len);
        assert!(prefix.records().eq(history.records().take(prefix_len)));
        assert!(prefix.records().any(|record| matches!(record,
            JournalRecord::ReadConcluded { outcome: ReadConclusion::Observed {
                outcome: DomainConclusion::Failure { original, decision: DomainDecision::Restart { .. } }, ..
            }, .. } if matches!(serde_json::from_slice::<EvmBalanceFailure>(original.canonical_bytes()).unwrap(),
                EvmBalanceFailure::AnchorChanged { previous, observed, .. }
                if previous.number == EvmU256::from_u64(7) && observed.number == EvmU256::from_u64(8))
        )));
        let cold = runtime.read(&run).await.unwrap();
        let RunViewState::Succeeded(cold_value) = cold.state() else {
            panic!("cold completion")
        };
        assert_eq!(
            serde_json::to_value(
                cold_value
                    .decode::<EvmBalanceCollectionCompletion<NoContext>>()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                value
                    .decode::<EvmBalanceCollectionCompletion<NoContext>>()
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(anchor_calls.load(Ordering::SeqCst), changed_call + 5);
        assert_eq!(balance_calls.load(Ordering::SeqCst), 3);
    }
}
