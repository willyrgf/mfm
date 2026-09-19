use std::num::NonZeroU64;

use mfm_capabilities::{AdapterError, ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_chain::balance::BalanceRead;
use mfm_chain::balance::BalanceSourceDefinition;
use mfm_evm::{
    EvmBalanceBinding, EvmNativeBalance, EvmOperationalError, EvmReadEvidence, EvmReadIntent,
    EvmTokenBalance,
};
use mfm_evm::{EvmBalanceRoute, EvmBalanceTarget, EvmEndpoint};
use mfm_evm_live::client::portfolio::{
    admit_enrichment, admit_snapshot, render_snapshot, snapshot_config, EvmPortfolioConfig,
};
use mfm_ids::{ContentRef, EntryPointId, StableId};
use mfm_portfolio::{
    PortfolioEnrichmentOperation, PortfolioSnapshotOperation, PortfolioSnapshotSelector,
};
use mfm_program::{
    compile, load, BindRead, CapabilityFamily, ProgramEnvironment, ProgramLimits, Resolve,
};
use mfm_values::InvocationDiagnostic;
use std::{future::Future, pin::Pin};

#[derive(Clone)]
enum Provider {
    Forbidden,
    Scripted,
    Pause(std::sync::Arc<tokio::sync::Notify>),
    ConfirmOnly,
    RejectToken,
}
struct Resources<const COLD: bool>(Provider);
impl<const COLD: bool> ProgramEnvironment for Resources<COLD> {
    type Sources = (PortfolioSnapshotOperation, PortfolioEnrichmentOperation);
}
impl<const COLD: bool> CapabilityFamily<BalanceRead> for Resources<COLD> {
    type Implementations = (EvmNativeBalance, EvmTokenBalance);
}
impl<K: mfm_values::MfmValue> Resolve<BalanceSourceDefinition<K>, BalanceRead>
    for Resources<false>
{
    fn implementation(source: &BalanceSourceDefinition<K>) -> mfm_program::Result<StableId> {
        Ok(
            if source
                .source()
                .target()
                .native()
                .decode::<EvmBalanceTarget>()
                .map_err(mfm_program::ProgramError::Diagnostic)?
                .token()
                .is_some()
            {
                EvmTokenBalance::implementation_id()?
            } else {
                EvmNativeBalance::implementation_id()?
            },
        )
    }
}
impl<const COLD: bool> ReadAdapter<EvmReadIntent, EvmReadEvidence, EvmOperationalError>
    for Resources<COLD>
{
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        intent_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<EvmReadEvidence, AdapterError<EvmOperationalError>>>
                + Send
                + 'a,
        >,
    > {
        assert!(
            !matches!(self.0, Provider::Forbidden),
            "construction and terminal inspection must not invoke provider IO"
        );
        Box::pin(async move {
            use mfm_evm::{EvmReadSubject, EvmReadValue, EvmU256};
            if matches!(self.0, Provider::ConfirmOnly) {
                assert!(
                    matches!(intent.subject(), EvmReadSubject::ConfirmAnchor { .. }),
                    "cold continuation must not repeat preparation or balance reads"
                );
            }
            if let Provider::Pause(entered) = &self.0 {
                if intent.source_ordinal() == 1
                    && matches!(intent.subject(), EvmReadSubject::ConfirmAnchor { .. })
                {
                    entered.notify_one();
                    std::future::pending::<()>().await;
                }
            }
            if matches!(self.0, Provider::RejectToken)
                && matches!(intent.subject(), EvmReadSubject::TokenBalance { .. })
            {
                return Ok(EvmReadEvidence::rejected(intent_ref.clone()));
            }
            let value = match intent.subject() {
                EvmReadSubject::ChainIdentity => EvmReadValue::ChainId(intent.chain_id()),
                EvmReadSubject::InitialAnchor => EvmReadValue::Anchor(mfm_evm::EvmBlockAnchor {
                    number: EvmU256::from_u64(7),
                    hash: mfm_evm::EvmHash::from_bytes([7; 32]),
                }),
                EvmReadSubject::ConfirmAnchor { anchor } => EvmReadValue::Anchor(anchor.clone()),
                EvmReadSubject::NativeBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(42))
                }
                EvmReadSubject::TokenBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(84))
                }
                EvmReadSubject::TokenDecimals { .. } => {
                    EvmReadValue::TokenDecimals(mfm_evm::EvmTokenDecimals::new(0).unwrap())
                }
            };
            Ok(EvmReadEvidence::returned(intent_ref.clone(), value))
        })
    }
}
impl<C, I, const COLD: bool> BindRead<C, I> for Resources<COLD>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<
        C,
        Binding = EvmBalanceBinding,
        NativeIntent = EvmReadIntent,
        NativeEvidence = EvmReadEvidence,
        OperationalError = EvmOperationalError,
    >,
{
    type Adapter = Self;
    fn bind_read(&self, _: &EvmBalanceBinding) -> Result<Self, InvocationDiagnostic> {
        Ok(Self(self.0.clone()))
    }
}

