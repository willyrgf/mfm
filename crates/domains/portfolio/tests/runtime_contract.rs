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
            plan_snapshot(selector, &config, std::slice::from_ref(&target)).unwrap();
        let mut builder = portfolio_states();
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

fn portfolio_states() -> RuntimeAssemblyBuilder {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_pure::<InitializePortfolio>().unwrap();
    builder.register_pure::<EnterPortfolioCollection>().unwrap();
    builder
        .register_pure::<ResumePortfolioCollection>()
        .unwrap();
    builder.register_pure::<ConsolidatePortfolio>().unwrap();
    builder
        .register_pure::<ConsolidateBalanceCollection<PortfolioContinuation>>()
        .unwrap();
    builder
        .register_read::<CheckChainIdentity<PortfolioContinuation>, EvmChainIdentityRead>()
        .unwrap();
    builder
        .register_read::<ReadInitialAnchor<PortfolioContinuation>, EvmAnchorRead>()
        .unwrap();
    builder
        .register_read::<ConfirmBalanceAnchor<PortfolioContinuation>, EvmAnchorRead>()
        .unwrap();
    builder
        .register_read::<ReadNativeBalance<PortfolioContinuation>, EvmBalanceRead>()
        .unwrap();
    builder
        .register_read::<ReadTokenDecimals<PortfolioContinuation>, EvmBalanceRead>()
        .unwrap();
    builder
        .register_read::<ReadTokenBalance<PortfolioContinuation>, EvmBalanceRead>()
        .unwrap();
    builder.register_map::<MapEvmBalanceFailure>().unwrap();
    builder
}

#[tokio::test]
async fn maximum_token_portfolio_admits_and_retains_the_first_typed_provider_failure() {
    let portfolio_id = "\"".repeat(256);
    let collections = (0..64)
        .map(|index| {
            serde_json::json!({
                "correlation": format!("{index:02}{}", "\"".repeat(254)),
                "request": {"sources": [{
                    "source_id": format!("{index:02}{}", "\"".repeat(254)),
                    "chain_id": u64::MAX,
                    "address": format!("0x{index:040x}"),
                    "token": format!("0x{:040x}", index + 64)
                }], "decimals": 30}
            })
        })
        .collect::<Vec<_>>();
    let config = serde_json::from_value(serde_json::json!({
        "portfolio_id": portfolio_id, "quotes": ["usd"], "collections": collections
    }))
    .unwrap();
    let selector =
        serde_json::from_value(serde_json::json!({"target": portfolio_id, "quote": "usd"}))
            .unwrap();
    let target = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(u64::MAX).unwrap(),
        endpoint_ref: EvmEndpoint::new("\"".repeat(256))
            .unwrap()
            .endpoint_ref()
            .unwrap(),
    };
    let (program, input) = plan_snapshot(selector, &config, std::slice::from_ref(&target)).unwrap();
    let mut builder = portfolio_states();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    builder
        .register_adapter::<EvmChainIdentityRead, _, _>(target.clone(), move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(AdapterError::Operational(EvmOperationalError::Timeout)) })
        })
        .unwrap();
    builder
        .register_adapter::<EvmAnchorRead, _, _>(target.clone(), |_, _| {
            panic!("terminal identity failure must prevent anchor IO")
        })
        .unwrap();
    builder
        .register_adapter::<EvmBalanceRead, _, _>(target, |_, _| {
            panic!("terminal identity failure must prevent balance IO")
        })
        .unwrap();
    let store = Arc::new(mfm_store::MemoryStore::new());
    let runtime = Runtime::new(builder.finish(), store.clone());
    let run = RunId::from_digest(DigestBytes::from_array([65; 32]));
    let terminal = runtime
        .start(run.clone(), program.clone(), input)
        .await
        .unwrap();
    assert_eq!(terminal.head_sequence(), 4);
    let RunViewState::Failed(report) = terminal.state() else {
        panic!("typed provider failure")
    };
    let FailureCauseView::Adapter(incident) = report.cause() else {
        panic!("original operational incident")
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
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    use mfm_store::Store;
    let history =
        mfm_journal::JournalHistory::qualify(&run, store.load_run(&run).await.unwrap().unwrap())
            .unwrap();
    let genesis = history.frame_lengths().next().unwrap();
    let bound = program
        .history_bound(mfm_program::ConclusionBound::new(genesis as u64).unwrap())
        .unwrap();
    assert_eq!(bound.frames(), 515);
    assert!(bound.bytes() <= mfm_journal::MAX_RUN_BYTES);
    eprintln!(
        "maximum Portfolio admission: genesis {genesis} bytes; {} frames / {} bytes",
        bound.frames(),
        bound.bytes()
    );
    let cold = runtime.read(&run).await.unwrap();
    let RunViewState::Failed(cold_report) = cold.state() else {
        panic!("cold typed provider failure")
    };
    assert_eq!(cold_report.value_ref(), report.value_ref());
    assert_eq!(cold_report.canonical_bytes(), report.canonical_bytes());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
