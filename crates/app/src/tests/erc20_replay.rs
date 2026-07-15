use super::*;

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, B256};
use mfm_adapters_evm::{
    register_evm_collectors_runners, EvmBoundProvider, EvmProviderFactory, EvmRunnerCapabilities,
};
use mfm_certify::CertificationRegistry;
use mfm_evm_capabilities::{
    evm_diagnostic, EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBalanceReadResponse,
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector,
    EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
    EvmCapabilityFuture, EvmNetworkBinding, EvmSourcePolicyId, EvmSourceRef,
    RedactedEvmSourceEvidence,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_portfolio_model::{
    ids::NormalizedEvmAddress,
    portfolio::{NetworkConfig, NetworkFamilyConfig},
};
use mfm_program::{
    build_root_with_registries, NoContext, PublicOutputKey, RootBuilder, ScopeKey, StateKey,
    StateRegistryBuilder,
};
use mfm_program_derive::PublicOutputs;
use mfm_runtime::{CapabilityImplementationId, ErasedRunnerRegistry};
use mfm_states_evm::{
    EvmAddressErc20BalanceSnapshotFact, ObserveErc20BalanceConfig, ObserveErc20BalanceInputHandles,
    ObserveErc20BalanceState, ObserveErc20TokenMetadataConfig,
    ObserveErc20TokenMetadataInputHandles, ObserveErc20TokenMetadataState,
    RecordErc20BalanceFactConfig, RecordErc20BalanceFactInputHandles, RecordErc20BalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInputHandles, ResolveEvmJointTipState,
    EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS, EVM_ERC20_METADATA_OBSERVE_SOURCE_READS,
    EVM_JOINT_TIP_SOURCE_READS,
};
use mfm_store::v1::{self as store, StoreScopeStore as _};

const ANCHOR_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";
const ACCOUNT: &str = "0x0000000000000000000000000000000000000011";

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.erc20_replay_public_outputs")]
struct Erc20ReplayPublicOutputs<'program, 'scope> {
    fact: mfm_program::Handle<'program, 'scope, EvmAddressErc20BalanceSnapshotFact>,
}

#[tokio::test]
async fn erc20_collector_replay_uses_retained_evidence_and_rejects_tampering() {
    let store = store::AsyncInMemoryRunStore::default();
    let certification = erc20_replay_certification_registry();
    let launch_services = make_run_services(
        erc20_replay_runners(&store),
        store.clone(),
        store.clone(),
        certification.clone(),
    );
    let request = crate::prepare_typed_program_run_launch_for_test(
        erc20_replay_draft(),
        BTreeMap::new(),
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("ERC-20 replay launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch ERC-20 observation graph");
    let (_, launched, _) = launch.into_response_parts();
    assert_eq!(
        launched.expect("completed ERC-20 launch").run_mode,
        RunModeStatus::Completed
    );
    drop(launch_services);

    // This service has neither a runner registry nor an EVM provider factory: replay can only
    // use the certified run stream and retained artifacts produced above.
    let replay_services =
        make_run_read_services(store.clone(), store.clone(), certification.clone());
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("replay ERC-20 metadata, balance, and fact from retained evidence");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);

    let tampered_replay_services = make_run_read_services(
        store.clone(),
        TamperedErc20CallEvidenceProvider::new(store),
        certification,
    );
    let error = tampered_replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect_err("replay must reject tampered retained ERC-20 call evidence");
    assert_eq!(error.code, "ArtifactEvidenceMismatch");
}

fn erc20_replay_draft() -> mfm_program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ResolveEvmJointTipState>()
        .expect("register joint-tip state");
    states
        .register::<ObserveErc20TokenMetadataState>()
        .expect("register ERC-20 metadata state");
    states
        .register::<ObserveErc20BalanceState>()
        .expect("register ERC-20 balance state");
    states
        .register::<RecordErc20BalanceFactState>()
        .expect("register ERC-20 fact state");

    build_root_with_registries(
        ScopeKey::new("erc20-replay").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let joint_tip = root.scope().state::<ResolveEvmJointTipState, _>(
                StateKey::new("resolve-joint-tip")?,
                NoContext,
                ResolveEvmJointTipConfig {
                    network: "ethereum-mainnet".to_owned(),
                    chain_id: 1,
                    max_source_reads: NonZeroU64::new(EVM_JOINT_TIP_SOURCE_READS)
                        .expect("joint-tip source-read policy"),
                },
                ResolveEvmJointTipInputHandles {},
            )?;
            let metadata = root.scope().state::<ObserveErc20TokenMetadataState, _>(
                StateKey::new("observe-token-metadata")?,
                NoContext,
                ObserveErc20TokenMetadataConfig {
                    network: erc20_network(),
                    contract_address: normalized_address(TOKEN),
                    max_source_reads: NonZeroU64::new(EVM_ERC20_METADATA_OBSERVE_SOURCE_READS)
                        .expect("metadata source-read policy"),
                },
                ObserveErc20TokenMetadataInputHandles {
                    joint_tip: joint_tip.clone(),
                },
            )?;
            let balance = root.scope().state::<ObserveErc20BalanceState, _>(
                StateKey::new("observe-token-balance")?,
                NoContext,
                ObserveErc20BalanceConfig {
                    network: erc20_network(),
                    contract_address: normalized_address(TOKEN),
                    account: normalized_address(ACCOUNT),
                    max_source_reads: NonZeroU64::new(EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS)
                        .expect("balance source-read policy"),
                },
                ObserveErc20BalanceInputHandles {
                    joint_tip,
                    metadata,
                },
            )?;
            let fact = root.scope().state::<RecordErc20BalanceFactState, _>(
                StateKey::new("record-token-balance")?,
                NoContext,
                RecordErc20BalanceFactConfig {},
                RecordErc20BalanceFactInputHandles {
                    observation: balance,
                },
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &Erc20ReplayPublicOutputs { fact },
            )
        },
    )
    .expect("ERC-20 replay draft")
}

