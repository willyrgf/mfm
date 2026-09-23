use super::*;
use crate::{EvmReadProvider, ProviderFuture};
use mfm_evm::*;
use mfm_ids::{ContentRef, DigestBytes, RunId};
use mfm_program::{compile, load, ProgramLimits};
use mfm_runtime::Runtime;
use mfm_store::MemoryStore;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    Rejected,
    IntegrityBlocked,
    MaximumTokenBalance,
}
struct Provider(AtomicUsize, Option<(usize, Fault)>);
impl EvmReadProvider for Provider {
    fn observe<'a>(
        &'a self,
        reference: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move {
            let call = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((at, fault)) = self.1 {
                if at == call
                    || (at == 0 && matches!(intent.subject(), EvmReadSubject::TokenBalance { .. }))
                {
                    return Ok(match fault {
                        Fault::MaximumTokenBalance => EvmReadEvidence::returned(
                            reference.clone(),
                            EvmReadValue::RawUnits(EvmU256::new("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap()),
                        ),
                        Fault::Rejected => EvmReadEvidence::rejected(reference.clone()),
                        Fault::IntegrityBlocked => {
                            EvmReadEvidence::integrity_blocked(reference.clone())
                        }
                    });
                }
            }
            let value = match intent.subject() {
                EvmReadSubject::ChainIdentity => EvmReadValue::ChainId(intent.chain_id()),
                EvmReadSubject::InitialAnchor | EvmReadSubject::ConfirmAnchor { .. } => {
                    EvmReadValue::Anchor(EvmBlockAnchor {
                        number: EvmU256::from_u64(9),
                        hash: EvmHash::from_bytes([3; 32]),
                    })
                }
                EvmReadSubject::TokenDecimals { .. } => {
                    EvmReadValue::TokenDecimals(EvmTokenDecimals::new(0).unwrap())
                }
                EvmReadSubject::NativeBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(42))
                }
                EvmReadSubject::TokenBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(84))
                }
            };
            Ok(EvmReadEvidence::returned(reference.clone(), value))
        })
    }
    fn observe_anchored_call<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        panic!("Portfolio does not use scalar contract calls")
    }
}
enum ReadMode {
    PauseConfirmation(Arc<tokio::sync::Notify>),
    ConfirmationOnly,
    Forbidden,
}
struct ControlledProvider {
    provider: Arc<Provider>,
    mode: ReadMode,
}
impl EvmReadProvider for ControlledProvider {
    fn observe<'a>(
        &'a self,
        reference: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move {
            match &self.mode {
                ReadMode::Forbidden => panic!("terminal inspection cannot call a provider"),
                ReadMode::ConfirmationOnly => assert!(
                    matches!(intent.subject(), EvmReadSubject::ConfirmAnchor { .. }),
                    "cold resume must not reread balances"
                ),
                ReadMode::PauseConfirmation(entered)
                    if intent.source_ordinal() == 1
                        && matches!(intent.subject(), EvmReadSubject::ConfirmAnchor { .. }) =>
                {
                    entered.notify_one();
                    std::future::pending::<()>().await;
                }
                ReadMode::PauseConfirmation(_) => {}
            }
            self.provider.observe(reference, intent).await
        })
    }
    fn observe_anchored_call<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        panic!("Portfolio does not use scalar contract calls")
    }
}

fn configuration(enrichment: bool) -> EvmPortfolioConfig {
    serde_json::from_value(serde_json::json!({
        "entry_point": if enrichment { PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID } else { PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID },
        "input": {
            "routes": [{"chain_id":1,"endpoint_id":"expected"}],
            "selector": {"target":"portfolio","quote":"usd"},
            "portfolio": {"portfolio_id":"portfolio", "quotes":["usd"], "collections":[{
                "correlation":"collection", "request":{"decimals":0,"sources":[
                    {"source_id":"native","chain_id":1,"address":EvmAddress::from_bytes([1;20]),"token":null},
                    {"source_id":"token","chain_id":1,"address":EvmAddress::from_bytes([1;20]),"token":EvmAddress::from_bytes([2;20])}
                ]}
            }]}
        }
    })).unwrap()
}
fn resources(endpoint: &str, provider: Arc<dyn EvmReadProvider>) -> PortfolioResources {
    PortfolioResources::new(
        vec![(
            EvmBalanceRoute::new(
                std::num::NonZeroU64::new(1).unwrap(),
                EvmEndpoint::new(endpoint).unwrap(),
            ),
            provider,
        )],
        vec![],
    )
    .unwrap()
}

