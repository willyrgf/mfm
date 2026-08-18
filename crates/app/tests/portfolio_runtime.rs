use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_app::{Application, ApplicationError};
use mfm_evm::{
    CheckChainIdentity, ConfirmBalanceAnchor, ConsolidateBalanceCollection, EvmAnchorRead,
    EvmBalanceRead, EvmChainIdentityRead, EvmPhysicalTarget, EvmReadValue, ReadInitialAnchor,
    ReadNativeBalance, ReadTokenBalance, ReadTokenDecimals, SelectBalanceAsset,
    EVM_BALANCE_SOURCE_LIMIT,
};
use mfm_evm_live::{register_evm_reads, EvmProvider, EvmProviderResponse};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId, StableId};
use mfm_portfolio::{
    plan_snapshot, ConsolidatePortfolio, EnterPortfolioCollection, InitializePortfolio,
    MapEvmBalanceFailure, PortfolioConfig, PortfolioContinuation, PortfolioSnapshotInput,
    PortfolioSnapshotSelector, ResumePortfolioCollection,
};
use mfm_runtime::{ReadAdapterError, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError};
use mfm_store::MemoryStore;

const ANCHOR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Provider {
    calls: AtomicUsize,
    reject_call: Option<usize>,
}

impl EvmProvider for Provider {
    fn request<'a>(
        &'a self,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvmProviderResponse, ReadAdapterError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.reject_call == Some(call) {
                return Ok(EvmProviderResponse::Rejected);
            }
            let request: serde_json::Value =
                serde_json::from_slice(&request_bytes).map_err(|_| ReadAdapterError::Internal)?;
            if request.get("operation").and_then(serde_json::Value::as_str)
                != Some(operation.as_str())
            {
                return Err(ReadAdapterError::Internal);
            }
            let value = match operation.as_str() {
                "mfm.evm.read-chain-identity@1" => EvmReadValue::ChainId(1),
                "mfm.evm.read-initial-anchor@1" | "mfm.evm.confirm-balance-anchor@1" => {
                    EvmReadValue::Anchor {
                        number: "100".to_owned(),
                        hash: ANCHOR.to_owned(),
                    }
                }
                "mfm.evm.read-native-balance@1" => {
                    EvmReadValue::RawUnits("1000000000000000000".to_owned())
                }
                "mfm.evm.read-token-decimals@1" => EvmReadValue::TokenDecimals(6),
                "mfm.evm.read-token-balance@1" => EvmReadValue::RawUnits("2500000".to_owned()),
                _ => return Err(ReadAdapterError::Internal),
            };
            Ok(EvmProviderResponse::Read(value))
        })
    }
}

fn endpoint_ref() -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32])),
    )
    .expect("content ref")
}

fn run_id() -> RunId {
    run_id_with(3)
}

