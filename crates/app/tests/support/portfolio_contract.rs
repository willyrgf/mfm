use mfm_capabilities::AdapterError;
use mfm_evm::*;
use mfm_ids::{DigestBytes, RunId};
use mfm_portfolio::*;
use mfm_runtime::{FailureCauseView, RunViewState, Runtime, RuntimeAssemblyBuilder};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[tokio::test]
async fn planned_native_and_token_collections_execute_and_reconstruct_typed_failures() {
    // Each scenario uses the actual domain planner, State implementations and failure map.
    for (scenario, token, failure) in [(1, false, 0), (2, true, 0), (3, false, 1), (4, true, 2)] {
        let config: PortfolioConfig = serde_json::from_value(serde_json::json!({
            "portfolio_id": "portfolio",
            "quotes": ["usd"],
            "collections": [{
                "correlation": "collection",
                "request": { "sources": [{
                    "source_id": "wallet",
                    "chain_id": 1,
                    "address": format!("0x{:040x}", 1),
                    "token": token.then(|| format!("0x{:040x}", 2))
                }], "decimals": 18 }
            }]
        }))
        .unwrap();
        let selector =
            serde_json::from_value(serde_json::json!({ "target": "portfolio", "quote": "usd" }))
                .unwrap();
        let target = EvmPhysicalTarget {
            chain_id: NonZeroU64::new(1).unwrap(),
            endpoint_ref: EvmEndpoint::new("fixture").unwrap().endpoint_ref().unwrap(),
        };
        let (program, input) =
            plan_snapshot(selector, &config, std::slice::from_ref(&target), None).unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        mfm_app::register_portfolio_states(&mut builder).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        builder
            .register_adapter::<EvmChainIdentityRead, _, _>(
                target.clone(),
                move |reference, intent| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        Ok(EvmReadEvidence::returned(
                            reference.clone(),
                            EvmReadValue::ChainId(intent.chain_id()),
                        ))
                    })
                },
            )
            .unwrap();
        let count = Arc::clone(&calls);
        builder
            .register_adapter::<EvmAnchorRead, _, _>(target.clone(), move |reference, _| {
                count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(EvmReadEvidence::returned(
                        reference.clone(),
                        EvmReadValue::Anchor(EvmBlockAnchor {
                            number: EvmU256::from_u64(7),
                            hash: EvmHash::from_bytes([7; 32]),
                        }),
                    ))
                })
            })
            .unwrap();
        let count = Arc::clone(&calls);
        builder
            .register_adapter::<EvmBalanceRead, _, _>(target, move |reference, intent| {
                count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    match failure {
                        1 => Ok(EvmReadEvidence::rejected(reference.clone())),
                        2 => Err(AdapterError::Operational(EvmOperationalError::Timeout)),
                        _ => {
                            let value =
                                if matches!(intent.subject(), EvmReadSubject::TokenDecimals { .. })
                                {
                                    EvmReadValue::TokenDecimals(EvmTokenDecimals::new(18).unwrap())
                                } else {
                                    EvmReadValue::RawUnits(
                                        EvmU256::new("1000000000000000000").unwrap(),
                                    )
                                };
                            Ok(EvmReadEvidence::returned(reference.clone(), value))
                        }
                    }
                })
            })
            .unwrap();
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(builder.finish(), store);
        let run = RunId::from_digest(DigestBytes::from_array([scenario; 32]));
        let hot = runtime.start(run.clone(), program, input).await.unwrap();
        let observed_calls = calls.load(Ordering::SeqCst);
        match (failure, hot.state()) {
            (0, RunViewState::Succeeded(value)) => {
                let output =
                    serde_json::to_value(value.decode::<PortfolioSnapshotOutput>().unwrap())
                        .unwrap();
                assert_eq!(
                    output["snapshot"]["collections"][0]["holdings"][0]["amount_dec"],
                    "1.000000000000000000"
                );
                assert_eq!(observed_calls, if token { 5 } else { 4 });
            }
            (1, RunViewState::Failed(report)) => {
                let FailureCauseView::Domain { original, root } = report.cause() else {
                    panic!("original and root domain failures")
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
            }
            (2, RunViewState::Failed(report)) => {
                let FailureCauseView::Adapter(incident) = report.cause() else {
                    panic!("typed adapter incident")
                };
                assert!(matches!(
                    incident.error.decode::<EvmOperationalError>().unwrap(),
                    EvmOperationalError::Timeout
                ));
                let context = incident
                    .state_context
                    .decode::<EvmBalanceAdapterContext>()
                    .unwrap();
                assert_eq!(context.collection_ordinal(), 0);
                assert_eq!(context.source_ordinal(), 0);
                assert!(matches!(
                    context.intent().subject(),
                    EvmReadSubject::TokenDecimals { .. }
                ));
            }
            _ => panic!("expected terminal domain result"),
        }
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_digest(), hot.head_digest());
        match (hot.state(), cold.state()) {
            (RunViewState::Succeeded(hot), RunViewState::Succeeded(cold)) => {
                assert_eq!(
                    hot.decode::<PortfolioSnapshotOutput>().unwrap(),
                    cold.decode::<PortfolioSnapshotOutput>().unwrap()
                );
            }
            (RunViewState::Failed(hot), RunViewState::Failed(cold)) => {
                assert_eq!(hot.value_ref(), cold.value_ref());
                assert_eq!(hot.canonical_bytes(), cold.canonical_bytes());
            }
            _ => panic!("cold reconstruction must preserve terminal meaning"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), observed_calls);
        let resumed = runtime.resume(&run).await.unwrap();
        assert_eq!(resumed.head_digest(), hot.head_digest());
        assert_eq!(calls.load(Ordering::SeqCst), observed_calls);
    }
}