#[tokio::test]
async fn both_continuations_execute_cold_and_publish_without_source_configuration() {
    for enrichment in [false, true] {
        let config = configuration(enrichment);
        let expected_wire = serde_json::to_value(configuration(false)).unwrap();
        let provider = Arc::new(Provider(AtomicUsize::new(0), None));
        let installed = resources("expected", provider.clone());
        let (program, input) = if enrichment {
            let input = admit_enrichment(&config, None).unwrap();
            let program = compile(
                config.entry_point_id().unwrap(),
                &mfm_portfolio::PortfolioEnrichmentOperation::default(),
                &input,
                &installed,
                ProgramLimits::new(0),
            )
            .unwrap();
            (program, Object::from_value(&input).unwrap())
        } else {
            let input = admit_snapshot(&config, None).unwrap();
            let program = compile(
                config.entry_point_id().unwrap(),
                &mfm_portfolio::PortfolioSnapshotOperation::default(),
                &input,
                &installed,
                ProgramLimits::new(0),
            )
            .unwrap();
            (program, Object::from_value(&input).unwrap())
        };
        assert_eq!(provider.0.load(Ordering::SeqCst), 0);
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(config);
        drop(installed);
        let wrong = resources("different", provider.clone());
        assert!(load(&bytes, &wrong).is_err());
        assert_eq!(provider.0.load(Ordering::SeqCst), 0);
        let entered = Arc::new(tokio::sync::Notify::new());
        let cold = resources(
            "expected",
            Arc::new(ControlledProvider {
                provider: provider.clone(),
                mode: ReadMode::PauseConfirmation(entered.clone()),
            }),
        );
        let reconstructed = load(&bytes, &cold).unwrap();
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let id = RunId::from_digest(DigestBytes::from_array(
            [if enrichment { 1 } else { 2 }; 32],
        ));
        let executing_id = id.clone();
        let task = tokio::spawn(async move {
            let runtime = Runtime::new(store);
            if enrichment {
                runtime
                    .execute(
                        executing_id,
                        &reconstructed,
                        &input.decode::<PortfolioEnrichmentInput>().unwrap(),
                    )
                    .await
            } else {
                runtime
                    .execute(
                        executing_id,
                        &reconstructed,
                        &input.decode::<PortfolioSnapshotInput>().unwrap(),
                    )
                    .await
            }
        });
        entered.notified().await;
        drop(cold);
        let document = runtime.program_document(&id).await.unwrap();
        let cold = load(
            document.canonical_bytes(),
            &resources(
                "expected",
                Arc::new(ControlledProvider {
                    provider: provider.clone(),
                    mode: ReadMode::ConfirmationOnly,
                }),
            ),
        )
        .unwrap();
        let pending = runtime.read(&id, &cold).await.unwrap();
        assert!(matches!(
            pending.state(),
            mfm_runtime::RunViewState::Runnable { .. }
        ));
        assert!(pending.success().is_none());
        assert!(pending.failure().is_none());
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        assert_eq!(
            runtime.read(&id, &cold).await.unwrap().head_digest(),
            pending.head_digest()
        );
        let result = runtime.resume(&id, &cold).await.unwrap();
        assert!(result.head_sequence() > pending.head_sequence());
        drop(cold);
        let terminal = load(
            document.canonical_bytes(),
            &resources(
                "expected",
                Arc::new(ControlledProvider {
                    provider: provider.clone(),
                    mode: ReadMode::Forbidden,
                }),
            ),
        )
        .unwrap();
        let inspected = runtime.read(&id, &terminal).await.unwrap();
        let resumed = runtime.resume(&id, &terminal).await.unwrap();
        assert_eq!(inspected.success(), result.success());
        assert_eq!(resumed.head_digest(), result.head_digest());
        if enrichment {
            let output = result
                .success()
                .unwrap()
                .decode::<PortfolioEnrichmentOutput>()
                .unwrap();
            assert!(result
                .success()
                .unwrap()
                .decode::<PortfolioSnapshotOutput>()
                .is_err());
            let published = snapshot_config(&output).unwrap();
            assert_eq!(serde_json::to_value(published).unwrap(), expected_wire);
        } else {
            let output = result
                .success()
                .unwrap()
                .decode::<PortfolioSnapshotOutput>()
                .unwrap();
            let rendered = render_snapshot(&output).unwrap();
            assert_eq!(
                rendered["report"]["totals_by_quote"][0]["total_value_dec"],
                "126"
            );
            assert_eq!(
                rendered["snapshot"]["collections"][0]["holdings"][0]["amount_dec"],
                "42"
            );
            assert_eq!(
                rendered["snapshot"]["collections"][0]["holdings"][1]["amount_dec"],
                "84"
            );
        }
    }
}

