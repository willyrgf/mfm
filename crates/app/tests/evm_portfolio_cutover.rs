use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_app::{application_catalog, AdmitRunRequest, Application, RunStatus};
use mfm_canonical::raw_content_digest;
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{
    EvmBalanceBindings, EvmBalanceRequest, EvmBalanceSource, EvmCapability, EvmConfig,
    EvmReadValue, EvmState, EvmSubmissionBindings, EvmTransactionTarget, NonceReservationEvidence,
};
use mfm_evm_live::{
    evm_live_adapter_implementation_ref, public_signer_key_instance_ref,
    wallet_nonce_effect_domain, EvmAdapterBinding, EvmAdapterError, EvmLiveAssembly,
    EvmPhysicalTarget, EvmProvider, EvmProviderResponse, WalletNonceAuthority,
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
use mfm_signing::{PublicSignerKeyInstance, Signer, SigningFuture, SigningRequest, SigningResult};
use mfm_store::{
    ConfigurationCommitOutcome, ResolvedConfigurationHead, StoreWorkLimits, StructuredStore,
    StructuredStoreIdentity,
};
use mfm_values::ValidatedConfig;

const CHAIN_ID: u64 = 1;
const SENDER: &str = "0x1111111111111111111111111111111111111111";
const NONCE_DOMAIN: &str = "wallet-main";
const ANCHOR_HASH: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct ScriptedProvider {
    calls: Arc<AtomicUsize>,
    broadcast_requests: Arc<Mutex<Vec<serde_json::Value>>>,
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
        let broadcast_requests = Arc::clone(&self.broadcast_requests);
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
            if operation.as_str() == "mfm.evm.broadcast-transaction@1" {
                let request = serde_json::from_slice(&request_bytes)
                    .map_err(|_| EvmAdapterError::Authentication)?;
                broadcast_requests
                    .lock()
                    .map_err(|_| EvmAdapterError::Authentication)?
                    .push(request);
                return Ok(EvmProviderResponse::Broadcast {
                    call_id,
                    operation,
                    transaction_hash: "0xfeed".to_owned(),
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
                "mfm.evm.read-transaction-receipt@1" => {
                    let transaction_hash = intent
                        .pointer("/subject/value/transaction_hash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .ok_or(EvmAdapterError::Authentication)?;
                    EvmReadValue::Receipt {
                        transaction_hash,
                        execution_disposition: "succeeded".to_owned(),
                        inclusion_block_number: "100".to_owned(),
                        inclusion_block_hash: ANCHOR_HASH.to_owned(),
                    }
                }
                "mfm.evm.read-finalized-head@1" => EvmReadValue::FinalizedHead {
                    number: "100".to_owned(),
                },
                "mfm.evm.read-canonical-inclusion-block@1" => EvmReadValue::CanonicalBlock {
                    number: "100".to_owned(),
                    hash: ANCHOR_HASH.to_owned(),
                },
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

struct ScriptedNonceAuthority {
    calls: Arc<AtomicUsize>,
    rejected: bool,
}

impl WalletNonceAuthority for ScriptedNonceAuthority {
    fn reserve(
        self: Arc<Self>,
        _operation_key: StableId,
    ) -> BoxFuture<std::result::Result<NonceReservationEvidence, EvmAdapterError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let rejected = self.rejected;
        Box::pin(async move {
            if rejected {
                Ok(NonceReservationEvidence::Rejected)
            } else {
                Ok(NonceReservationEvidence::Reserved { nonce: 7 })
            }
        })
    }
}

struct ScriptedSigner {
    identity: PublicSignerKeyInstance,
    calls: Arc<AtomicUsize>,
}

impl Signer for ScriptedSigner {
    fn public_identity(&self) -> &PublicSignerKeyInstance {
        &self.identity
    }

    fn sign(&self, request: SigningRequest) -> SigningFuture<SigningResult> {
        let key_instance = self.identity.key_instance_id.clone();
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            if request.purpose.as_str() != "mfm.evm.broadcast-transaction@1" {
                return Err(mfm_signing::SigningError::InvalidRequest);
            }
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(SigningResult {
                key_instance,
                signature: "signature-canary-must-not-persist".to_owned(),
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

fn adapter_ref() -> ContentRef {
    evm_live_adapter_implementation_ref().expect("adapter identity")
}

fn read_binding<S, C>(target: &EvmPhysicalTarget) -> BindingDescriptor
where
    S: State,
    C: AccessCapabilityContract,
{
    BindingDescriptor::new(
        state_implementation_ref::<S>().expect("state ref"),
        Some(capability_contract_ref::<C>().expect("capability ref")),
        Some(adapter_ref()),
        target.content_ref().expect("target ref"),
        None,
        None,
    )
    .expect("read binding")
}

fn effect_binding<S, C>(
    target: &EvmPhysicalTarget,
    effect_domain: StableId,
    signer: Option<ContentRef>,
) -> BindingDescriptor
where
    S: State,
    C: AccessCapabilityContract,
{
    BindingDescriptor::new(
        state_implementation_ref::<S>().expect("state ref"),
        Some(capability_contract_ref::<C>().expect("capability ref")),
        Some(adapter_ref()),
        target.content_ref().expect("target ref"),
        Some(effect_domain),
        signer,
    )
    .expect("effect binding")
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

async fn drive_to_terminal(
    application: &Application,
    label: &str,
    run_id: mfm_ids::RunId,
    provider_calls: &AtomicUsize,
) {
    let mut prior_head = 0;
    for step in 0..64 {
        let response = match application.drive(run_id.clone()).await {
            Ok(response) => response,
            Err(error) => panic!(
                "{label} drive {step} failed: {error:?}; provider_calls={}; trace={:?}",
                provider_calls.load(Ordering::SeqCst),
                application.trace_run(run_id.clone()).await
            ),
        };
        if response.head_sequence <= prior_head {
            panic!(
                "{label} drive {step} made no durable progress: {response:?}; provider_calls={}; trace={:?}",
                provider_calls.load(Ordering::SeqCst),
                application.trace_run(run_id.clone()).await
            );
        }
        prior_head = response.head_sequence;
        if matches!(response.status, RunStatus::Terminal | RunStatus::Failed) {
            if response.status != RunStatus::Terminal {
                panic!(
                    "{label} reached a failed terminal state: trace={:?}",
                    application.trace_run(run_id.clone()).await,
                );
            }
            return;
        }
    }
    panic!("run did not terminate within the fixed State graph bound");
}

async fn drive_to_failure(
    application: &Application,
    label: &str,
    run_id: mfm_ids::RunId,
    provider_calls: &AtomicUsize,
) {
    let mut prior_head = 0;
    for step in 0..64 {
        let response = match application.drive(run_id.clone()).await {
            Ok(response) => response,
            Err(error) => panic!(
                "{label} drive {step} failed: {error:?}; provider_calls={}; trace={:?}",
                provider_calls.load(Ordering::SeqCst),
                application.trace_run(run_id.clone()).await
            ),
        };
        assert!(
            response.head_sequence > prior_head,
            "{label} drive {step} made no durable progress: {response:?}; provider_calls={}; trace={:?}",
            provider_calls.load(Ordering::SeqCst),
            application.trace_run(run_id.clone()).await
        );
        prior_head = response.head_sequence;
        if matches!(response.status, RunStatus::Terminal | RunStatus::Failed) {
            assert_eq!(
                response.status,
                RunStatus::Failed,
                "{label} fabricated a success terminal state: trace={:?}",
                application.trace_run(run_id).await
            );
            return;
        }
    }
    panic!("{label} did not reach its typed failure within the fixed State graph bound");
}

struct Fixture {
    application: Application,
    runtime: Runtime,
    portfolio_config: PortfolioConfig,
    portfolio_configuration_head: ResolvedConfigurationHead,
    balance_bindings: Vec<EvmBalanceBindings>,
    provider_calls: Arc<AtomicUsize>,
    provider_operations: Arc<Mutex<Vec<String>>>,
    nonce_calls: Arc<AtomicUsize>,
    signer_calls: Arc<AtomicUsize>,
    broadcast_requests: Arc<Mutex<Vec<serde_json::Value>>>,
    signer_ref_json: serde_json::Value,
}

async fn compose_application(rejected_operation: Option<&str>, reject_nonce: bool) -> Fixture {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let nonce_calls = Arc::new(AtomicUsize::new(0));
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let broadcast_requests = Arc::new(Mutex::new(Vec::new()));
    let provider_operations = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        calls: Arc::clone(&provider_calls),
        broadcast_requests: Arc::clone(&broadcast_requests),
        operations: Arc::clone(&provider_operations),
        rejected_operation: rejected_operation.map(str::to_owned),
    });
    let nonce_authority = Arc::new(ScriptedNonceAuthority {
        calls: Arc::clone(&nonce_calls),
        rejected: reject_nonce,
    });
    let signer = Arc::new(ScriptedSigner {
        identity: PublicSignerKeyInstance {
            signer_id: StableId::new("mfm.signer.cutover-test").expect("signer"),
            key_instance_id: StableId::new("mfm.key-instance.cutover-test").expect("key"),
            algorithm: StableId::new("mfm.algorithm.cutover-test").expect("algorithm"),
        },
        calls: Arc::clone(&signer_calls),
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
    let transaction_target =
        EvmTransactionTarget::new(CHAIN_ID, SENDER.to_owned(), NONCE_DOMAIN.to_owned())
            .expect("transaction target");
    let signer_ref = public_signer_key_instance_ref(signer.public_identity()).expect("signer ref");
    let signer_ref_json = serde_json::to_value(&signer_ref).expect("signer ref JSON");

    let reserve_nonce = effect_binding::<EvmState<0, 0>, EvmCapability<0>>(
        &physical_target,
        wallet_nonce_effect_domain(identity.tenant(), SENDER, NONCE_DOMAIN)
            .expect("nonce effect domain"),
        None,
    );
    let broadcast = effect_binding::<EvmState<0, 2>, EvmCapability<1>>(
        &physical_target,
        StableId::new("mfm.evm.effect.broadcast.cutover-test").expect("broadcast effect domain"),
        Some(signer_ref.clone()),
    );
    let receipt = read_binding::<EvmState<0, 3>, EvmCapability<3>>(&physical_target);
    let finalized_head = read_binding::<EvmState<0, 4>, EvmCapability<3>>(&physical_target);
    let canonical_block = read_binding::<EvmState<0, 5>, EvmCapability<3>>(&physical_target);
    let submission_bindings = EvmSubmissionBindings::new(
        transaction_target.clone(),
        [
            reserve_nonce.clone(),
            broadcast.clone(),
            receipt.clone(),
            finalized_head.clone(),
            canonical_block.clone(),
        ],
    )
    .expect("submission bindings");

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
        vec![submission_bindings],
        vec![balance_bindings],
        vec![
            EvmAdapterBinding::reserve_nonce(
                reserve_nonce,
                physical_target.clone(),
                identity.tenant().clone(),
                SENDER.to_owned(),
                NONCE_DOMAIN.to_owned(),
                nonce_authority,
            )
            .expect("nonce adapter"),
            EvmAdapterBinding::broadcast(
                broadcast,
                physical_target.clone(),
                SENDER.to_owned(),
                NONCE_DOMAIN.to_owned(),
                provider.clone(),
                signer,
            )
            .expect("broadcast adapter"),
            EvmAdapterBinding::read(receipt, physical_target.clone(), provider.clone())
                .expect("receipt adapter"),
            EvmAdapterBinding::read(finalized_head, physical_target.clone(), provider.clone())
                .expect("head adapter"),
            EvmAdapterBinding::read(canonical_block, physical_target.clone(), provider.clone())
                .expect("canonical adapter"),
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
            EvmAdapterBinding::read(confirm_anchor, physical_target, provider.clone())
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
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": portfolio_id,
        "quote": "usd",
    }))
    .expect("portfolio selector");
    let (planned_input, planned_program, _) =
        plan_snapshot(selector, &portfolio_config, live.planning_bindings().1)
            .expect("portfolio plan")
            .into_parts();
    let planned_contract = planned_program.admitted_context_contract_ref().clone();
    application_catalog()
        .expect("catalog")
        .qualify(planned_contract, planned_input)
        .expect("planned portfolio C0 qualifies");
    let portfolio_owner = configuration
        .initial_write_session::<PortfolioConfig>()
        .prepare_local(
            AppendRequestId::new("mfm.config.cutover-portfolio").expect("request"),
            ValidatedConfig::new(portfolio_config).expect("portfolio config"),
        )
        .expect("portfolio owner");
    let portfolio_configuration = commit_configuration(&configuration, portfolio_owner).await;
    let portfolio_configuration_head = portfolio_configuration.head().clone();
    let evm_config: EvmConfig = serde_json::from_value(serde_json::json!({
        "submission_routes": [{
            "target": transaction_target,
            "public_signer_key_instance_ref": signer_ref,
        }],
    }))
    .expect("EVM config");
    let evm_owner = configuration
        .write_session::<EvmConfig>(portfolio_configuration.head())
        .expect("evm write session")
        .prepare_local(
            AppendRequestId::new("mfm.config.cutover-evm").expect("request"),
            ValidatedConfig::new(evm_config).expect("evm config"),
        )
        .expect("evm owner");
    let evm_configuration = commit_configuration(&configuration, evm_owner).await;
    let (submission_bindings, balance_bindings) = live.planning_bindings();
    let test_runtime = runtime.clone();
    let test_balance_bindings = balance_bindings.to_vec();
    let application = Application::new(
        reader,
        configuration,
        audit,
        runtime,
        portfolio_configuration,
        evm_configuration,
        submission_bindings.to_vec(),
        balance_bindings.to_vec(),
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
        nonce_calls,
        signer_calls,
        broadcast_requests,
        signer_ref_json,
    }
}

#[tokio::test]
async fn forged_portfolio_route_never_enters_the_provider() {
    let fixture = compose_application(None, false).await;
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
    assert_provider_operations(&fixture, &[]);
}

#[tokio::test]
async fn one_live_runtime_drives_submission_and_native_token_portfolio_programs() {
    let fixture = compose_application(None, false).await;
    let application = &fixture.application;
    let provider_calls = &fixture.provider_calls;
    let nonce_calls = &fixture.nonce_calls;
    let signer_calls = &fixture.signer_calls;
    let broadcast_requests = &fixture.broadcast_requests;

    let submission = application
        .admit_run(
            AdmitRunRequest::new(
                StableId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry"),
                serde_json::json!({
                    "target": {
                        "chain_id": CHAIN_ID,
                        "sender": SENDER,
                        "nonce_domain": NONCE_DOMAIN
                    },
                    "idempotency_key": "submission-1",
                    "data": [1, 2, 3],
                    "gas_limit": 21000,
                    "max_fee": "100"
                }),
            )
            .expect("submission request"),
        )
        .await
        .expect("admit submission");
    drive_to_terminal(
        application,
        "submission",
        submission.run_id.clone(),
        provider_calls,
    )
    .await;

    {
        let requests = broadcast_requests.lock().expect("broadcast requests");
        assert_eq!(requests.len(), 1);
        let intent = requests[0]
            .get("intent")
            .expect("committed broadcast intent");
        assert_eq!(
            intent
                .pointer("/target/chain_id")
                .and_then(serde_json::Value::as_u64),
            Some(CHAIN_ID)
        );
        assert_eq!(
            intent
                .pointer("/target/sender")
                .and_then(serde_json::Value::as_str),
            Some(SENDER)
        );
        assert_eq!(
            intent
                .pointer("/target/nonce_domain")
                .and_then(serde_json::Value::as_str),
            Some(NONCE_DOMAIN)
        );
        assert_eq!(
            intent
                .get("idempotency_key")
                .and_then(serde_json::Value::as_str),
            Some("submission-1")
        );
        assert_eq!(
            intent
                .get("candidate_id")
                .and_then(serde_json::Value::as_str),
            Some("mfm.evm.candidate/1/0x1111111111111111111111111111111111111111/wallet-main/7")
        );
        assert_eq!(
            intent.get("nonce").and_then(serde_json::Value::as_u64),
            Some(7)
        );
        assert_eq!(
            intent.get("sender").and_then(serde_json::Value::as_str),
            Some(SENDER)
        );
        assert_eq!(
            intent
                .get("nonce_domain")
                .and_then(serde_json::Value::as_str),
            Some(NONCE_DOMAIN)
        );
        assert_eq!(intent.get("data"), Some(&serde_json::json!([1, 2, 3])));
        assert_eq!(
            intent.get("gas_limit").and_then(serde_json::Value::as_u64),
            Some(21_000)
        );
        assert_eq!(
            intent.get("max_fee").and_then(serde_json::Value::as_str),
            Some("100")
        );
        assert_eq!(
            intent.get("public_signer_key_instance_ref"),
            Some(&fixture.signer_ref_json)
        );
    }

    let portfolio = application
        .admit_run(
            AdmitRunRequest::new(
                StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).expect("entry"),
                serde_json::json!({"target": "portfolio-example", "quote": "usd"}),
            )
            .expect("portfolio request"),
        )
        .await
        .expect("admit portfolio");
    drive_to_terminal(
        application,
        "portfolio",
        portfolio.run_id.clone(),
        provider_calls,
    )
    .await;

    assert_eq!(nonce_calls.load(Ordering::SeqCst), 1);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 13);
    let provider_calls_after_execution = provider_calls.load(Ordering::SeqCst);
    let view = application
        .read_public_run(submission.run_id.clone())
        .await
        .expect("callback-free public view");
    assert_eq!(view.status, RunStatus::Terminal);
    let replay = application
        .replay_run(submission.run_id.clone())
        .await
        .expect("callback-free replay");
    assert!(replay.terminal);
    let trace = application
        .trace_run(submission.run_id.clone())
        .await
        .expect("redacted trace");
    let trace_bytes = serde_json::to_vec(&trace).expect("trace JSON");
    assert!(!trace_bytes
        .windows(b"signature-canary-must-not-persist".len())
        .any(|window| window == b"signature-canary-must-not-persist"));
    let export = application
        .export_run(submission.run_id.clone())
        .await
        .expect("export");
    assert!(!export
        .bytes()
        .windows(b"signature-canary-must-not-persist".len())
        .any(|window| window == b"signature-canary-must-not-persist"));
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
        "portable export must retain the admitted Program exactly once",
    );
    assert_eq!(
        provider_calls.load(Ordering::SeqCst),
        provider_calls_after_execution,
        "callback-free read, replay, trace, and export must not re-enter the provider",
    );
}

async fn admit_submission(application: &Application, idempotency_key: &str) -> mfm_ids::RunId {
    application
        .admit_run(
            AdmitRunRequest::new(
                StableId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry"),
                serde_json::json!({
                    "target": {
                        "chain_id": CHAIN_ID,
                        "sender": SENDER,
                        "nonce_domain": NONCE_DOMAIN
                    },
                    "idempotency_key": idempotency_key,
                    "data": [1, 2, 3],
                    "gas_limit": 21000,
                    "max_fee": "100"
                }),
            )
            .expect("submission request"),
        )
        .await
        .expect("admit submission")
        .run_id
}

async fn admit_portfolio(application: &Application) -> mfm_ids::RunId {
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

fn provider_operations(fixture: &Fixture) -> Vec<String> {
    fixture
        .provider_operations
        .lock()
        .expect("provider operations")
        .clone()
}

fn assert_provider_operations(fixture: &Fixture, expected: &[&str]) {
    assert_eq!(
        provider_operations(fixture),
        expected
            .iter()
            .map(|operation| (*operation).to_owned())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn submission_failures_stop_every_later_provider_operation() {
    let nonce_failure = compose_application(None, true).await;
    let run_id = admit_submission(&nonce_failure.application, "nonce-failure").await;
    drive_to_failure(
        &nonce_failure.application,
        "nonce failure",
        run_id,
        &nonce_failure.provider_calls,
    )
    .await;
    assert_eq!(nonce_failure.nonce_calls.load(Ordering::SeqCst), 1);
    assert_eq!(nonce_failure.signer_calls.load(Ordering::SeqCst), 0);
    assert_provider_operations(&nonce_failure, &[]);

    for (operation, expected) in [
        (
            "mfm.evm.broadcast-transaction@1",
            &["mfm.evm.broadcast-transaction@1"][..],
        ),
        (
            "mfm.evm.read-transaction-receipt@1",
            &[
                "mfm.evm.broadcast-transaction@1",
                "mfm.evm.read-transaction-receipt@1",
            ][..],
        ),
        (
            "mfm.evm.read-finalized-head@1",
            &[
                "mfm.evm.broadcast-transaction@1",
                "mfm.evm.read-transaction-receipt@1",
                "mfm.evm.read-finalized-head@1",
            ][..],
        ),
        (
            "mfm.evm.read-canonical-inclusion-block@1",
            &[
                "mfm.evm.broadcast-transaction@1",
                "mfm.evm.read-transaction-receipt@1",
                "mfm.evm.read-finalized-head@1",
                "mfm.evm.read-canonical-inclusion-block@1",
            ][..],
        ),
    ] {
        let fixture = compose_application(Some(operation), false).await;
        let run_id = admit_submission(&fixture.application, operation).await;
        drive_to_failure(
            &fixture.application,
            operation,
            run_id,
            &fixture.provider_calls,
        )
        .await;
        assert_eq!(fixture.nonce_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 1);
        assert_provider_operations(&fixture, expected);
    }
}

#[tokio::test]
async fn balance_failures_stop_later_sources_and_collections() {
    for (operation, expected) in [
        (
            "mfm.evm.read-chain-identity@1",
            &["mfm.evm.read-chain-identity@1"][..],
        ),
        (
            "mfm.evm.read-initial-anchor@1",
            &[
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
            ][..],
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
            "mfm.evm.confirm-balance-anchor@1",
            &[
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-native-balance@1",
                "mfm.evm.confirm-balance-anchor@1",
            ][..],
        ),
        (
            "mfm.evm.read-token-decimals@1",
            &[
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-native-balance@1",
                "mfm.evm.confirm-balance-anchor@1",
                "mfm.evm.read-chain-identity@1",
                "mfm.evm.read-initial-anchor@1",
                "mfm.evm.read-token-decimals@1",
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
        let fixture = compose_application(Some(operation), false).await;
        let run_id = admit_portfolio(&fixture.application).await;
        drive_to_failure(
            &fixture.application,
            operation,
            run_id,
            &fixture.provider_calls,
        )
        .await;
        assert_eq!(fixture.nonce_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 0);
        assert_provider_operations(&fixture, expected);
    }
}
