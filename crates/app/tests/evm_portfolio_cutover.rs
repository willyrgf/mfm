use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_app::{application_catalog, AdmitRunRequest, Application, PublicError, RunStatus};
use mfm_canonical::raw_content_digest;
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{
    EvmBalanceBindings, EvmBalanceRequest, EvmBalanceSource, EvmCapability, EvmReadValue, EvmState,
};
use mfm_evm_live::{
    evm_live_adapter_implementation_ref, EvmAdapterBinding, EvmAdapterError, EvmLiveAssembly,
    EvmPhysicalTarget, EvmProvider, EvmProviderResponse,
};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId, StableId,
    StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::RunRecord;
use mfm_portfolio::{
    plan_snapshot, PortfolioConfig, PortfolioContinuation, PortfolioId, PortfolioSnapshotInput,
    PortfolioSnapshotSelector, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_program::{capability_contract_ref, state_implementation_ref, BindingDescriptor, State};
use mfm_replay::PortableRun;
use mfm_runtime::{BoxFuture, Runtime, RuntimeAssemblyBuilder, RuntimeStep, SpawnStep};
use mfm_store::{
    ConfigurationCommitOutcome, ResolvedConfigurationHead, StoreWorkLimits, StructuredStore,
    StructuredStoreIdentity,
};
use mfm_values::ValidatedConfig;

const CHAIN_ID: u64 = 1;
const SENDER: &str = "0x1111111111111111111111111111111111111111";
const ANCHOR_HASH: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct ScriptedProvider {
    calls: Arc<AtomicUsize>,
    operations: Arc<Mutex<Vec<String>>>,
    rejected_operation: Option<String>,
}

impl EvmProvider for ScriptedProvider {
    fn request(
        &self,
        call_id: StableId,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> BoxFuture<std::result::Result<EvmProviderResponse, EvmAdapterError>> {
        let calls = Arc::clone(&self.calls);
        let operations = Arc::clone(&self.operations);
        let rejected_operation = self.rejected_operation.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            operations
                .lock()
                .map_err(|_| EvmAdapterError::Authentication)?
                .push(operation.as_str().to_owned());
            if rejected_operation.as_deref() == Some(operation.as_str()) {
                return Ok(EvmProviderResponse::Rejected {
                    call_id,
                    operation,
                    code: "scripted_rejection".to_owned(),
                });
            }
            let intent: serde_json::Value = serde_json::from_slice(&request_bytes)
                .map_err(|_| EvmAdapterError::Authentication)?;
            let intent_operation = intent
                .get("operation")
                .and_then(serde_json::Value::as_str)
                .ok_or(EvmAdapterError::Authentication)?;
            let value = match intent_operation {
                "mfm.evm.read-chain-identity@1" => EvmReadValue::ChainId(CHAIN_ID),
                "mfm.evm.read-initial-anchor@1" | "mfm.evm.confirm-balance-anchor@1" => {
                    EvmReadValue::Anchor {
                        number: "100".to_owned(),
                        hash: ANCHOR_HASH.to_owned(),
                    }
                }
                "mfm.evm.read-native-balance@1" | "mfm.evm.read-token-balance@1" => {
                    EvmReadValue::RawUnits("1000000000000000000".to_owned())
                }
                "mfm.evm.read-token-decimals@1" => EvmReadValue::TokenDecimals(18),
                _ => return Err(EvmAdapterError::Authentication),
            };
            Ok(EvmProviderResponse::Read {
                call_id,
                operation,
                value,
            })
        })
    }
}

