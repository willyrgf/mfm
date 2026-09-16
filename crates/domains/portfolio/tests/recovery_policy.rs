#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct EmptyContext {}

use mfm_evm::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_portfolio::{MapEvmBalanceFailure, PortfolioSnapshotFailure};
use mfm_program::*;
use mfm_runtime::{RunViewState, RunnableReason, Runtime, RuntimeAssemblyBuilder};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

type Context = EvmBalanceContext<EmptyContext>;

struct RestartCollection;
impl Handler for RestartCollection {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.portfolio.restart-collection@1").unwrap())
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
struct RecoveringCollection(CollectEvmBalances<EmptyContext>);
impl Operation for RecoveringCollection {
    type Input = Context;
    type Output = EvmBalanceCollectionCompletion<EmptyContext>;
    type Failure = PortfolioSnapshotFailure;
    fn validate_input(&self, input: &Context) -> mfm_program::Result<()> {
        self.0.validate_input(input)
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        let checkpoint = body.checkpoint::<Context>()?;
        body.handler(HandlerBinding::new::<RestartCollection>(NoParams)?.checkpoint(&checkpoint)?)?;
        body.allowances(RecoveryAllowances::new(0, 1))?;
        body.operation::<CollectEvmBalances<EmptyContext>, MapEvmBalanceFailure>(&self.0, NoParams)
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
            EmptyContext {},
            0,
            "collection".into(),
            target.binding_ref().unwrap(),
        )
        .unwrap();
        let operation = RecoveringCollection(
            CollectEvmBalances::new(target.binding_ref().unwrap(), request.clone()).unwrap(),
        );
        let program = expand_program(
            EntryPointId::new("mfm.test.portfolio/restart@1").unwrap(),
            &operation,
            &input,
            ProgramLimits::new(1),
        )
        .unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_map::<MapEvmBalanceFailure>().unwrap();
        builder
            .register_read::<CheckChainIdentity<EmptyContext>, EvmChainIdentityRead>()
            .unwrap();
        builder
            .register_read::<ReadInitialAnchor<EmptyContext>, EvmAnchorRead>()
            .unwrap();
        builder
            .register_read::<ConfirmBalanceAnchor<EmptyContext>, EvmAnchorRead>()
            .unwrap();
        builder
            .register_read::<ReadNativeBalance<EmptyContext>, EvmBalanceRead>()
            .unwrap();
        builder
            .register_pure::<ConsolidateBalanceCollection<EmptyContext>>()
            .unwrap();
        builder.register_handler::<RestartCollection>().unwrap();
        builder.register_handler::<Stop>().unwrap();
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
        let prefix = store.load_run(&run, None).await.unwrap().unwrap();
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_digest(), yielded.head_digest());
        assert_eq!(anchor_calls.load(Ordering::SeqCst), changed_call + 1);
        let terminal = runtime.resume(&run).await.unwrap();
        let RunViewState::Succeeded(value) = terminal.state() else {
            panic!("coherent restarted collection")
        };
        let (_, _, _, number, hash, balances, total) = value
            .decode::<EvmBalanceCollectionCompletion<EmptyContext>>()
            .unwrap()
            .into_parts();
        assert_eq!(number, "8");
        assert_eq!(hash, EvmHash::from_bytes([8; 32]).to_string());
        assert_eq!(balances.len(), 2);
        assert_eq!(total, "400");
        let retained = store
            .load_run(&run, Some(prefix.head().head_sequence()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.head_sequence(), yielded.head_sequence() + 9);
        assert_eq!(retained.admission(), prefix.admission());
        assert_eq!(retained.probe().unwrap(), prefix.latest());
        let frame = mfm_journal::decode_frame(prefix.latest()).unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(frame.payload().as_bytes()).unwrap();
        let original = serde_json::from_value::<mfm_values::Object>(
            payload["operation"]["recovered"]["failure"]["domain"]["original"].clone(),
        )
        .unwrap();
        assert!(matches!(original.decode::<EvmBalanceFailure>().unwrap(),
            EvmBalanceFailure::AnchorChanged { previous, observed, .. }
                if previous.number == EvmU256::from_u64(7) && observed.number == EvmU256::from_u64(8)));
        let cold = runtime.read(&run).await.unwrap();
        let RunViewState::Succeeded(cold_value) = cold.state() else {
            panic!("cold completion")
        };
        assert_eq!(
            serde_json::to_value(
                cold_value
                    .decode::<EvmBalanceCollectionCompletion<EmptyContext>>()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                value
                    .decode::<EvmBalanceCollectionCompletion<EmptyContext>>()
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(anchor_calls.load(Ordering::SeqCst), changed_call + 5);
        assert_eq!(balance_calls.load(Ordering::SeqCst), 3);
    }
}