#[tokio::test]
async fn enrichment_retains_native_and_nonzero_candidates_and_never_filters_provider_failure() {
    for fail in [false, true] {
        let candidates = serde_json::json!({
            "portfolio_id": "candidates", "quotes": ["eur", "usd"],
            "collections": [{"correlation": "wallet", "request": {"decimals": 18,
                "sources": [
                    {"source_id": "funded", "chain_id": 1, "address": format!("0x{:040x}", 1), "token": format!("0x{:040x}", 2)},
                    {"source_id": "native", "chain_id": 1, "address": format!("0x{:040x}", 1), "token": null},
                    {"source_id": "empty", "chain_id": 1, "address": format!("0x{:040x}", 1), "token": format!("0x{:040x}", 3)}
                ]}}]
        });
        let config = serde_json::from_value(candidates.clone()).unwrap();
        let selector =
            serde_json::from_value(serde_json::json!({"target": "candidates", "quote": "usd"}))
                .unwrap();
        let target = EvmPhysicalTarget {
            chain_id: NonZeroU64::new(1).unwrap(),
            endpoint_ref: EvmEndpoint::new("enrichment")
                .unwrap()
                .endpoint_ref()
                .unwrap(),
        };
        let (program, input) =
            plan_enrichment(selector, &config, std::slice::from_ref(&target), None).unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        mfm_app::register_portfolio_states(&mut builder).unwrap();
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
            .register_adapter::<EvmAnchorRead, _, _>(target.clone(), |reference, _| {
                Box::pin(async move {
                    Ok(EvmReadEvidence::returned(
                        reference.clone(),
                        EvmReadValue::Anchor(EvmBlockAnchor {
                            number: EvmU256::from_u64(7),
                            hash: EvmHash::from_bytes([7; 32]),
                        }),
                    ))
                })
            })
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        builder
            .register_adapter::<EvmBalanceRead, _, _>(target.clone(), move |reference, intent| {
                count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    let value = match intent.subject() {
                        EvmReadSubject::TokenDecimals { .. } => {
                            EvmReadValue::TokenDecimals(EvmTokenDecimals::new(18).unwrap())
                        }
                        EvmReadSubject::TokenBalance { source, .. }
                            if source.source_id() == "funded" =>
                        {
                            EvmReadValue::RawUnits(EvmU256::from_u64(1))
                        }
                        EvmReadSubject::TokenBalance { .. } if fail => {
                            return Err(AdapterError::Operational(EvmOperationalError::Timeout))
                        }
                        _ => EvmReadValue::RawUnits(EvmU256::from_u64(0)),
                    };
                    Ok(EvmReadEvidence::returned(reference.clone(), value))
                })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
        let id = RunId::from_digest(DigestBytes::from_array([90 + u8::from(fail); 32]));
        let hot = runtime.start(id.clone(), program, input).await.unwrap();
        match hot.state() {
            RunViewState::Succeeded(value) if !fail => {
                let output = value.decode::<PortfolioEnrichmentOutput>().unwrap();
                let mut expected = candidates;
                expected["collections"][0]["request"]["sources"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                assert_eq!(
                    output.bindings().collect::<Vec<_>>(),
                    vec![(target.chain_id, &target.binding_ref().unwrap())]
                );
                let wire = serde_json::to_value(&output).unwrap();
                assert_eq!(wire["collections"][0]["anchor"]["number"], "7");
                assert_eq!(wire["quote"], "usd");
                assert_eq!(
                    serde_json::to_value(output.into_snapshot_config().0).unwrap(),
                    expected
                );
            }
            RunViewState::Failed(report) if fail => {
                let FailureCauseView::Adapter(incident) = report.cause() else {
                    panic!("typed provider cause");
                };
                assert!(matches!(
                    incident.error.decode::<EvmOperationalError>().unwrap(),
                    EvmOperationalError::Timeout
                ));
            }
            _ => panic!("candidate enrichment outcome"),
        }
        let count = calls.load(Ordering::SeqCst);
        let cold = runtime.read(&id).await.unwrap();
        assert_eq!(cold.head_digest(), hot.head_digest());
        assert_eq!(calls.load(Ordering::SeqCst), count);
    }
}