fn test_ref(label: &str) -> ContentRef {
    let schema = SchemaId::new(
        "mfm.evm-portfolio-cutover-test",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .expect("schema");
    ContentRef::new(schema, raw_content_digest(label.as_bytes())).expect("content ref")
}

fn read_binding<S, C>(target: &EvmPhysicalTarget) -> BindingDescriptor
where
    S: State,
    C: AccessCapabilityContract,
{
    BindingDescriptor::new(
        state_implementation_ref::<S>().expect("state ref"),
        Some(capability_contract_ref::<C>().expect("capability ref")),
        Some(evm_live_adapter_implementation_ref().expect("adapter identity")),
        target.content_ref().expect("target ref"),
        None,
        None,
    )
    .expect("read binding")
}

async fn commit_configuration<C: mfm_values::MfmConfig>(
    store: &mfm_store::ConfigurationStore,
    owner: mfm_store::PreparedConfigurationAppend<C>,
) -> mfm_store::ResolvedConfiguration<C> {
    match store.commit(owner).await.expect("configuration commit") {
        ConfigurationCommitOutcome::NewlyCommitted(value)
        | ConfigurationCommitOutcome::Found(value) => value,
        _ => panic!("unexpected configuration outcome"),
    }
}

struct Fixture {
    application: Application,
    runtime: Runtime,
    portfolio_config: PortfolioConfig,
    portfolio_configuration_head: ResolvedConfigurationHead,
    balance_bindings: Vec<EvmBalanceBindings>,
    provider_calls: Arc<AtomicUsize>,
    provider_operations: Arc<Mutex<Vec<String>>>,
}

async fn compose_application(rejected_operation: Option<&str>) -> Fixture {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let provider_operations = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        calls: Arc::clone(&provider_calls),
        operations: Arc::clone(&provider_operations),
        rejected_operation: rejected_operation.map(str::to_owned),
    });
    let identity = StructuredStoreIdentity::new(
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope"),
        StoreEpoch::new(1),
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant"),
    );
    let physical_target = EvmPhysicalTarget {
        chain_id: CHAIN_ID,
        endpoint_ref: test_ref("endpoint"),
    };
    let chain_identity =
        read_binding::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(&physical_target);
    let initial_anchor =
        read_binding::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(&physical_target);
    let native_balance =
        read_binding::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(&physical_target);
    let token_decimals =
        read_binding::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(&physical_target);
    let token_balance =
        read_binding::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(&physical_target);
    let confirm_anchor =
        read_binding::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(&physical_target);
    let balance_bindings = EvmBalanceBindings::new(
        CHAIN_ID,
        [
            chain_identity.clone(),
            initial_anchor.clone(),
            native_balance.clone(),
            token_decimals.clone(),
            token_balance.clone(),
            confirm_anchor.clone(),
        ],
    )
    .expect("balance bindings");

    let catalog = application_catalog().expect("catalog");
    let mut assembly_builder = RuntimeAssemblyBuilder::new(catalog.clone());
    let live = EvmLiveAssembly::install(
        &mut assembly_builder,
        vec![balance_bindings],
        vec![
            EvmAdapterBinding::read(chain_identity, physical_target.clone(), provider.clone())
                .expect("chain adapter"),
            EvmAdapterBinding::read(initial_anchor, physical_target.clone(), provider.clone())
                .expect("anchor adapter"),
            EvmAdapterBinding::read(native_balance, physical_target.clone(), provider.clone())
                .expect("native adapter"),
            EvmAdapterBinding::read(token_decimals, physical_target.clone(), provider.clone())
                .expect("decimals adapter"),
            EvmAdapterBinding::read(token_balance, physical_target.clone(), provider.clone())
                .expect("token adapter"),
            EvmAdapterBinding::read(confirm_anchor, physical_target, provider)
                .expect("confirm adapter"),
        ],
    )
    .expect("live closure");

    let assembly = assembly_builder.finish().expect("finish assembly");
    let store =
        StructuredStore::open_memory(identity, catalog, StoreWorkLimits::default()).expect("store");
    let (history, reader, configuration, audit) = store.split().into_parts();
    let runtime = Runtime::new(assembly, history).expect("runtime");

    let native = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "wallet.native".to_owned(),
            chain_id: CHAIN_ID,
            address: SENDER.to_owned(),
            token: None,
        }],
        18,
    )
    .expect("native request");
    let token = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "wallet.token".to_owned(),
            chain_id: CHAIN_ID,
            address: SENDER.to_owned(),
            token: Some("0x2222222222222222222222222222222222222222".to_owned()),
        }],
        18,
    )
    .expect("token request");
    let portfolio_id = PortfolioId {
        value: "portfolio-example".to_owned(),
    };
    let portfolio_config: PortfolioConfig = serde_json::from_value(serde_json::json!({
        "portfolio_id": &portfolio_id,
        "quotes": ["usd"],
        "collections": [
            {"correlation": "native-collection", "request": native},
            {"correlation": "token-collection", "request": token},
        ],
    }))
    .expect("portfolio config");
    let planned_portfolio_config = portfolio_config.clone();
    let portfolio_owner = configuration
        .initial_write_session::<PortfolioConfig>()
        .prepare_local(
            AppendRequestId::new("mfm.config.cutover-portfolio").expect("request"),
            ValidatedConfig::new(portfolio_config).expect("portfolio config"),
        )
        .expect("portfolio owner");
    let portfolio_configuration = commit_configuration(&configuration, portfolio_owner).await;
    let portfolio_configuration_head = portfolio_configuration.head().clone();
    let test_runtime = runtime.clone();
    let test_balance_bindings = live.planning_bindings().to_vec();
    let application = Application::new(
        reader,
        configuration,
        audit,
        runtime,
        portfolio_configuration,
        test_balance_bindings.clone(),
    )
    .expect("application");

    Fixture {
        application,
        runtime: test_runtime,
        portfolio_config: planned_portfolio_config,
        portfolio_configuration_head,
        balance_bindings: test_balance_bindings,
        provider_calls,
        provider_operations,
    }
}