fn erc20_replay_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<ResolveEvmJointTipState>()
        .expect("certify joint-tip state");
    registry
        .register_state::<ObserveErc20TokenMetadataState>()
        .expect("certify ERC-20 metadata state");
    registry
        .register_state::<ObserveErc20BalanceState>()
        .expect("certify ERC-20 balance state");
    registry
        .register_state::<RecordErc20BalanceFactState>()
        .expect("certify ERC-20 fact state");
    registry
        .register_fact_type::<EvmAddressErc20BalanceSnapshotFact>()
        .expect("certify ERC-20 fact descriptor");
    registry
}

fn erc20_replay_runners(store: &store::AsyncInMemoryRunStore) -> ErasedRunnerRegistry {
    let mut runners = ErasedRunnerRegistry::new();
    runners
        .register_capability_spec::<FactRecordCapability>(
            CapabilityImplementationId::new(MANAGED_FACT_RECORD_CAPABILITY_IMPLEMENTATION_ID)
                .expect("managed fact implementation id"),
        )
        .expect("register managed fact capability");
    register_evm_collectors_runners(
        &mut runners,
        EvmRunnerCapabilities::new(
            Arc::new(store.clone()),
            Arc::new(Erc20ReplayProviderFactory),
        ),
    )
    .expect("register ERC-20 collector runners");
    runners
}

fn erc20_network() -> NetworkConfig {
    NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        Some(18),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("semantic EVM network")
}

fn normalized_address(value: &str) -> NormalizedEvmAddress {
    NormalizedEvmAddress::new(value, "test_address").expect("normalized EVM address")
}

fn anchor_hash() -> B256 {
    B256::from_str(ANCHOR_HASH.strip_prefix("0x").expect("hash prefix")).expect("anchor hash")
}

fn token_address() -> Address {
    Address::from_str(TOKEN).expect("token address")
}

fn provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