#[tokio::test]
async fn real_native_failures_project_both_continuations_hot_and_cold_without_config() {
    for (enrichment, shared) in [(false, false), (true, false), (false, true)] {
        let mut wire = serde_json::to_value(configuration(enrichment)).unwrap();
        let mut prior = wire["input"]["portfolio"]["collections"][0].clone();
        prior["correlation"] = serde_json::json!("prior-collection");
        prior["request"]["sources"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        prior["request"]["sources"][0]["source_id"] = serde_json::json!("prior-native");
        wire["input"]["portfolio"]["collections"]
            .as_array_mut()
            .unwrap()
            .insert(0, prior);
        let config: EvmPortfolioConfig = serde_json::from_value(wire).unwrap();
        let entry = config.entry_point_id().unwrap();
        let provider = Arc::new(Provider(
            AtomicUsize::new(0),
            Some(if shared {
                (12, Fault::IntegrityBlocked)
            } else {
                (11, Fault::Rejected)
            }),
        ));
        let installed = resources("expected", provider.clone());
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let id = RunId::from_digest(DigestBytes::from_array(
            [if enrichment { 11 } else { 12 }; 32],
        ));
        let view = if enrichment {
            let input = admit_enrichment(&config, None).unwrap();
            let program = compile(
                entry.clone(),
                &mfm_portfolio::PortfolioEnrichmentOperation::default(),
                &input,
                &installed,
                ProgramLimits::new(0),
            )
            .unwrap();
            runtime.start(id.clone(), &program, &input).await.unwrap()
        } else {
            let input = admit_snapshot(&config, None).unwrap();
            let program = compile(
                entry.clone(),
                &mfm_portfolio::PortfolioSnapshotOperation::default(),
                &input,
                &installed,
                ProgramLimits::new(0),
            )
            .unwrap();
            runtime.start(id.clone(), &program, &input).await.unwrap()
        };
        drop(config);
        let hot = serde_json::to_value(mfm_app::SerializableRunView::new(&view).unwrap()).unwrap();
        assert_eq!(
            hot["state"]["product_failure"],
            serde_json::json!({"kind":"collection_failed", "value":{"ordinal":1,"code":if shared { "integrity_blocked" } else { "observation_unavailable" }}})
        );
        let report = view.failure().unwrap();
        if shared {
            report
                .failure()
                .original()
                .decode::<mfm_chain::balance::ObserveBalanceFailure>()
                .unwrap();
            let mfm_runtime::Failure::Domain {
                call:
                    mfm_runtime::StateCall::Read {
                        call,
                        intent,
                        evidence,
                    },
                ..
            } = report.failure()
            else {
                panic!("retain the complete failed observation")
            };
            let prepared = call.input().decode::<mfm_chain::balance::PreparedBalance<mfm_portfolio::PortfolioContinuation>>().unwrap();
            let context = prepared.context();
            let intent = intent
                .decode::<mfm_chain::balance::ReadBalanceAt>()
                .unwrap();
            assert_eq!(intent.route_ref(), context.metadata().route_ref());
            assert_eq!(intent.target(), context.active_source().unwrap().target());
            assert!(matches!(
                evidence.decode::<EvmReadEvidence>().unwrap(),
                EvmReadEvidence::IntegrityBlocked { .. }
            ));
        } else {
            report
                .failure()
                .original()
                .decode::<EvmBalanceFailure>()
                .unwrap();
        }
        let wrong = if enrichment {
            snapshot_failure(
                report.declaration(),
                report.failure().call().input(),
                report.failure().original(),
            )
        } else {
            enrichment_failure(
                report.declaration(),
                report.failure().call().input(),
                report.failure().original(),
            )
        };
        assert!(wrong.is_err());
        let project = if enrichment {
            enrichment_failure
        } else {
            snapshot_failure
        };
        let input = report.failure().call().input();
        let retained: serde_json::Value = serde_json::from_slice(input.canonical_bytes()).unwrap();
        let context = if shared {
            &retained["context"]
        } else {
            &retained["checked"]["context"]
        };
        assert_eq!(context["completed"].as_array().unwrap().len(), 1);
        let caller = if enrichment {
            &context["caller"]["progress"]
        } else {
            &context["caller"]
        };
        assert_eq!(caller["completed_collections"].as_array().unwrap().len(), 1);
        for field in ["ordinal", "correlation", "route", "request", "source"] {
            let mut forged = retained.clone();
            let context = if shared {
                &mut forged["context"]
            } else {
                &mut forged["checked"]["context"]
            };
            match field {
                "ordinal" => context["metadata"]["collection_ordinal"] = serde_json::json!(0),
                "correlation" => {
                    context["metadata"]["correlation"] = serde_json::json!("different")
                }
                "route" => {
                    context["metadata"]["route_ref"] = serde_json::to_value(
                        Object::from_value(&EvmU256::from_u64(9))
                            .unwrap()
                            .value_ref(),
                    )
                    .unwrap()
                }
                "request" => context["request"]["decimals"] = serde_json::json!(1),
                "source" => {
                    context["request"]["sources"][1]["source_id"] = serde_json::json!("substituted")
                }
                _ => unreachable!(),
            }
            let canonical =
                mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&forged.to_string()).unwrap();
            let object = Object::from_canonical(
                ContentRef::new(
                    input.value_ref().schema_id().clone(),
                    mfm_canonical::raw_content_digest(canonical.as_bytes()),
                )
                .unwrap(),
                canonical.as_bytes(),
            )
            .unwrap();
            assert!(
                project(report.declaration(), &object, report.failure().original()).is_err(),
                "forged {field}"
            );
        }
        let foreign = Object::from_value(&mfm_program::NoParams).unwrap();
        assert!(project(report.declaration(), &foreign, report.failure().original()).is_err());
        assert!(project(report.declaration(), input, &foreign).is_err());
        let document = runtime.program_document(&id).await.unwrap();
        let program = load(
            document.canonical_bytes(),
            &resources(
                "expected",
                Arc::new(ControlledProvider {
                    provider: provider.clone(),
                    mode: ReadMode::Forbidden,
                }),
            ),
        )
        .unwrap();
        // A valid declaration for another retained State cannot authorize this native input/original.
        assert!(project(
            &program.declarations()[0],
            input,
            report.failure().original()
        )
        .is_err());
        let cold_runtime = Runtime::new(store);
        let cold_view = cold_runtime.read(&id, &program).await.unwrap();
        let resumed = cold_runtime.resume(&id, &program).await.unwrap();
        assert_eq!(
            resumed.failure().unwrap().canonical_bytes(),
            report.canonical_bytes()
        );
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            if shared { 12 } else { 11 }
        );
        assert_eq!(cold_view.head_digest(), view.head_digest());
        assert_eq!(
            serde_json::to_value(mfm_app::SerializableRunView::new(&cold_view).unwrap()).unwrap(),
            hot
        );
        assert_eq!(
            cold_view.failure().unwrap().canonical_bytes(),
            report.canonical_bytes()
        );
    }
}