async fn admit_portfolio(application: &Application) -> RunId {
    application
        .admit_run(
            AdmitRunRequest::new(
                StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).expect("entry"),
                serde_json::json!({"target": "portfolio-example", "quote": "usd"}),
            )
            .expect("portfolio request"),
        )
        .await
        .expect("admit portfolio")
        .run_id
}

async fn drive_to(application: &Application, run_id: RunId, expected: RunStatus) {
    let mut prior_head = 0;
    for _ in 0..64 {
        let response = application.drive(run_id.clone()).await.expect("drive run");
        assert!(response.head_sequence > prior_head);
        prior_head = response.head_sequence;
        if matches!(response.status, RunStatus::Terminal | RunStatus::Failed) {
            assert_eq!(response.status, expected);
            return;
        }
    }
    panic!("run did not terminate within the fixed State graph bound");
}

fn provider_operations(fixture: &Fixture) -> Vec<String> {
    fixture
        .provider_operations
        .lock()
        .expect("provider operations")
        .clone()
}

#[tokio::test]
async fn forged_portfolio_route_never_enters_the_provider() {
    let fixture = compose_application(None).await;
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": "portfolio-example",
        "quote": "usd",
    }))
    .expect("portfolio selector");
    let (input, document, source_refs) = plan_snapshot(
        selector,
        &fixture.portfolio_config,
        &fixture.balance_bindings,
    )
    .expect("portfolio plan")
    .into_parts();
    let mut forged_wire = serde_json::to_value(input).expect("Portfolio C0 JSON");
    forged_wire["collections"][0]["route_ref"] =
        serde_json::to_value(test_ref("forged-route")).expect("forged route JSON");
    let forged_input: PortfolioSnapshotInput =
        serde_json::from_value(forged_wire).expect("strict forged Portfolio C0");
    let catalog = fixture.runtime.catalog();
    let program = catalog.program(document).expect("qualified Program");
    let value = catalog
        .qualify(
            program.document().admitted_context_contract_ref().clone(),
            forged_input,
        )
        .expect("qualified forged C0");
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    )
    .expect("run id");
    let session = match fixture
        .runtime
        .admission(
            run_id,
            program,
            value,
            fixture.portfolio_configuration_head.clone(),
            source_refs,
        )
        .expect("admission")
        .spawn()
        .await
    {
        SpawnStep::Active(session) => session,
        _ => panic!("admission must begin at Portfolio initialization"),
    };
    let session = match session.drive().await {
        RuntimeStep::Advanced(session) => session,
        _ => panic!("Portfolio initialization must advance"),
    };
    let session = match session.drive().await {
        RuntimeStep::Advanced(session) => session,
        _ => panic!("Portfolio collection entry must advance"),
    };
    let session = match session.drive().await {
        RuntimeStep::Advanced(session) => session,
        _ => panic!("route mismatch must conclude the EVM integrity failure"),
    };
    assert!(matches!(session.drive().await, RuntimeStep::Terminal(_)));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert!(provider_operations(&fixture).is_empty());
}

