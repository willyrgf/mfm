//! Explicit recovery policy around the shipping native balance protocol.
use mfm_chain::balance::{
    BalanceCollectionMetadata, BalanceContext, BalanceExecutionConfig, BalanceRequest,
    BalanceSource, BalanceSourceDefinition, ConsolidateBalanceCollection, DecimalScale,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_evm::*;
use mfm_evm_live::{EvmReadProvider, EvmResources, ProviderFuture};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::*;
use mfm_runtime::{RunViewState, RunnableReason, Runtime};
use mfm_store::{MemoryStore, Store};
use mfm_values::Object;
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

type Context = BalanceContext<NoParams>;
struct RestartCollection;
impl Handler for RestartCollection {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        Ok(StableId::new("mfm.test.portfolio.restart-collection@1")?)
    }
    fn handle(
        _: &NoParams,
        classification: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, mfm_values::InvocationDiagnostic> {
        Ok(match classification {
            Classification::InputInvalidated => context
                .eligible_restart_targets()
                .first()
                .copied()
                .map(RecoveryRequest::Restart)
                .unwrap_or(RecoveryRequest::Stop),
            _ => RecoveryRequest::Stop,
        })
    }
}
struct CollectionStart;
impl CheckpointMarker for CollectionStart {
    type Context = Context;
}
struct RecoveryPolicy;
impl OperationDefaults for RecoveryPolicy {
    type Handler = RestartCollection;
    type Targets = (CollectionStart,);
}
impl ResolveDefaults<Definition> for RecoveryPolicy {
    fn resolve(_: &Definition) -> Result<PolicyValues<RestartCollection>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(1),
        })
    }
}
struct Definition {
    sources: Vec<BalanceSource>,
    execution: BalanceExecutionConfig,
}
impl OperationDefinition for Definition {
    type Body = (
        Checkpoint<CollectionStart>,
        Vec<Operation<BalanceSourceDefinition<NoParams>>>,
        Pure<ConsolidateBalanceCollection<NoParams>>,
    );
}
impl Plan<Context> for Definition {
    type Config = Self;
    fn plan<'a>(&'a self, _: &'a Context) -> Result<(&'a Self, Self::Body)> {
        Ok((
            self,
            (
                Checkpoint::default(),
                self.sources
                    .iter()
                    .enumerate()
                    .map(|(ordinal, source)| {
                        Operation::new(BalanceSourceDefinition::new(
                            source.clone(),
                            ordinal as u32,
                            DecimalScale::new(18).unwrap(),
                            self.execution.clone(),
                        ))
                    })
                    .collect(),
                Pure::default(),
            ),
        ))
    }
}
struct Provider {
    changed_call: usize,
    anchors: AtomicUsize,
    balances: AtomicUsize,
}
impl EvmReadProvider for Provider {
    fn observe<'a>(
        &'a self,
        reference: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move {
            let value = match intent.subject() {
                EvmReadSubject::ChainIdentity => EvmReadValue::ChainId(intent.chain_id()),
                EvmReadSubject::InitialAnchor | EvmReadSubject::ConfirmAnchor { .. } => {
                    let call = self.anchors.fetch_add(1, Ordering::SeqCst);
                    let height = if call < self.changed_call { 7 } else { 8 };
                    EvmReadValue::Anchor(EvmBlockAnchor {
                        number: EvmU256::from_u64(height),
                        hash: EvmHash::from_bytes([height as u8; 32]),
                    })
                }
                EvmReadSubject::NativeBalance { .. } => {
                    let call = self.balances.fetch_add(1, Ordering::SeqCst);
                    EvmReadValue::RawUnits(EvmU256::from_u64(if call == 0 { 100 } else { 200 }))
                }
                _ => panic!("native-only recovery fixture"),
            };
            Ok(EvmReadEvidence::returned(reference.clone(), value))
        })
    }
    fn observe_anchored_call<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        panic!("not a scalar read")
    }
}

