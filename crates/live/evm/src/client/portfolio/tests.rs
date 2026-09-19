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
    SafeFailure,
    IntegrityBlocked,
    WrongChain,
    ChangedAnchor,
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
                        Fault::SafeFailure => EvmReadEvidence::safe_failure(reference.clone()),
                        Fault::IntegrityBlocked => {
                            EvmReadEvidence::integrity_blocked(reference.clone())
                        }
                        Fault::WrongChain => EvmReadEvidence::returned(
                            reference.clone(),
                            EvmReadValue::ChainId(std::num::NonZeroU64::new(2).unwrap()),
                        ),
                        Fault::ChangedAnchor => EvmReadEvidence::returned(
                            reference.clone(),
                            EvmReadValue::Anchor(EvmBlockAnchor {
                                number: EvmU256::from_u64(10),
                                hash: EvmHash::from_bytes([4; 32]),
                            }),
                        ),
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
fn resources(endpoint: &str, provider: Arc<Provider>) -> PortfolioResources {
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
        let cold = resources("expected", provider.clone());
        let reconstructed = load(&bytes, &cold).unwrap();
        let runtime = Runtime::new(Arc::new(MemoryStore::new()));
        let id = RunId::from_digest(DigestBytes::from_array(
            [if enrichment { 1 } else { 2 }; 32],
        ));
        let result = if enrichment {
            runtime
                .execute(
                    id,
                    &reconstructed,
                    &input.decode::<PortfolioEnrichmentInput>().unwrap(),
                )
                .await
                .unwrap()
        } else {
            runtime
                .execute(
                    id,
                    &reconstructed,
                    &input.decode::<PortfolioSnapshotInput>().unwrap(),
                )
                .await
                .unwrap()
        };
        assert_eq!(provider.0.load(Ordering::SeqCst), 9);
        drop(reconstructed);
        drop(cold);
        drop(runtime);
        if enrichment {
            let output = result
                .success()
                .unwrap()
                .decode::<PortfolioEnrichmentOutput>()
                .unwrap();
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
    for enrichment in [false, true] {
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
        let provider = Arc::new(Provider(AtomicUsize::new(0), Some((11, Fault::Rejected))));
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
            serde_json::json!({"kind":"collection_failed", "value":{"ordinal":1,"code":"observation_unavailable"}})
        );
        let report = view.failure().unwrap();
        assert!(report
            .failure()
            .original()
            .decode::<EvmBalanceFailure>()
            .is_ok());
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
        let context = &retained["checked"]["context"];
        assert_eq!(context["completed"].as_array().unwrap().len(), 1);
        let caller = if enrichment {
            &context["caller"]["progress"]
        } else {
            &context["caller"]
        };
        assert_eq!(caller["completed_collections"].as_array().unwrap().len(), 1);
        for field in ["ordinal", "correlation", "route", "request", "source"] {
            let mut forged = retained.clone();
            let context = &mut forged["checked"]["context"];
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
        let program = load(document.canonical_bytes(), &installed).unwrap();
        // A valid declaration for another retained State cannot authorize this native input/original.
        assert!(project(
            &program.declarations()[0],
            input,
            report.failure().original()
        )
        .is_err());
        let cold_runtime = Runtime::new(store);
        let cold_view = cold_runtime.read(&id, &program).await.unwrap();
        assert_eq!(provider.0.load(Ordering::SeqCst), 11);
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

// Each failure occurs after a complete earlier collection and at least one confirmed source.
// Reconstruct before execution and again before inspection; neither path can consult configuration.
#[tokio::test]
async fn every_observation_stage_projects_its_exact_failure_without_source_configuration() {
    for enrichment in [false, true] {
        let mut wire = serde_json::to_value(configuration(enrichment)).unwrap();
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
        let mut last = collections[0]["request"]["sources"][0].clone();
        last["source_id"] = serde_json::json!("last-native");
        collections[0]["request"]["sources"]
            .as_array_mut()
            .unwrap()
            .push(last);
        collections.insert(0, prior);
        let config: EvmPortfolioConfig = serde_json::from_value(wire).unwrap();
        let unused = Arc::new(Provider(AtomicUsize::new(0), None));
        let installed = resources("expected", unused.clone());
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
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(config);
        drop(installed);
        assert_eq!(unused.0.load(Ordering::SeqCst), 0);
        for (at, fault, expected, shared) in [
            (9, Fault::Rejected, "chain_identity_unavailable", false),
            (10, Fault::Rejected, "observation_unavailable", false),
            (11, Fault::Rejected, "observation_unavailable", false),
            (12, Fault::Rejected, "observation_unavailable", true),
            (13, Fault::Rejected, "observation_unavailable", false),
            (15, Fault::Rejected, "observation_unavailable", false),
            (16, Fault::Rejected, "observation_unavailable", true),
            (12, Fault::SafeFailure, "observation_unavailable", true),
            (12, Fault::IntegrityBlocked, "integrity_blocked", true),
            (9, Fault::WrongChain, "chain_identity_unavailable", false),
            (13, Fault::ChangedAnchor, "anchor_changed", false),
        ] {
            let provider = Arc::new(Provider(AtomicUsize::new(0), Some((at, fault))));
            let installed = resources("expected", provider.clone());
            let program = load(&bytes, &installed).unwrap();
            let store = Arc::new(MemoryStore::new());
            let runtime = Runtime::new(store.clone());
            let run = RunId::from_digest(DigestBytes::from_array([40; 32]));
            let view = if enrichment {
                runtime
                    .start(
                        run.clone(),
                        &program,
                        &input.decode::<PortfolioEnrichmentInput>().unwrap(),
                    )
                    .await
                    .unwrap()
            } else {
                runtime
                    .start(
                        run.clone(),
                        &program,
                        &input.decode::<PortfolioSnapshotInput>().unwrap(),
                    )
                    .await
                    .unwrap()
            };
            let report = view
                .failure()
                .unwrap_or_else(|| panic!("expected failure at {at}: {fault:?}"));
            if shared {
                report
                    .failure()
                    .original()
                    .decode::<mfm_chain::balance::ObserveBalanceFailure>()
                    .unwrap();
            } else {
                report
                    .failure()
                    .original()
                    .decode::<EvmBalanceFailure>()
                    .unwrap();
            }
            let hot =
                serde_json::to_value(mfm_app::SerializableRunView::new(&view).unwrap()).unwrap();
            assert_eq!(
                hot["state"]["product_failure"],
                serde_json::json!({
                    "kind":"collection_failed", "value":{"ordinal":1,"code":expected}
                }),
                "{enrichment} {at} {fault:?}"
            );
            let document = runtime.program_document(&run).await.unwrap();
            drop(program);
            drop(runtime);
            drop(installed);
            let cold = load(
                document.canonical_bytes(),
                &resources("expected", provider.clone()),
            )
            .unwrap();
            let inspected = Runtime::new(store).read(&run, &cold).await.unwrap();
            assert_eq!(inspected.head_digest(), view.head_digest());
            assert_eq!(
                inspected.failure().unwrap().canonical_bytes(),
                report.canonical_bytes()
            );
            assert_eq!(
                serde_json::to_value(mfm_app::SerializableRunView::new(&inspected).unwrap())
                    .unwrap(),
                hot
            );
            assert_eq!(provider.0.load(Ordering::SeqCst), at);
        }
    }
}

// Arithmetic remains an exact native/shared original; only the checked client projects a product
// code. Portfolio's cross-collection decimal alignment can separately fail at its own Pure State.
#[tokio::test]
async fn arithmetic_and_product_failures_project_exact_originals_after_cold_loading() {
    use mfm_chain::balance::{BalanceArithmetic, BalanceCollectionFailure};
    for (enrichment, stage) in [(false, 0), (true, 0), (false, 1), (true, 1), (false, 2)] {
        let mut wire = serde_json::to_value(configuration(enrichment)).unwrap();
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
        let native = collections[0]["request"]["sources"][0].clone();
        let token = collections[0]["request"]["sources"][1].clone();
        collections[0]["request"]["sources"] = serde_json::Value::Array(
            (0..if stage == 1 { 9 } else { 1 })
                .map(|ordinal| {
                    let mut source = token.clone();
                    source["source_id"] = serde_json::json!(format!("token-{ordinal}"));
                    source
                })
                .collect(),
        );
        collections[0]["request"]["sources"]
            .as_array_mut()
            .unwrap()
            .insert(0, native);
        collections[0]["request"]["decimals"] = serde_json::json!(match stage {
            0 => 3,
            1 => 2,
            _ => 0,
        });
        collections.insert(0, prior);
        let config: EvmPortfolioConfig = serde_json::from_value(wire).unwrap();
        let provider = Arc::new(Provider(
            AtomicUsize::new(0),
            Some((0, Fault::MaximumTokenBalance)),
        ));
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
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(config);
        drop(installed);
        assert_eq!(provider.0.load(Ordering::SeqCst), 0);
        let program = load(&bytes, &resources("expected", provider.clone())).unwrap();
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([70 + stage; 32]));
        let view = if enrichment {
            runtime
                .start(
                    run.clone(),
                    &program,
                    &input.decode::<PortfolioEnrichmentInput>().unwrap(),
                )
                .await
                .unwrap()
        } else {
            runtime
                .start(
                    run.clone(),
                    &program,
                    &input.decode::<PortfolioSnapshotInput>().unwrap(),
                )
                .await
                .unwrap()
        };
        let report = view.failure().expect("actual arithmetic domain failure");
        let original = report.failure().original();
        let arithmetic = match stage {
            0 => match original.decode::<EvmBalanceFailure>().unwrap() {
                EvmBalanceFailure::Collection { source } => Some(source),
                _ => panic!("native confirmation arithmetic failure"),
            },
            1 => Some(original.decode::<BalanceCollectionFailure>().unwrap()),
            _ => {
                assert!(matches!(
                    original
                        .decode::<mfm_portfolio::PortfolioSnapshotFailure>()
                        .unwrap(),
                    mfm_portfolio::PortfolioSnapshotFailure::ConsolidationFailed
                ));
                None
            }
        };
        if let Some(BalanceCollectionFailure::DecimalCapacityExceeded {
            operation, size, ..
        }) = arithmetic
        {
            assert_eq!(
                operation,
                if stage == 0 {
                    BalanceArithmetic::Scale
                } else {
                    BalanceArithmetic::Sum
                }
            );
            assert_eq!(size.actual(), 81);
            assert_eq!(size.limit(), 80);
        } else {
            assert_eq!(stage, 2);
        }
        let hot = serde_json::to_value(mfm_app::SerializableRunView::new(&view).unwrap()).unwrap();
        assert_eq!(
            hot["state"]["product_failure"],
            if stage == 2 {
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
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            if stage == 1 { 53 } else { 13 }
        );
    }
}
