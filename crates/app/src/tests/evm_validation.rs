use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_primitives::{address, b256, keccak256, Address, Bytes, B256, U256};
use mfm_adapters_evm::{register_evm_read_runners, EvmReadRunnerCapabilities};
use mfm_certify::CertificationRegistry;
use mfm_evm_capabilities::{
    evm_diagnostic, EvmBlockAnchor, EvmBlockSelector, EvmCall, EvmCapabilityError, EvmCode,
    EvmNetworkBinding, EvmReadSession, EvmSessionEvidence, EvmSessionFuture,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, NoContext, PublicOutputKey, RootBuilder, ScopeKey,
    SeedKey, StateKey, StateRegistryBuilder,
};
use mfm_program_derive::PublicOutputs;
use mfm_runtime::ErasedRunnerRegistry;
use mfm_states_evm::{
    EvmContractCallCheck, EvmContractValidationConfig, EvmContractValidationTarget,
    ValidateEvmContractState, VerifiedEvmContract,
};
use mfm_store::v1::{self as store, StoreScopeStore as _};

const CONTRACT: Address = address!("1111111111111111111111111111111111111111");
const CALLER: Address = address!("2222222222222222222222222222222222222222");
const ANCHOR_HASH: B256 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
const CODE: &[u8] = &[0x60, 0x00, 0x60, 0x01];
const CALLDATA: &[u8] = &[0xde, 0xad];
const RETURN_DATA: &[u8] = &[0x12, 0x34];

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.evm_validation_public_outputs")]
struct EvmValidationPublicOutputs<'program, 'scope> {
    verified: mfm_program::Handle<'program, 'scope, VerifiedEvmContract>,
}

#[tokio::test]
async fn exact_anchor_validation_replays_without_live_evm_authority() {
    let store = store::AsyncInMemoryRunStore::default();
    let live_reads = Arc::new(AtomicUsize::new(0));
    let certification = validation_certification_registry();
    let launch_services = make_run_services(
        validation_runners(&store, Arc::clone(&live_reads)),
        store.clone(),
        store.clone(),
        certification.clone(),
    );
    let (draft, seed_material) = validation_launch_material();
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("validation launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch exact-anchor validation");
    let (_, launched, _) = launch.into_response_parts();
    assert_eq!(
        launched.expect("completed validation").run_mode,
        RunModeStatus::Completed
    );
    assert_eq!(live_reads.load(Ordering::SeqCst), 3);
    drop(launch_services);

    // Read services have no runner registry, session binder, transport, or runtime config.
    let replay_services = make_run_read_services(store.clone(), store, certification);
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only validation replay");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
    assert_eq!(live_reads.load(Ordering::SeqCst), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn contract_validation_validates_its_async_route_before_admission() {
    let store = store::AsyncInMemoryRunStore::default();
    let (draft, seed_material) = validation_launch_material();
    let certification = production_certification_registry().expect("production registry");
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        &certification,
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("validation launch request");
    let run_id = request.run_id.clone();
    let fact_index = crate::ProjectionFactIndexProvider::new(store.clone());
    let runners = production_runner_registry(Arc::new(store.clone()), Arc::new(fact_index), None)
        .expect("production runners without EVM config");
    let services = make_run_services(runners, store.clone(), store.clone(), certification);

    let error = services
        .launch_run(request)
        .await
        .expect_err("missing EVM route rejects validation before admission");

    assert_eq!(error.code, "RuntimeConfigRequired");
    assert_eq!(error.diagnostics.len(), 1);
    assert_eq!(error.diagnostics[0].provider_family().as_str(), "evm");
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .is_empty());
}

fn validation_launch_material() -> (
    mfm_program::TypedProgramDraft,
    BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
) {
    let target = EvmContractValidationTarget::new(
        CONTRACT,
        EvmBlockAnchor::new(U256::from(100), ANCHOR_HASH),
    )
    .expect("validation target");
    let target_seed = CanonicalSeed::from_value(&target).expect("target seed");
    let target_bytes = target_seed.canonical_json().clone();
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ValidateEvmContractState>()
        .expect("register validation state");
    let draft = build_root_with_registries(
        ScopeKey::new("evm-validation").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let target = root.seed(SeedKey::new("target")?, target_seed.clone())?;
            let verified = root.scope().state::<ValidateEvmContractState, _>(
                StateKey::new("validate-contract")?,
                NoContext,
                validation_config(),
                target,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &EvmValidationPublicOutputs { verified },
            )
        },
    )
    .expect("validation draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), target_bytes)]);
    (draft, seed_material)
}

fn validation_config() -> EvmContractValidationConfig {
    EvmContractValidationConfig::new(
        "ethereum-mainnet",
        1,
        keccak256(CODE),
        vec![EvmContractCallCheck::new(
            CALLER,
            CONTRACT,
            U256::from(7),
            CALLDATA,
            U256::from(75_000),
            Vec::new(),
            RETURN_DATA,
        )
        .expect("call check")],
    )
    .expect("validation config")
}

fn validation_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<ValidateEvmContractState>()
        .expect("certify validation state");
    registry
}