// Both changed confirmation and changed initial anchor after a confirmed source require a
// collection restart while every earlier acknowledged frame and exact original remain immutable.
#[tokio::test]
async fn changed_anchor_restarts_the_real_collection_and_preserves_its_acknowledged_prefix() {
    for changed_call in [1, 2] {
        let chain = NonZeroU64::new(1).unwrap();
        let route = EvmBalanceRoute::new(chain, EvmEndpoint::new("restart-fixture").unwrap());
        let ledger =
            LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap());
        let sources: Vec<_> = (0..2)
            .map(|index| {
                BalanceSource::new(
                    format!("wallet-{index}"),
                    BalanceTarget::new(
                        ledger.clone(),
                        Object::from_value(&EvmBalanceTarget::new(
                            EvmAddress::from_bytes([index; 20]),
                            None,
                        ))
                        .unwrap(),
                    ),
                )
                .unwrap()
            })
            .collect();
        let input = Context::new(
            BalanceRequest::new(sources.clone(), DecimalScale::new(18).unwrap()).unwrap(),
            NoParams,
            BalanceCollectionMetadata::new(0, "collection".into(), route.route_ref().unwrap())
                .unwrap(),
        );
        let definition = Definition {
            sources,
            execution: BalanceExecutionConfig::new(
                route.route_ref().unwrap(),
                Object::from_value(&route).unwrap(),
            ),
        };
        let provider = Arc::new(Provider {
            changed_call,
            anchors: AtomicUsize::new(0),
            balances: AtomicUsize::new(0),
        });
        let resources = EvmResources::<Operation<Definition, RecoveryPolicy>>::new(
            vec![(route, provider.clone())],
            vec![],
        )
        .unwrap();
        let operation = Operation::<Definition, RecoveryPolicy>::from(definition);
        let program = compile(
            EntryPointId::new("mfm.test.portfolio/restart@1").unwrap(),
            &operation,
            &input,
            &resources,
            ProgramLimits::new(1),
        )
        .unwrap();
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([changed_call as u8 + 10; 32]));
        let yielded = runtime.start(run.clone(), &program, &input).await.unwrap();
        assert!(matches!(
            yielded.state(),
            RunViewState::Runnable {
                reason: RunnableReason::Restart { .. },
                ..
            }
        ));
        let prefix = store.load_run(&run, None).await.unwrap().unwrap();
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(operation);
        let cold = load(&bytes, &resources).unwrap();
        assert_eq!(
            runtime.read(&run, &cold).await.unwrap().head_digest(),
            yielded.head_digest()
        );
        assert_eq!(provider.anchors.load(Ordering::SeqCst), changed_call + 1);
        let terminal = runtime.resume(&run, &cold).await.unwrap();
        let value = terminal.success().expect("coherent restarted collection");
        let completion = value
            .decode::<mfm_chain::balance::BalanceCollectionCompletion<NoParams>>()
            .unwrap();
        let (context, total) = completion.into_parts();
        assert_eq!(total, "400");
        assert_eq!(context.completed().len(), 2);
        for balance in context.completed() {
            let point = balance
                .observed_at()
                .native()
                .decode::<EvmBlockPoint>()
                .unwrap();
            assert_eq!(point.number(), &EvmU256::from_u64(8));
            assert_eq!(point.hash(), &EvmHash::from_bytes([8; 32]));
        }
        let retained = store
            .load_run(&run, Some(prefix.head().head_sequence()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.admission(), prefix.admission());
        assert_eq!(retained.probe().unwrap(), prefix.latest());
        let frame = mfm_journal::decode_frame(prefix.latest()).unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(frame.payload().as_bytes()).unwrap();
        let original = serde_json::from_value::<Object>(
            payload["operation"]["recovered"]["failure"]["domain"]["original"].clone(),
        )
        .unwrap();
        assert!(
            matches!(original.decode::<EvmBalanceFailure>().unwrap(), EvmBalanceFailure::AnchorChanged { previous, observed, .. } if previous.number == EvmU256::from_u64(7) && observed.number == EvmU256::from_u64(8))
        );
        let inspected = runtime.read(&run, &cold).await.unwrap();
        assert_eq!(inspected.success(), Some(value));
        assert_eq!(provider.anchors.load(Ordering::SeqCst), changed_call + 5);
        assert_eq!(provider.balances.load(Ordering::SeqCst), 3);
    }
}