#[derive(Clone)]
struct Erc20ReplayProviderFactory;

impl EvmProviderFactory for Erc20ReplayProviderFactory {
    fn validate_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        if binding.network_id().as_str() == "ethereum-mainnet" && binding.expected_chain_id() == 1 {
            Ok(())
        } else {
            Err(provider_failure())
        }
    }

    fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmBoundProvider>> {
        self.validate_network_binding(&binding)?;
        Ok(Arc::new(Erc20ReplayProvider { binding }))
    }
}

struct Erc20ReplayProvider {
    binding: EvmNetworkBinding,
}

impl Erc20ReplayProvider {
    fn source_evidence(&self) -> RedactedEvmSourceEvidence {
        RedactedEvmSourceEvidence::from_binding(
            &self.binding,
            1,
            EvmSourceRef::new("primary").expect("source ref"),
            EvmSourcePolicyId::new("default").expect("source policy"),
        )
        .expect("redacted source evidence")
    }

    fn block_response(&self) -> EvmBlockReadResponse {
        EvmBlockReadResponse {
            evidence: self.source_evidence(),
            block_number: 100,
            block_hash: anchor_hash(),
        }
    }
}

impl EvmBlockReadProvider for Erc20ReplayProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        let result = match request.block() {
            EvmBlockSelector::Latest => Ok(self.block_response()),
            EvmBlockSelector::Hash(hash) if *hash == anchor_hash() => Ok(self.block_response()),
            _ => Err(provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }
}

impl EvmBalanceReadProvider for Erc20ReplayProvider {
    fn read_balance<'a>(
        &'a self,
        _request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBalanceReadResponse> {
        Box::pin(std::future::ready(Err(provider_failure())))
    }
}

impl EvmCallReadProvider for Erc20ReplayProvider {
    fn read_call<'a>(
        &'a self,
        request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        let result = if request.to() != token_address()
            || !matches!(request.block(), EvmBlockSelector::Hash(hash) if *hash == anchor_hash())
        {
            Err(provider_failure())
        } else if request.calldata() == [0x31, 0x3c, 0xe5, 0x67] {
            let mut return_data = vec![0; 32];
            return_data[31] = 18;
            Ok(EvmCallReadResponse {
                evidence: self.source_evidence(),
                return_data,
            })
        } else if request.calldata() == erc20_balance_calldata() {
            let mut return_data = vec![0; 32];
            return_data[31] = 42;
            Ok(EvmCallReadResponse {
                evidence: self.source_evidence(),
                return_data,
            })
        } else {
            Err(provider_failure())
        };
        Box::pin(std::future::ready(result))
    }
}

fn erc20_balance_calldata() -> Vec<u8> {
    let mut calldata = vec![0x70, 0xa0, 0x82, 0x31];
    calldata.extend_from_slice(&[0; 31]);
    calldata.push(0x11);
    calldata
}

#[derive(Clone)]
struct TamperedErc20CallEvidenceProvider {
    inner: store::AsyncInMemoryRunStore,
}

impl TamperedErc20CallEvidenceProvider {
    fn new(inner: store::AsyncInMemoryRunStore) -> Self {
        Self { inner }
    }
}

impl store::RetainedArtifactReadProvider for TamperedErc20CallEvidenceProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let artifact = self.inner.read_retained_artifact(requirement).await?;
            let is_erc20_call_evidence = artifact.evidence().artifact_role
                == mfm_events::v1::ArtifactRole::ExternalReadEvidence
                && serde_json::from_slice::<serde_json::Value>(artifact.bytes())
                    .ok()
                    .is_some_and(|value| value.get("destination").is_some());
            if !is_erc20_call_evidence {
                return Ok(artifact);
            }

            let mut tampered_bytes = artifact.bytes().to_vec();
            tampered_bytes.push(b'\n');
            store::VerifiedRunArtifactBytes::new(
                tampered_bytes,
                artifact.evidence().clone(),
                requirement,
            )
        })
    }
}