fn run_id_with(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn assembly(
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmProvider>,
) -> mfm_runtime::RuntimeAssembly {
    let mut builder = state_builder();
    register_evm_reads(&mut builder, target, provider).expect("adapters");
    builder.finish().expect("assembly")
}

fn state_builder() -> RuntimeAssemblyBuilder {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<InitializePortfolio>()
        .expect("initialize");
    builder
        .register_pure::<EnterPortfolioCollection>()
        .expect("enter");
    builder
        .register_pure::<ResumePortfolioCollection>()
        .expect("resume");
    builder
        .register_pure::<MapEvmBalanceFailure>()
        .expect("failure mapper");
    builder
        .register_pure::<ConsolidatePortfolio>()
        .expect("consolidate");
    builder
        .register_read::<CheckChainIdentity<PortfolioContinuation>, EvmChainIdentityRead>()
        .expect("chain read");
    builder
        .register_read::<ReadInitialAnchor<PortfolioContinuation>, EvmAnchorRead>()
        .expect("initial anchor");
    builder
        .register_pure::<SelectBalanceAsset<PortfolioContinuation>>()
        .expect("asset selector");
    builder
        .register_read::<ReadNativeBalance<PortfolioContinuation>, EvmBalanceRead>()
        .expect("native balance");
    builder
        .register_read::<ReadTokenDecimals<PortfolioContinuation>, EvmBalanceRead>()
        .expect("token decimals");
    builder
        .register_read::<ReadTokenBalance<PortfolioContinuation>, EvmBalanceRead>()
        .expect("token balance");
    builder
        .register_read::<ConfirmBalanceAnchor<PortfolioContinuation>, EvmAnchorRead>()
        .expect("confirm anchor");
    builder
        .register_pure::<ConsolidateBalanceCollection<PortfolioContinuation>>()
        .expect("collection consolidate");
    builder
}

#[tokio::test]
async fn portfolio_native_run_is_hot_cold_equivalent() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        reject_call: None,
    });
    let store = Arc::new(MemoryStore::new());
    let cold_assembly = assembly(target.clone(), provider.clone());
    let app = Application::new(Runtime::new(
        assembly(target.clone(), provider.clone()),
        store.clone(),
    ));
    let config: PortfolioConfig = serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "native-collection",
            "request": {
                "sources": [{
                    "source_id": "wallet-a.native",
                    "chain_id": 1,
                    "address": "0x1111111111111111111111111111111111111111",
                    "token": null
                }],
                "decimals": 18
            }
        }]
    }))
    .expect("config");
    let run_id = run_id();
    let hot = app
        .start_portfolio(
            run_id.clone(),
            selector("portfolio-example"),
            &config,
            std::slice::from_ref(&target),
        )
        .await
        .expect("start");
    let RunViewState::Succeeded(output) = hot.state() else {
        panic!("portfolio must succeed");
    };
    assert_eq!(
        output.canonical_bytes(),
        include_str!("../../../docs/contracts/evm-portfolio/portfolio-snapshot.json")
            .trim()
            .as_bytes()
    );
    assert_eq!(hot.head_sequence(), 11);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);

    let exact_retry = app
        .start_portfolio(
            run_id.clone(),
            selector("portfolio-example"),
            &config,
            std::slice::from_ref(&target),
        )
        .await
        .expect("exact admission retry");
    assert_eq!(exact_retry.head_digest(), hot.head_digest());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);

    drop(app);
    drop(config);
    drop(target);
    let app = Application::new(Runtime::new(cold_assembly, store));

    let cold = app.read(&run_id).await.expect("cold read");
    assert!(matches!(cold.state(), RunViewState::Succeeded(_)));
    assert_eq!(cold.head_sequence(), hot.head_sequence());
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);

    let resumed = app.resume(&run_id).await.expect("terminal resume");
    assert_eq!(resumed.head_digest(), hot.head_digest());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
}

struct FixedProvider {
    calls: AtomicUsize,
    response: std::result::Result<EvmProviderResponse, ReadAdapterError>,
}

impl EvmProvider for FixedProvider {
    fn request<'a>(
        &'a self,
        _operation: StableId,
        _request_bytes: Vec<u8>,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvmProviderResponse, ReadAdapterError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response.clone()
        })
    }
}

fn native_config() -> PortfolioConfig {
    serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "native-collection",
            "request": {
                "sources": [{
                    "source_id": "wallet-a.native",
                    "chain_id": 1,
                    "address": "0x1111111111111111111111111111111111111111",
                    "token": null
                }],
                "decimals": 18
            }
        }]
    }))
    .expect("config")
}

fn token_config() -> PortfolioConfig {
    serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "token-collection",
            "request": {
                "sources": [{
                    "source_id": "wallet.token",
                    "chain_id": 1,
                    "address": "0x2222222222222222222222222222222222222222",
                    "token": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }],
                "decimals": 18
            }
        }]
    }))
    .expect("token config")
}

fn mixed_config() -> PortfolioConfig {
    serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "mixed-collection",
            "request": {
                "sources": [
                    {
                        "source_id": "wallet.native",
                        "chain_id": 1,
                        "address": "0x1111111111111111111111111111111111111111",
                        "token": null
                    },
                    {
                        "source_id": "wallet.token",
                        "chain_id": 1,
                        "address": "0x2222222222222222222222222222222222222222",
                        "token": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }
                ],
                "decimals": 18
            }
        }]
    }))
    .expect("mixed config")
}

fn selector(target: &str) -> PortfolioSnapshotSelector {
    serde_json::from_value(serde_json::json!({
        "target": target,
        "quote": "usd"
    }))
    .expect("selector")
}