// Arithmetic remains an exact native/shared original; only the checked client projects a product
// code. Portfolio's cross-collection decimal alignment can separately fail at its own Pure State.
#[tokio::test]
async fn arithmetic_and_product_failures_project_exact_originals_after_cold_loading() {
    use mfm_chain::balance::{BalanceArithmetic, BalanceCollectionFailure};
    for scale_overflow in [true, false] {
        let mut wire = serde_json::to_value(configuration(false)).unwrap();
        let collections = wire["input"]["portfolio"]["collections"]
            .as_array_mut()
            .unwrap();
        let mut prior = collections[0].clone();
        prior["correlation"] = serde_json::json!("prior-collection");
        prior["request"]["sources"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        prior["request"]["sources"][0]["source_id"] = serde_json::json!("prior-native");
        prior["request"]["decimals"] = serde_json::json!(18);
        collections[0]["request"]["decimals"] =
            serde_json::json!(if scale_overflow { 3 } else { 0 });
        collections.insert(0, prior);
        let config: EvmPortfolioConfig = serde_json::from_value(wire).unwrap();
        let provider = Arc::new(Provider(
            AtomicUsize::new(0),
            Some((0, Fault::MaximumTokenBalance)),
        ));
        let installed = resources("expected", provider.clone());
        let input = admit_snapshot(&config, None).unwrap();
        let program = compile(
            config.entry_point_id().unwrap(),
            &mfm_portfolio::PortfolioSnapshotOperation::default(),
            &input,
            &installed,
            ProgramLimits::new(0),
        )
        .unwrap();
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(config);
        drop(installed);
        assert_eq!(provider.0.load(Ordering::SeqCst), 0);
        let program = load(&bytes, &resources("expected", provider.clone())).unwrap();
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([70 + u8::from(scale_overflow); 32]));
        let view = runtime.start(run.clone(), &program, &input).await.unwrap();
        let report = view.failure().expect("actual arithmetic domain failure");
        let original = report.failure().original();
        if scale_overflow {
            let EvmBalanceFailure::Collection {
                source:
                    BalanceCollectionFailure::DecimalCapacityExceeded {
                        operation, size, ..
                    },
            } = original.decode().unwrap()
            else {
                panic!("native confirmation retains its arithmetic original")
            };
            assert_eq!(operation, BalanceArithmetic::Scale);
            assert_eq!((size.actual(), size.limit()), (81, 80));
        } else {
            assert!(matches!(
                original
                    .decode::<mfm_portfolio::PortfolioConsolidationFailure>()
                    .unwrap(),
                mfm_portfolio::PortfolioConsolidationFailure::AggregateCapacityExceeded
            ));
        }
        let hot = serde_json::to_value(mfm_app::SerializableRunView::new(&view).unwrap()).unwrap();
        assert_eq!(
            hot["state"]["product_failure"],
            if !scale_overflow {
                serde_json::json!({"kind":"consolidation_failed"})
            } else {
                serde_json::json!({"kind":"collection_failed","value":{"ordinal":1,"code":"observation_unavailable"}})
            }
        );
        drop(program);
        drop(runtime);
        let cold = load(&bytes, &resources("expected", provider.clone())).unwrap();
        let inspected = Runtime::new(store).read(&run, &cold).await.unwrap();
        assert_eq!(inspected.head_digest(), view.head_digest());
        assert_eq!(
            inspected.failure().unwrap().canonical_bytes(),
            report.canonical_bytes()
        );
        assert_eq!(
            serde_json::to_value(mfm_app::SerializableRunView::new(&inspected).unwrap()).unwrap(),
            hot
        );
        assert_eq!(provider.0.load(Ordering::SeqCst), 13);
    }
}

mod projection;