fn validation_runners(
    store: &store::AsyncInMemoryRunStore,
    live_reads: Arc<AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut runners = ErasedRunnerRegistry::new();
    register_evm_read_runners(
        &mut runners,
        EvmReadRunnerCapabilities::new(
            Arc::new(store.clone()),
            |binding| Box::pin(async move { validate_binding(&binding) }),
            move |binding| {
                let live_reads = Arc::clone(&live_reads);
                Box::pin(async move {
                    validate_binding(&binding)?;
                    Ok(Arc::new(ValidationSession::new(binding, live_reads))
                        as Arc<dyn EvmReadSession>)
                })
            },
        ),
    )
    .expect("register validation runner");
    runners
}

fn validate_binding(binding: &EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> {
    if binding.network_id().as_str() == "ethereum-mainnet" && binding.expected_chain_id() == 1 {
        Ok(())
    } else {
        Err(provider_failure())
    }
}

fn provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

struct ValidationSession {
    evidence: EvmSessionEvidence,
    live_reads: Arc<AtomicUsize>,
}

impl ValidationSession {
    fn new(binding: EvmNetworkBinding, live_reads: Arc<AtomicUsize>) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("primary").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            live_reads,
        }
    }

    fn record_read(&self) {
        self.live_reads.fetch_add(1, Ordering::SeqCst);
    }
}

impl EvmReadSession for ValidationSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        self.record_read();
        let result = match selector {
            EvmBlockSelector::Number(number) if *number == U256::from(100) => {
                Ok(EvmBlockAnchor::new(U256::from(100), ANCHOR_HASH))
            }
            _ => Err(provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(std::future::ready(Err(provider_failure())))
    }

    fn read_code<'a>(
        &'a self,
        address: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.record_read();
        let result = if address == CONTRACT
            && matches!(block, EvmBlockSelector::ExactHash(hash) if *hash == ANCHOR_HASH)
        {
            Ok(EvmCode {
                bytes: Bytes::copy_from_slice(CODE),
                hash: keccak256(CODE),
            })
        } else {
            Err(provider_failure())
        };
        Box::pin(std::future::ready(result))
    }

    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.record_read();
        let result = if request.from() == CALLER
            && request.to() == CONTRACT
            && request.value() == U256::from(7)
            && request.input().as_ref() == CALLDATA
            && request.gas_limit() == U256::from(75_000)
            && request.access_list().is_empty()
            && matches!(request.block(), EvmBlockSelector::ExactHash(hash) if *hash == ANCHOR_HASH)
        {
            Ok(Bytes::copy_from_slice(RETURN_DATA))
        } else {
            Err(provider_failure())
        };
        Box::pin(std::future::ready(result))
    }
}