#[tokio::test]
async fn one_live_runtime_drives_native_and_token_portfolio_reads() {
    let fixture = compose_application(None).await;
    let run_id = admit_portfolio(&fixture.application).await;
    drive_to(&fixture.application, run_id.clone(), RunStatus::Terminal).await;
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 9);

    let calls_after_execution = fixture.provider_calls.load(Ordering::SeqCst);
    assert_eq!(
        fixture
            .application
            .read_public_run(run_id.clone())
            .await
            .expect("public view")
            .status,
        RunStatus::Terminal,
    );
    assert!(
        fixture
            .application
            .replay_run(run_id.clone())
            .await
            .expect("replay")
            .terminal
    );
    assert!(!fixture
        .application
        .trace_run(run_id.clone())
        .await
        .expect("trace")
        .records
        .is_empty());
    let export = fixture
        .application
        .export_run(run_id)
        .await
        .expect("export");
    let portable = PortableRun::decode(export.bytes()).expect("strict portable export");
    let admission = portable.frames().first().expect("admission frame");
    let RunRecord::RunAdmitted(admitted) = admission.record() else {
        panic!("first portable frame must be admission");
    };
    assert_eq!(
        admission
            .objects()
            .iter()
            .filter(|object| object.content_ref() == admitted.program_ref())
            .count(),
        1,
    );
    assert_eq!(
        fixture.provider_calls.load(Ordering::SeqCst),
        calls_after_execution
    );
}

#[tokio::test]
async fn retired_submission_entry_point_is_not_registered() {
    let fixture = compose_application(None).await;
    let error = fixture
        .application
        .admit_run(
            AdmitRunRequest::new(
                StableId::new("mfm.evm/submit-transaction@1").expect("retired entry"),
                serde_json::json!({}),
            )
            .expect("request"),
        )
        .await
        .expect_err("retired entry point must be rejected");
    assert_eq!(
        error,
        PublicError::BadRequest {
            code: "EntryPointNotFound",
            message: "The entry point is not registered",
        },
    );
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn balance_failures_stop_later_sources_and_collections() {
    for (operation, expected) in [
        (
            "mfm.evm.read-chain-identity@1",
            &["mfm.evm.read-chain-identity@1"][..],
        ),
        (
            "mfm.evm.read-native-balance@1",
            &[
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-native-balance@1",
            ][..],
        ),
        (
            "mfm.evm.read-token-balance@1",
            &[
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-native-balance@1",
                "mfm.evm.confirm-balance-anchor@1",
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-token-decimals@1",
                "mfm.evm.read-token-balance@1",
            ][..],
        ),
    ] {
        let fixture = compose_application(Some(operation)).await;
        let run_id = admit_portfolio(&fixture.application).await;
        drive_to(&fixture.application, run_id, RunStatus::Failed).await;
        assert_eq!(
            provider_operations(&fixture),
            expected
                .iter()
                .map(|operation| (*operation).to_owned())
                .collect::<Vec<_>>(),
        );
    }
}