#[tokio::test]
async fn token_and_mixed_source_runs_are_hot_cold_equivalent() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        reject_call: None,
    });
    let app = Application::new(Runtime::new(
        assembly(target.clone(), provider.clone()),
        Arc::new(MemoryStore::new()),
    ));

    for (byte, config, expected_calls) in [
        (51, token_config(), 5_usize),
        (52, mixed_config(), 14_usize),
    ] {
        let id = run_id_with(byte);
        let hot = app
            .start_portfolio(
                id.clone(),
                selector("portfolio-example"),
                &config,
                std::slice::from_ref(&target),
            )
            .await
            .expect("portfolio success");
        let RunViewState::Succeeded(output) = hot.state() else {
            panic!("portfolio must succeed");
        };
        let output = std::str::from_utf8(output.canonical_bytes()).expect("utf8");
        assert!(output.contains("wallet.token"));
        if byte == 52 {
            assert!(output.contains("wallet.native"));
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), expected_calls);
        let cold = app.read(&id).await.expect("cold read");
        assert_eq!(cold.head_digest(), hot.head_digest());
        assert_eq!(provider.calls.load(Ordering::SeqCst), expected_calls);
    }
}

#[tokio::test]
async fn second_source_typed_failure_reaches_the_root_without_a_third_provider_call() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        reject_call: Some(5),
    });
    let app = Application::new(Runtime::new(
        assembly(target.clone(), provider.clone()),
        Arc::new(MemoryStore::new()),
    ));
    let view = app
        .start_portfolio(
            run_id_with(53),
            selector("portfolio-example"),
            &mixed_config(),
            &[target],
        )
        .await
        .expect("durable typed failure");
    let RunViewState::Failed(failure) = view.state() else {
        panic!("second-source rejection must reach root failure");
    };
    assert!(std::str::from_utf8(failure.canonical_bytes())
        .expect("utf8")
        .contains("chain_identity_unavailable"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn missing_or_wrong_live_association_is_rejected_before_store_io() {
    let planned_target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("planned target");
    let config = native_config();
    for (byte, assembly) in [
        (
            54,
            state_builder().finish().expect("missing adapter assembly"),
        ),
        (
            55,
            assembly(
                EvmPhysicalTarget::new(
                    1,
                    ContentRef::new(
                        endpoint_ref().schema_id().clone(),
                        ContentDigest::from_digest(
                            DigestAlgorithm::Sha256V1,
                            DigestBytes::from_array([9; 32]),
                        ),
                    )
                    .expect("other endpoint"),
                )
                .expect("other target"),
                Arc::new(Provider {
                    calls: AtomicUsize::new(0),
                    reject_call: None,
                }),
            ),
        ),
    ] {
        let (program, input) = plan_snapshot(
            selector("portfolio-example"),
            &config,
            std::slice::from_ref(&planned_target),
        )
        .expect("plan");
        let runtime = Runtime::new(assembly, Arc::new(MemoryStore::new()));
        let id = run_id_with(byte);
        assert!(matches!(
            runtime.start(id.clone(), program, input).await,
            Err(RuntimeError::IncompatibleAssembly)
        ));
        assert!(matches!(runtime.read(&id).await, Err(RuntimeError::Absent)));
    }
}

#[tokio::test]
async fn wrong_chain_and_wrong_route_fail_inside_adapter_without_provider_or_conclusion() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    for (byte, mutate) in [(56, "chain"), (57, "route")] {
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            reject_call: None,
        });
        let (program, input) = plan_snapshot(
            selector("portfolio-example"),
            &native_config(),
            std::slice::from_ref(&target),
        )
        .expect("plan");
        let mut input = serde_json::to_value(input).expect("input json");
        if mutate == "chain" {
            input["collections"][0]["request"]["sources"][0]["chain_id"] = serde_json::json!(2);
        } else {
            input["collections"][0]["route_ref"]["content_digest"] = serde_json::json!(
                "content:sha256-v1:9999999999999999999999999999999999999999999999999999999999999999"
            );
        }
        let input: PortfolioSnapshotInput = serde_json::from_value(input).expect("checked input");
        let runtime = Runtime::new(
            assembly(target.clone(), provider.clone()),
            Arc::new(MemoryStore::new()),
        );
        let id = run_id_with(byte);
        assert!(matches!(
            runtime.start(id.clone(), program, input).await,
            Err(RuntimeError::Internal)
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        let view = runtime.read(&id).await.expect("admission only");
        assert_eq!(
            view.head_sequence(),
            3,
            "only the pre-read Pure prefix exists"
        );
        assert!(matches!(view.state(), RunViewState::Runnable));
    }
}

#[tokio::test]
async fn every_authenticated_failure_evidence_reaches_the_exact_root_failure() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    for (byte, response, expected_code) in [
        (
            61,
            EvmProviderResponse::Rejected,
            "chain_identity_unavailable",
        ),
        (
            62,
            EvmProviderResponse::SafeFailure,
            "chain_identity_unavailable",
        ),
        (
            63,
            EvmProviderResponse::IntegrityBlocked,
            "integrity_blocked",
        ),
    ] {
        let provider = Arc::new(FixedProvider {
            calls: AtomicUsize::new(0),
            response: Ok(response),
        });
        let app = Application::new(Runtime::new(
            assembly(target.clone(), provider.clone()),
            Arc::new(MemoryStore::new()),
        ));
        let view = app
            .start_portfolio(
                run_id_with(byte),
                selector("portfolio-example"),
                &native_config(),
                std::slice::from_ref(&target),
            )
            .await
            .expect("durable domain failure");
        let RunViewState::Failed(failure) = view.state() else {
            panic!("evidence must reach the root failure");
        };
        if byte == 61 {
            assert_eq!(
                failure.canonical_bytes(),
                include_str!(
                    "../../../docs/contracts/evm-portfolio/portfolio-snapshot-failure.json"
                )
                .trim()
                .as_bytes()
            );
        }
        assert!(std::str::from_utf8(failure.canonical_bytes())
            .expect("utf8")
            .contains(expected_code));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn application_error_ownership_is_exact() {
    let target = EvmPhysicalTarget::new(1, endpoint_ref()).expect("target");
    let provider = Arc::new(FixedProvider {
        calls: AtomicUsize::new(0),
        response: Err(ReadAdapterError::Unavailable),
    });
    let app = Application::new(Runtime::new(
        assembly(target.clone(), provider.clone()),
        Arc::new(MemoryStore::new()),
    ));

    assert!(matches!(
        app.read(&run_id()).await,
        Err(ApplicationError::Runtime(RuntimeError::Absent))
    ));
    assert!(matches!(
        app.start_portfolio(
            run_id(),
            selector("not-the-configured-portfolio"),
            &native_config(),
            std::slice::from_ref(&target),
        )
        .await,
        Err(ApplicationError::InvalidRequest)
    ));
    assert!(matches!(
        app.start_portfolio(
            run_id(),
            selector("portfolio-example"),
            &native_config(),
            &[],
        )
        .await,
        Err(ApplicationError::Internal)
    ));
    assert!(matches!(
        app.start_portfolio(
            run_id(),
            selector("portfolio-example"),
            &native_config(),
            &[target],
        )
        .await,
        Err(ApplicationError::Runtime(RuntimeError::Unavailable))
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn target_descriptor_bound_precedes_runtime_admission() {
    let targets = (1..=EVM_BALANCE_SOURCE_LIMIT + 1)
        .map(|chain_id| EvmPhysicalTarget::new(chain_id as u64, endpoint_ref()).expect("target"))
        .collect::<Vec<_>>();
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        reject_call: None,
    });
    let app = Application::new(Runtime::new(
        assembly(targets[0].clone(), provider.clone()),
        Arc::new(MemoryStore::new()),
    ));

    app.start_portfolio(
        run_id_with(70),
        selector("portfolio-example"),
        &native_config(),
        &targets[..EVM_BALANCE_SOURCE_LIMIT],
    )
    .await
    .expect("64 target descriptors");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);

    let rejected_id = run_id_with(71);
    assert!(matches!(
        app.start_portfolio(
            rejected_id.clone(),
            selector("portfolio-example"),
            &native_config(),
            &targets,
        )
        .await,
        Err(ApplicationError::Internal)
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);

    app.start_portfolio(
        rejected_id,
        selector("portfolio-example"),
        &native_config(),
        &targets[..EVM_BALANCE_SOURCE_LIMIT],
    )
    .await
    .expect("rejected RunId was never admitted");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 8);
}