// A snapshot must commit to its selected routes in both Program identity and input, rejecting
// incomplete or misordered route sets.
#[test]
fn planning_uses_the_selected_routes_and_rejects_unusable_route_sets() {
    let config: serde_json::Value = serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [
            {
                "correlation": "chain-one",
                "request": {
                    "sources": [{
                        "source_id": "wallet.one",
                        "chain_id": 1,
                        "address": "0x0000000000000000000000000000000000000001",
                        "token": null
                    }],
                    "decimals": 18
                }
            },
            {
                "correlation": "chain-two",
                "request": {
                    "sources": [{
                        "source_id": "wallet.two",
                        "chain_id": 2,
                        "address": "0x0000000000000000000000000000000000000002",
                        "token": null
                    }],
                    "decimals": 18
                }
            }
        ]
    }))
    .expect("Portfolio config");
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": "portfolio-example",
        "quote": "usd"
    }))
    .expect("selector");
    let alpha = EvmBalanceRoute::new(
        NonZeroU64::new(1).expect("nonzero chain"),
        EvmEndpoint::new("alpha").unwrap(),
    );
    let beta = EvmBalanceRoute::new(
        NonZeroU64::new(2).expect("nonzero chain"),
        EvmEndpoint::new("beta").unwrap(),
    );

    let input = admit_snapshot(&serde_json::from_value::<EvmPortfolioConfig>(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":&[alpha.clone(), beta.clone()].iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap(), None)
    .expect("planned snapshot");
    let program = compile(
        EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
        &PortfolioSnapshotOperation::default(),
        &input,
        &Resources::<false>(Provider::Forbidden),
        ProgramLimits::new(0),
    )
    .unwrap();
    let decoded = load(
        program.canonical_bytes(),
        &Resources::<true>(Provider::Forbidden),
    )
    .expect("persisted planned Program");
    assert_eq!(decoded.content_ref(), program.content_ref());
    assert_eq!(decoded.canonical_bytes(), program.canonical_bytes());
    let mut noncanonical = program.canonical_bytes().to_vec();
    noncanonical.push(b' ');
    assert!(matches!(
        load(&noncanonical, &Resources::<true>(Provider::Forbidden)),
        Err(mfm_program::ProgramError::Encoding(_))
    ));
    assert_eq!(
        input.collections()[0].executions()[0].route_ref(),
        &alpha.physical_target().unwrap().binding_ref().unwrap()
    );
    assert_eq!(
        input.collections()[1].executions()[0].route_ref(),
        &beta.physical_target().unwrap().binding_ref().unwrap()
    );
    let input = serde_json::to_value(input).unwrap();

    let replacement = EvmBalanceRoute::new(
        NonZeroU64::new(2).expect("nonzero chain"),
        EvmEndpoint::new("replacement").unwrap(),
    );
    let replacement_input = admit_snapshot(&serde_json::from_value::<EvmPortfolioConfig>(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":&[alpha.clone(), replacement].iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap(), None)
    .expect("replacement plan");
    let replacement_program = compile(
        EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
        &PortfolioSnapshotOperation::default(),
        &replacement_input,
        &Resources::<false>(Provider::Forbidden),
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_ne!(program.content_ref(), replacement_program.content_ref());
    assert_ne!(
        input,
        serde_json::to_value(replacement_input).expect("replacement input")
    );

    for targets in [
        Vec::new(),
        vec![beta.clone(), alpha.clone()],
        vec![alpha.clone()],
        vec![alpha.clone(), alpha],
    ] {
        let native: EvmPortfolioConfig = serde_json::from_value(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":targets.iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap();
        assert!(admit_snapshot(&native, None).is_err());
    }
}

// Both maintained callers execute the same native/token protocols with distinct typed continuations.
// The provider is scripted; State execution, Program loading, Runtime and Store are production code.
#[tokio::test]
async fn snapshot_and_enrichment_execute_native_and_token_sources_and_inspect_cold_without_io() {
    use mfm_ids::{DigestBytes, RunId};
    use mfm_portfolio::{PortfolioEnrichmentOutput, PortfolioSnapshotOutput};
    use mfm_runtime::Runtime;
    use mfm_store::MemoryStore;
    use std::sync::Arc;
    let config: serde_json::Value = serde_json::from_value(serde_json::json!({
        "portfolio_id":"runtime-proof", "quotes":["usd"], "collections":[{
            "correlation":"native-token", "request":{"decimals":0,"sources":[
                {"source_id":"native","chain_id":1,"address":"0x0000000000000000000000000000000000000001","token":null},
                {"source_id":"token","chain_id":1,"address":"0x0000000000000000000000000000000000000001","token":"0x0000000000000000000000000000000000000002"}
            ]}
        }]
    })).unwrap();
    let selector: PortfolioSnapshotSelector =
        serde_json::from_value(serde_json::json!({"target":"runtime-proof","quote":"usd"}))
            .unwrap();
    let route = EvmBalanceRoute::new(
        NonZeroU64::new(1).unwrap(),
        EvmEndpoint::new("scripted").unwrap(),
    );
    let snapshot_input = admit_snapshot(&serde_json::from_value::<EvmPortfolioConfig>(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":std::slice::from_ref(&route).iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap(), None)
    .unwrap();
    let enrichment_config: EvmPortfolioConfig = serde_json::from_value(serde_json::json!({"entry_point":"mfm.portfolio/enrich@1","input":{"selector":selector,"portfolio":config,"routes":[{"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()}]}})).unwrap();
    let enrichment_input = admit_enrichment(&enrichment_config, None).unwrap();
    drop(config);
    let snapshot = compile(
        EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
        &PortfolioSnapshotOperation::default(),
        &snapshot_input,
        &Resources::<false>(Provider::Forbidden),
        ProgramLimits::new(0),
    )
    .unwrap();
    let enrichment = compile(
        EntryPointId::new(mfm_portfolio::PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID).unwrap(),
        &PortfolioEnrichmentOperation::default(),
        &enrichment_input,
        &Resources::<false>(Provider::Forbidden),
        ProgramLimits::new(0),
    )
    .unwrap();
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store);
    let snapshot_run = RunId::from_digest(DigestBytes::from_array([91; 32]));
    let enrichment_run = RunId::from_digest(DigestBytes::from_array([92; 32]));
    let snapshot = load(
        snapshot.canonical_bytes(),
        &Resources::<true>(Provider::Scripted),
    )
    .unwrap();
    let enrichment = load(
        enrichment.canonical_bytes(),
        &Resources::<true>(Provider::Scripted),
    )
    .unwrap();
    let result = runtime
        .execute(snapshot_run.clone(), &snapshot, &snapshot_input)
        .await
        .unwrap();
    let output = result
        .success()
        .unwrap()
        .decode::<PortfolioSnapshotOutput>()
        .unwrap();
    let wire = render_snapshot(&output).unwrap();
    assert_eq!(
        wire["report"]["totals_by_quote"][0]["total_value_dec"],
        "126"
    );
    assert_eq!(
        wire["snapshot"]["collections"][0]["holdings"][0]["raw_units"],
        "42"
    );
    assert_eq!(
        wire["snapshot"]["collections"][0]["holdings"][1]["raw_units"],
        "84"
    );
    let enriched = runtime
        .execute(enrichment_run.clone(), &enrichment, &enrichment_input)
        .await
        .unwrap();
    assert!(enriched
        .success()
        .unwrap()
        .decode::<PortfolioSnapshotOutput>()
        .is_err());
    let selected = snapshot_config(
        &enriched
            .success()
            .unwrap()
            .decode::<PortfolioEnrichmentOutput>()
            .unwrap(),
    )
    .unwrap();
    let selected = serde_json::to_value(selected).unwrap()["input"]["portfolio"].clone();
    assert_eq!(
        selected["collections"][0]["request"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(snapshot);
    drop(enrichment);
    for (run, expected) in [(snapshot_run, result), (enrichment_run, enriched)] {
        let document = runtime.program_document(&run).await.unwrap();
        let program = load(
            document.canonical_bytes(),
            &Resources::<true>(Provider::Forbidden),
        )
        .unwrap();
        let read = runtime.read(&run, &program).await.unwrap();
        let resumed = runtime.resume(&run, &program).await.unwrap();
        assert_eq!(read.success(), expected.success());
        assert_eq!(resumed.success(), expected.success());
        assert_eq!(read.head_digest(), resumed.head_digest());
    }
}

#[tokio::test]
async fn both_continuations_restore_the_last_candidate_before_confirmation_without_rereading_balances(
) {
    use mfm_ids::{DigestBytes, RunId};
    use mfm_portfolio::{PortfolioEnrichmentOutput, PortfolioSnapshotOutput};
    use mfm_runtime::Runtime;
    use mfm_store::MemoryStore;
    use std::sync::Arc;
    for enrichment in [false, true] {
        let config: serde_json::Value = serde_json::from_value(serde_json::json!({
            "portfolio_id":"candidate-proof", "quotes":["usd"], "collections":[{
                "correlation":"native-token", "request":{"decimals":0,"sources":[
                    {"source_id":"native","chain_id":1,"address":"0x0000000000000000000000000000000000000001","token":null},
                    {"source_id":"token","chain_id":1,"address":"0x0000000000000000000000000000000000000001","token":"0x0000000000000000000000000000000000000002"}
                ]}
            }]
        })).unwrap();
        let selector: PortfolioSnapshotSelector =
            serde_json::from_value(serde_json::json!({"target":"candidate-proof","quote":"usd"}))
                .unwrap();
        let route = EvmBalanceRoute::new(
            NonZeroU64::new(1).unwrap(),
            EvmEndpoint::new("candidate-rpc").unwrap(),
        );
        let snapshot_input = admit_snapshot(&serde_json::from_value::<EvmPortfolioConfig>(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":std::slice::from_ref(&route).iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap(), None)
        .unwrap();
        let enrichment_config: EvmPortfolioConfig = serde_json::from_value(serde_json::json!({"entry_point":"mfm.portfolio/enrich@1","input":{"selector":selector,"portfolio":config,"routes":[{"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()}]}})).unwrap();
        let enrichment_input = admit_enrichment(&enrichment_config, None).unwrap();
        drop(config);
        let program = if enrichment {
            compile(
                EntryPointId::new(mfm_portfolio::PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID).unwrap(),
                &PortfolioEnrichmentOperation::default(),
                &enrichment_input,
                &Resources::<false>(Provider::Forbidden),
                ProgramLimits::new(0),
            )
        } else {
            compile(
                EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
                &PortfolioSnapshotOperation::default(),
                &snapshot_input,
                &Resources::<false>(Provider::Forbidden),
                ProgramLimits::new(0),
            )
        }
        .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let document = program.canonical_bytes().to_vec();
        drop(program);
        let program = load(
            &document,
            &Resources::<true>(Provider::Pause(entered.clone())),
        )
        .unwrap();
        let store = Arc::new(MemoryStore::new());
        let executing_store = store.clone();
        let run = RunId::from_digest(DigestBytes::from_array([120 + u8::from(enrichment); 32]));
        let executing_run = run.clone();
        let task = tokio::spawn(async move {
            let runtime = Runtime::new(executing_store);
            if enrichment {
                runtime
                    .execute(executing_run, &program, &enrichment_input)
                    .await
            } else {
                runtime
                    .execute(executing_run, &program, &snapshot_input)
                    .await
            }
        });
        entered.notified().await;
        let runtime = Runtime::new(store);
        let retained = runtime.program_document(&run).await.unwrap();
        let cold_program = load(
            retained.canonical_bytes(),
            &Resources::<true>(Provider::ConfirmOnly),
        )
        .unwrap();
        let pending = runtime.read(&run, &cold_program).await.unwrap();
        assert!(pending.success().is_none());
        assert!(pending.failure().is_none());
        assert!(matches!(
            pending.state(),
            mfm_runtime::RunViewState::Runnable { .. }
        ));
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        assert_eq!(
            runtime
                .read(&run, &cold_program)
                .await
                .unwrap()
                .head_digest(),
            pending.head_digest()
        );
        let completed = runtime.resume(&run, &cold_program).await.unwrap();
        if enrichment {
            let output = completed
                .success()
                .unwrap()
                .decode::<PortfolioEnrichmentOutput>()
                .unwrap();
            let config = snapshot_config(&output).unwrap();
            assert_eq!(
                serde_json::to_value(config).unwrap()["input"]["portfolio"]["collections"][0]
                    ["request"]["sources"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        } else {
            let output = completed
                .success()
                .unwrap()
                .decode::<PortfolioSnapshotOutput>()
                .unwrap();
            assert_eq!(
                serde_json::to_value(output).unwrap()["report"]["totals_by_quote"][0]
                    ["total_value_dec"],
                "126"
            );
        }
        assert!(completed.head_sequence() > pending.head_sequence());
    }
}

// The failed collection is identified from retained domain input, never from an expanded State
// index. Earlier completed collections and the native original must survive cold projection.
#[tokio::test]
async fn failed_second_collection_retains_ordinal_source_and_native_original_hot_and_cold() {
    use mfm_chain::balance::{ObserveBalanceFailure, PreparedBalance, ReadBalanceAt};
    use mfm_ids::{DigestBytes, RunId};
    use mfm_portfolio::PortfolioContinuation;
    use mfm_runtime::{Failure, Runtime, StateCall};
    use mfm_store::MemoryStore;
    use std::sync::Arc;
    let config: serde_json::Value = serde_json::from_value(serde_json::json!({
        "portfolio_id":"failure-proof", "quotes":["usd"], "collections":[
            {"correlation":"completed", "request":{"decimals":0,"sources":[
                {"source_id":"first-native","chain_id":1,"address":"0x0000000000000000000000000000000000000001","token":null}
            ]}},
            {"correlation":"failed", "request":{"decimals":0,"sources":[
                {"source_id":"second-native","chain_id":2,"address":"0x0000000000000000000000000000000000000001","token":null},
                {"source_id":"second-token","chain_id":2,"address":"0x0000000000000000000000000000000000000001","token":"0x0000000000000000000000000000000000000002"}
            ]}}
        ]
    })).unwrap();
    let selector: PortfolioSnapshotSelector =
        serde_json::from_value(serde_json::json!({"target":"failure-proof","quote":"usd"}))
            .unwrap();
    let targets: Vec<_> = [1, 2]
        .into_iter()
        .map(|chain| {
            EvmBalanceRoute::new(
                NonZeroU64::new(chain).unwrap(),
                EvmEndpoint::new(format!("chain-{chain}")).unwrap(),
            )
        })
        .collect();
    let native_config: EvmPortfolioConfig = serde_json::from_value(serde_json::json!({"entry_point":"mfm.portfolio/snapshot@1","input":{"selector":selector,"portfolio":config,"routes":targets.iter().map(|route| serde_json::json!({"chain_id":route.chain_id(),"endpoint_id":route.endpoint().endpoint_id()})).collect::<Vec<_>>()}})).unwrap();
    let input = admit_snapshot(&native_config, None).unwrap();
    drop(config);
    let program = compile(
        EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
        &PortfolioSnapshotOperation::default(),
        &input,
        &Resources::<false>(Provider::RejectToken),
        ProgramLimits::new(0),
    )
    .unwrap();
    let runtime = Runtime::new(Arc::new(MemoryStore::new()));
    let run = RunId::from_digest(DigestBytes::from_array([125; 32]));
    let result = runtime
        .execute(run.clone(), &program, &input)
        .await
        .unwrap();
    let report = result
        .failure()
        .expect("authenticated token rejection must fail the snapshot");
    let document = runtime.program_document(&run).await.unwrap();
    drop(program);
    let program = load(
        document.canonical_bytes(),
        &Resources::<true>(Provider::Forbidden),
    )
    .unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
    let resumed = runtime.resume(&run, &program).await.unwrap();
    for observed in [report, cold.failure().unwrap(), resumed.failure().unwrap()] {
        assert_eq!(observed.canonical_bytes(), report.canonical_bytes());
        assert_eq!(
            observed
                .failure()
                .original()
                .decode::<ObserveBalanceFailure>()
                .unwrap(),
            ObserveBalanceFailure::ObservationUnavailable
        );
        let Failure::Domain {
            call:
                StateCall::Read {
                    call,
                    intent,
                    evidence,
                },
            ..
        } = observed.failure()
        else {
            panic!("retain the complete failed observation")
        };
        let prepared = call
            .input()
            .decode::<PreparedBalance<PortfolioContinuation>>()
            .unwrap();
        let context = prepared.context();
        assert_eq!(context.metadata().collection_ordinal(), 1);
        assert_eq!(context.metadata().correlation(), "failed");
        assert_eq!(context.completed().len(), 1);
        assert_eq!(context.active_source().unwrap().source_id(), "second-token");
        assert_eq!(
            serde_json::to_value(context.caller()).unwrap()["completed_collections"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let intent = intent.decode::<ReadBalanceAt>().unwrap();
        assert_eq!(intent.route_ref(), context.metadata().route_ref());
        assert_eq!(intent.target(), context.active_source().unwrap().target());
        let native = evidence.decode::<EvmReadEvidence>().unwrap();
        assert!(matches!(native, EvmReadEvidence::Rejected { .. }));
    }
}
