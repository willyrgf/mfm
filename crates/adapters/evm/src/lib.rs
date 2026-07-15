#![warn(missing_docs)]
//! EVM collector adapter runners.
//!
//! Binds reusable EVM native-balance and ERC-20 state contracts to block, balance, and generic
//! call-read capabilities. Protocol IO stays in the transport crate; this crate owns request
//! mapping, redacted evidence recording, replay recomputation, runner registration, and managed
//! fact recording.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, B256};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadCapability, EvmBalanceReadProvider, EvmBalanceReadRequest,
    EvmBlockReadCapability, EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockSelector,
    EvmCallReadCapability, EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse,
    EvmCapabilityError, EvmNetworkBinding, EvmNetworkId, EvmSourcePolicyId, EvmSourceRef,
    RedactedEvmSourceEvidence, EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config_for_node, load_materialized_struct_input, load_runner_config_for_node,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_evm::{
    assemble_evm_native_balance_batch, erc20_balance_record_visibility, evm_jsonrpc_adapter_kind,
    evm_jsonrpc_adapter_version, materialize_evm_joint_tip, native_balance_record_visibility,
    normalize_erc20_balance_from_capability, normalize_erc20_token_metadata_from_capability,
    normalize_evm_native_balance_observation, AssembleEvmNativeBalanceBatchConfig,
    AssembleEvmNativeBalanceBatchInput, AssembleEvmNativeBalanceBatchState,
    EvmAddressErc20BalanceObservation, EvmAddressErc20BalanceSnapshotFact,
    EvmAddressNativeBalanceObservation, EvmAddressNativeBalanceSnapshotFact, EvmErc20TokenMetadata,
    EvmJointTip, ObserveErc20BalanceConfig, ObserveErc20BalanceInput, ObserveErc20BalanceState,
    ObserveErc20TokenMetadataConfig, ObserveErc20TokenMetadataInput,
    ObserveErc20TokenMetadataState, ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceState, RecordErc20BalanceFactState, RecordEvmNativeBalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInput, ResolveEvmJointTipState,
    EVM_JOINT_TIP_SOURCE_READS,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue, NonEmpty};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.jsonrpc",
    name = "source_read_evidence",
    version = "1",
    schema = "mfm.evm.jsonrpc.external_read.source"
)]
struct EvmSourceReadEvidence {
    network_id: String,
    expected_chain_id: u64,
    observed_chain_id: u64,
    source_ref: String,
    policy_id: String,
}

impl EvmSourceReadEvidence {
    fn from_capability(evidence: &mfm_evm_capabilities::RedactedEvmSourceEvidence) -> Self {
        Self {
            network_id: evidence.network_id.as_str().to_owned(),
            expected_chain_id: evidence.expected_chain_id,
            observed_chain_id: evidence.observed_chain_id,
            source_ref: evidence.source_ref.as_str().to_owned(),
            policy_id: evidence.policy_id.as_str().to_owned(),
        }
    }

    fn to_capability(&self) -> Result<RedactedEvmSourceEvidence> {
        let network_id = EvmNetworkId::new(&self.network_id)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        let binding = EvmNetworkBinding::new(network_id, self.expected_chain_id)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        let source_ref = EvmSourceRef::new(&self.source_ref)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        let policy_id = EvmSourcePolicyId::new(&self.policy_id)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        RedactedEvmSourceEvidence::from_binding(
            &binding,
            self.observed_chain_id,
            source_ref,
            policy_id,
        )
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.jsonrpc",
    name = "joint_tip_read_evidence",
    version = "1",
    schema = "mfm.evm.jsonrpc.external_read.joint_tip"
)]
struct EvmJointTipReadEvidence {
    request_block_selector: String,
    response_block_number: u64,
    response_block_hash: String,
    source: EvmSourceReadEvidence,
}

impl EvmJointTipReadEvidence {
    fn from_capability(
        request: &EvmBlockReadRequest,
        response: &mfm_evm_capabilities::EvmBlockReadResponse,
    ) -> Self {
        Self {
            request_block_selector: evm_block_selector_text(request.block()),
            response_block_number: response.block_number,
            response_block_hash: format!("{:#x}", response.block_hash),
            source: EvmSourceReadEvidence::from_capability(&response.evidence),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.jsonrpc",
    name = "native_balance_read_evidence",
    version = "1",
    schema = "mfm.evm.jsonrpc.external_read.native_balance"
)]
struct EvmNativeBalanceReadEvidence {
    balance_request_account: String,
    balance_request_block_selector: String,
    balance_response_wei: String,
    balance_source: EvmSourceReadEvidence,
    verification_request_block_selector: String,
    verification_response_block_number: u64,
    verification_response_block_hash: String,
    verification_source: EvmSourceReadEvidence,
}

impl EvmNativeBalanceReadEvidence {
    fn from_capability(
        balance_request: &EvmBalanceReadRequest,
        balance_response: &mfm_evm_capabilities::EvmBalanceReadResponse,
        verification_request: &EvmBlockReadRequest,
        verification_response: &mfm_evm_capabilities::EvmBlockReadResponse,
    ) -> Self {
        Self {
            balance_request_account: format!("{:#x}", balance_request.account()),
            balance_request_block_selector: evm_block_selector_text(balance_request.block()),
            balance_response_wei: balance_response.balance_wei.to_string(),
            balance_source: EvmSourceReadEvidence::from_capability(&balance_response.evidence),
            verification_request_block_selector: evm_block_selector_text(
                verification_request.block(),
            ),
            verification_response_block_number: verification_response.block_number,
            verification_response_block_hash: format!("{:#x}", verification_response.block_hash),
            verification_source: EvmSourceReadEvidence::from_capability(
                &verification_response.evidence,
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.jsonrpc",
    name = "canonical_hash_selector_evidence",
    version = "1",
    schema = "mfm.evm.jsonrpc.external_read.canonical_hash_selector"
)]
struct EvmCanonicalHashSelectorEvidence {
    block_hash: String,
    require_canonical: bool,
}

impl EvmCanonicalHashSelectorEvidence {
    fn from_selector(selector: &EvmBlockSelector) -> Result<Self> {
        let EvmBlockSelector::Hash(block_hash) = selector else {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        };
        Ok(Self {
            block_hash: format!("{block_hash:#x}"),
            require_canonical: true,
        })
    }

    fn to_selector(&self) -> Result<EvmBlockSelector> {
        if !self.require_canonical {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let block_hash = parse_block_hash(&self.block_hash)?;
        if format!("{block_hash:#x}") != self.block_hash {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        Ok(EvmBlockSelector::Hash(block_hash))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.jsonrpc",
    name = "erc20_call_read_evidence",
    version = "1",
    schema = "mfm.evm.jsonrpc.external_read.erc20_call"
)]
struct EvmErc20CallReadEvidence {
    destination: String,
    calldata: String,
    block_selector: EvmCanonicalHashSelectorEvidence,
    return_data: String,
    source: EvmSourceReadEvidence,
    verification_block_hash: String,
    verification_response_block_number: u64,
    verification_response_block_hash: String,
    verification_source: EvmSourceReadEvidence,
}

impl EvmErc20CallReadEvidence {
    fn from_capability(
        request: &EvmCallReadRequest,
        response: &EvmCallReadResponse,
        verification_request: &EvmBlockReadRequest,
        verification_response: &mfm_evm_capabilities::EvmBlockReadResponse,
    ) -> Result<Self> {
        Ok(Self {
            destination: format!("{:#x}", request.to()),
            calldata: mfm_evm_core::hex::bytes_to_hex_prefixed(request.calldata()),
            block_selector: EvmCanonicalHashSelectorEvidence::from_selector(request.block())?,
            return_data: mfm_evm_core::hex::bytes_to_hex_prefixed(&response.return_data),
            source: EvmSourceReadEvidence::from_capability(&response.evidence),
            verification_block_hash: hash_selector_text(verification_request.block())?,
            verification_response_block_number: verification_response.block_number,
            verification_response_block_hash: format!("{:#x}", verification_response.block_hash),
            verification_source: EvmSourceReadEvidence::from_capability(
                &verification_response.evidence,
            ),
        })
    }

    fn replay_material(
        &self,
    ) -> Result<(
        EvmCallReadRequest,
        EvmCallReadResponse,
        EvmBlockReadRequest,
        mfm_evm_capabilities::EvmBlockReadResponse,
    )> {
        let destination = parse_account(&self.destination)?;
        if format!("{destination:#x}") != self.destination {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let calldata = mfm_evm_core::hex::hex_to_bytes(&self.calldata)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        if mfm_evm_core::hex::bytes_to_hex_prefixed(&calldata) != self.calldata {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let return_data = mfm_evm_core::hex::hex_to_bytes(&self.return_data)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        if mfm_evm_core::hex::bytes_to_hex_prefixed(&return_data) != self.return_data {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let block = self.block_selector.to_selector()?;
        let verification_block_hash = parse_block_hash(&self.verification_block_hash)?;
        if format!("{verification_block_hash:#x}") != self.verification_block_hash {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let verification_hash = parse_block_hash(&self.verification_response_block_hash)?;
        if format!("{verification_hash:#x}") != self.verification_response_block_hash {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        Ok((
            EvmCallReadRequest::new(destination, calldata, block),
            EvmCallReadResponse {
                evidence: self.source.to_capability()?,
                return_data,
            },
            EvmBlockReadRequest::new(EvmBlockSelector::Hash(verification_block_hash)),
            mfm_evm_capabilities::EvmBlockReadResponse {
                evidence: self.verification_source.to_capability()?,
                block_number: self.verification_response_block_number,
                block_hash: verification_hash,
            },
        ))
    }
}

fn hash_selector_text(selector: &EvmBlockSelector) -> Result<String> {
    let EvmBlockSelector::Hash(block_hash) = selector else {
        return Err(EvmAdapterError::InvalidCapabilityRequest);
    };
    Ok(format!("{block_hash:#x}"))
}

fn evm_block_selector_text(selector: &EvmBlockSelector) -> String {
    match selector {
        EvmBlockSelector::Latest => "latest".to_owned(),
        EvmBlockSelector::Pending => "pending".to_owned(),
        EvmBlockSelector::Number(number) => format!("number:{number}"),
        EvmBlockSelector::Hash(hash) => format!("hash:{hash:#x}"),
    }
}

/// Result type for EVM adapter operations.
pub type Result<T> = std::result::Result<T, EvmAdapterError>;

/// Factory for EVM network providers bound to a certified network binding.
pub trait EvmProviderFactory: Send + Sync {
    /// Validates that the binding can resolve without live network IO.
    fn validate_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()>;

    /// Binds a checked network binding to a provider that supports block, balance, and call reads.
    fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmBoundProvider>>;
}

/// Bound EVM provider exposing the collector-needed read surfaces.
pub trait EvmBoundProvider:
    EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

impl<T> EvmBoundProvider for T where
    T: EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

/// Runtime capabilities used by EVM collector runners.
#[derive(Clone)]
pub struct EvmRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl EvmRunnerCapabilities {
    /// Creates runner capabilities from artifact and EVM provider factory.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        evm: Arc<dyn EvmProviderFactory>,
    ) -> Self {
        Self { artifacts, evm }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn evm(&self) -> Arc<dyn EvmProviderFactory> {
        Arc::clone(&self.evm)
    }
}

/// Registers typed EVM collector runners.
pub fn register_evm_collectors_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    let evm = capabilities.evm();
    registry.register_capability_spec::<EvmBlockReadCapability>(
        CapabilityImplementationId::new(EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID)?,
    )?;
    registry.register_capability_spec::<EvmBalanceReadCapability>(
        CapabilityImplementationId::new(EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID)?,
    )?;
    registry.register_capability_spec::<EvmCallReadCapability>(CapabilityImplementationId::new(
        EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID,
    )?)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm",
        "typed-evm-jsonrpc",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(PURE_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let managed_write_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(MANAGED_WRITE_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<ResolveEvmJointTipState>(
        &read_factory,
        Arc::new(ResolveJointTipRunner {
            artifacts: artifacts.clone(),
            evm: evm.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveEvmNativeBalanceState>(
        &read_factory,
        Arc::new(ObserveNativeBalanceRunner {
            artifacts: artifacts.clone(),
            evm: evm.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20TokenMetadataState>(
        &read_factory,
        Arc::new(ObserveErc20TokenMetadataRunner {
            artifacts: artifacts.clone(),
            evm: evm.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20BalanceState>(
        &read_factory,
        Arc::new(ObserveErc20BalanceRunner {
            artifacts: artifacts.clone(),
            evm,
        }),
    )?;
    registrations.register_state_runner_with_factory::<RecordEvmNativeBalanceFactState>(
        &managed_write_factory,
        Arc::new(
            ManagedFactRecordRunner::<RecordEvmNativeBalanceFactState>::new(
                artifacts.clone(),
                native_balance_record_visibility(),
            ),
        ),
    )?;
    registrations.register_state_runner_with_factory::<RecordErc20BalanceFactState>(
        &managed_write_factory,
        Arc::new(ManagedFactRecordRunner::<RecordErc20BalanceFactState>::new(
            artifacts.clone(),
            erc20_balance_record_visibility(),
        )),
    )?;
    registrations.register_state_runner_with_factory::<AssembleEvmNativeBalanceBatchState>(
        &pure_factory,
        Arc::new(AssembleNativeBalanceBatchRunner { artifacts }),
    )?;
    Ok(())
}

/// Redaction-safe EVM adapter error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmAdapterError {
    /// Adapter could not build a capability request from certified state config.
    #[error("EVM adapter could not build capability request")]
    InvalidCapabilityRequest,
}

fn network_binding(network: &str, chain_id: u64) -> Result<EvmNetworkBinding> {
    let network_id =
        EvmNetworkId::new(network).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    EvmNetworkBinding::new(network_id, chain_id)
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

fn joint_tip_binding(config: &ResolveEvmJointTipConfig) -> Result<EvmNetworkBinding> {
    network_binding(&config.network, config.chain_id)
}

fn balance_binding(config: &ObserveEvmNativeBalanceConfig) -> Result<EvmNetworkBinding> {
    let (network, chain_id, _) = config
        .evm_network_parts()
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    network_binding(network, chain_id)
}

fn erc20_metadata_binding(config: &ObserveErc20TokenMetadataConfig) -> Result<EvmNetworkBinding> {
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    network_binding(network, chain_id)
}

fn erc20_balance_binding(config: &ObserveErc20BalanceConfig) -> Result<EvmNetworkBinding> {
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    network_binding(network, chain_id)
}

fn joint_tip_config_for_observation(
    config: &ObserveEvmNativeBalanceConfig,
) -> Result<ResolveEvmJointTipConfig> {
    let (network, chain_id, _) = config
        .evm_network_parts()
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    let max_source_reads = NonZeroU64::new(EVM_JOINT_TIP_SOURCE_READS)
        .ok_or(EvmAdapterError::InvalidCapabilityRequest)?;
    Ok(ResolveEvmJointTipConfig {
        network: network.to_owned(),
        chain_id,
        max_source_reads,
    })
}

fn parse_block_hash(value: &str) -> Result<B256> {
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    B256::from_str(hex).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

fn parse_account(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

struct ResolveJointTipRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl ErasedNodeRunner for ResolveJointTipRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ResolveEvmJointTipConfig>(&ctx, ctx.node())?;
        let binding = joint_tip_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ResolveEvmJointTipConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding = joint_tip_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
            let state = ResolveEvmJointTipState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let _input = load_materialized_struct_input::<ResolveEvmJointTipInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let provider = self
                .evm
                .bind_network(binding)
                .map_err(evm_capability_runtime_error)?;
            let response = provider
                .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Latest))
                .await
                .map_err(evm_capability_runtime_error)?;
            let tip = state
                .materialize_response(&response)
                .map_err(evm_state_runtime_error)?;
            let request = EvmBlockReadRequest::new(EvmBlockSelector::Latest);
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&EvmJointTipReadEvidence::from_capability(
                &request, &response,
            ))?;
            output.state_output(&tip)?;
            Ok(output.finish())
        })
    }
}

struct ObserveNativeBalanceRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl ErasedNodeRunner for ObserveNativeBalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config =
            load_launch_config_for_node::<ObserveEvmNativeBalanceConfig>(&ctx, ctx.node())?;
        let binding = balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ObserveEvmNativeBalanceConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding = balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
            let state = ObserveEvmNativeBalanceState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveEvmNativeBalanceInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let account =
                parse_account(&state.config().account).map_err(evm_adapter_runtime_error)?;
            let tip_hash = parse_block_hash(input.joint_tip.block_hash())
                .map_err(evm_adapter_runtime_error)?;
            let provider = self
                .evm
                .bind_network(binding)
                .map_err(evm_capability_runtime_error)?;
            // Hash-bound balance (EIP-1898 style).
            let balance = provider
                .read_balance(&EvmBalanceReadRequest::new(
                    account,
                    EvmBlockSelector::Hash(tip_hash),
                ))
                .await
                .map_err(evm_capability_runtime_error)?;
            // Re-read tip by hash to prove no drift / mismatch before write.
            let verified_block = provider
                .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Hash(tip_hash)))
                .await
                .map_err(evm_capability_runtime_error)?;
            let verified_tip = materialize_evm_joint_tip(
                &joint_tip_config_for_observation(state.config())
                    .map_err(evm_adapter_runtime_error)?,
                &verified_block,
            )
            .map_err(evm_state_runtime_error)?;
            // Explicit tip equality check (number must match joint tip as well).
            if verified_tip.block_number() != input.joint_tip.block_number()
                || verified_tip.block_hash() != input.joint_tip.block_hash()
            {
                return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "tip drift or hash mismatch before Platform write".to_owned(),
                ));
            }
            let observation = state
                .materialize_response(&input, &verified_tip, &balance)
                .map_err(evm_state_runtime_error)?;
            let balance_request =
                EvmBalanceReadRequest::new(account, EvmBlockSelector::Hash(tip_hash));
            let verification_request = EvmBlockReadRequest::new(EvmBlockSelector::Hash(tip_hash));
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(
                &EvmNativeBalanceReadEvidence::from_capability(
                    &balance_request,
                    &balance,
                    &verification_request,
                    &verified_block,
                ),
            )?;
            output.state_output(&observation)?;
            Ok(output.finish())
        })
    }
}

struct ObserveErc20TokenMetadataRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl ErasedNodeRunner for ObserveErc20TokenMetadataRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config =
            load_launch_config_for_node::<ObserveErc20TokenMetadataConfig>(&ctx, ctx.node())?;
        let binding = erc20_metadata_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ObserveErc20TokenMetadataConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding =
                erc20_metadata_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
            let state = ObserveErc20TokenMetadataState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveErc20TokenMetadataInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let request = state
                .call_request(&input)
                .map_err(evm_state_runtime_error)?;
            let verification_request = EvmBlockReadRequest::new(request.block().clone());
            let provider = self
                .evm
                .bind_network(binding)
                .map_err(evm_capability_runtime_error)?;
            let response = provider
                .read_call(&request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification = provider
                .read_block(&verification_request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let metadata = state
                .materialize_response(&input, &request, &response, &verification)
                .map_err(evm_state_runtime_error)?;
            let evidence = EvmErc20CallReadEvidence::from_capability(
                &request,
                &response,
                &verification_request,
                &verification,
            )
            .map_err(evm_adapter_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&evidence)?;
            output.state_output(&metadata)?;
            Ok(output.finish())
        })
    }
}

struct ObserveErc20BalanceRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl ErasedNodeRunner for ObserveErc20BalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ObserveErc20BalanceConfig>(&ctx, ctx.node())?;
        let binding = erc20_balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ObserveErc20BalanceConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding =
                erc20_balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
            let state = ObserveErc20BalanceState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveErc20BalanceInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let request = state
                .call_request(&input)
                .map_err(evm_state_runtime_error)?;
            let verification_request = EvmBlockReadRequest::new(request.block().clone());
            let provider = self
                .evm
                .bind_network(binding)
                .map_err(evm_capability_runtime_error)?;
            let response = provider
                .read_call(&request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification = provider
                .read_block(&verification_request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let observation = state
                .materialize_response(&input, &request, &response, &verification)
                .map_err(evm_state_runtime_error)?;
            let evidence = EvmErc20CallReadEvidence::from_capability(
                &request,
                &response,
                &verification_request,
                &verification,
            )
            .map_err(evm_adapter_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&evidence)?;
            output.state_output(&observation)?;
            Ok(output.finish())
        })
    }
}

struct AssembleNativeBalanceBatchRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleNativeBalanceBatchRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<AssembleEvmNativeBalanceBatchConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleEvmNativeBalanceBatchInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let summary = assemble_evm_native_balance_batch(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &summary)
        })
    }
}

struct ManagedFactRecordRunner<S> {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    visibility: mfm_program::facts::FactVisibility,
    _state: PhantomData<fn() -> S>,
}

impl<S> ManagedFactRecordRunner<S> {
    fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        visibility: mfm_program::facts::FactVisibility,
    ) -> Self {
        Self {
            artifacts,
            visibility,
            _state: PhantomData,
        }
    }
}

impl<S> ErasedNodeRunner for ManagedFactRecordRunner<S>
where
    S: ManagedWriteState<Caps = (FactRecordCapability,)>,
    S::Input: serde::de::DeserializeOwned,
    S::Output: MfmFactType + MfmValue,
{
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config_for_node::<S::Config>(ctx.node(), self.artifacts.as_ref())
                    .await?;
            let state = S::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input =
                load_materialized_struct_input::<S::Input>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let context = ctx.certified_context::<S::Context>()?;
            let fact = state
                .run(input, &(FactRecordCapability,), &context)
                .await
                .map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.state_output_and_record_fact(
                mfm_runtime::FactRecordInput::new(fact, self.visibility.clone()),
                fact_record_capability_binding()?,
            )?;
            Ok(output.finish())
        })
    }
}

fn fact_record_capability_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<FactRecordCapability>(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
    )
}

fn evm_capability_runtime_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn evm_state_runtime_error(error: mfm_states_evm::EvmStateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn evm_adapter_runtime_error(error: EvmAdapterError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

/// Verifies EVM collector outputs from retained capability read evidence only.
pub fn verify_evm_collector_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    verify_evm_joint_tip_replay(broker)?;
    let native_frames = replay_frames_for_state::<ObserveEvmNativeBalanceState>(broker)?;
    for frame in &native_frames {
        verify_evm_native_balance_observation_replay(broker, frame)?;
    }
    verify_evm_native_balance_fact_replay(broker)?;

    let metadata_frames = replay_frames_for_state::<ObserveErc20TokenMetadataState>(broker)?;
    for frame in &metadata_frames {
        verify_erc20_token_metadata_replay(broker, frame)?;
    }
    let balance_frames = replay_frames_for_state::<ObserveErc20BalanceState>(broker)?;
    for frame in &balance_frames {
        verify_erc20_balance_observation_replay(broker, frame)?;
    }
    verify_erc20_balance_fact_replay(broker)?;

    verify_evm_shared_joint_tips(broker, &native_frames, &metadata_frames, &balance_frames)?;
    verify_evm_native_balance_batch_replay(broker)?;
    Ok(())
}

fn replay_frames_for_state<S: StateSpec>(
    broker: &replay::ReplayBroker,
) -> replay::Result<Vec<replay::ProducedCellReplayFrame>> {
    let state_kind = S::kind().map_err(replay_adapter_error)?;
    let state_version = S::version().map_err(replay_adapter_error)?;
    broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })
}

fn verify_evm_shared_joint_tips(
    broker: &replay::ReplayBroker,
    native_frames: &[replay::ProducedCellReplayFrame],
    metadata_frames: &[replay::ProducedCellReplayFrame],
    balance_frames: &[replay::ProducedCellReplayFrame],
) -> replay::Result<()> {
    let mut anchors = BTreeMap::<String, (u64, String)>::new();
    for frame in native_frames {
        let config: ObserveEvmNativeBalanceConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (tip.block_number(), tip.block_hash().to_owned());
        let (network, _, _) = config.evm_network_parts().map_err(replay_adapter_error)?;
        if anchors
            .insert(network.to_owned(), anchor.clone())
            .is_some_and(|existing| existing != anchor)
        {
            return Err(replay_evm_mismatch(
                "EVM same-network observations did not share one joint tip",
            ));
        }
    }
    for frame in metadata_frames {
        let config: ObserveErc20TokenMetadataConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (tip.block_number(), tip.block_hash().to_owned());
        let (network, _) = config.evm_network_parts().map_err(replay_adapter_error)?;
        if anchors
            .insert(network.to_owned(), anchor.clone())
            .is_some_and(|existing| existing != anchor)
        {
            return Err(replay_evm_mismatch(
                "EVM same-network observations did not share one joint tip",
            ));
        }
    }
    for frame in balance_frames {
        let config: ObserveErc20BalanceConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (tip.block_number(), tip.block_hash().to_owned());
        let (network, _) = config.evm_network_parts().map_err(replay_adapter_error)?;
        if anchors
            .insert(network.to_owned(), anchor.clone())
            .is_some_and(|existing| existing != anchor)
        {
            return Err(replay_evm_mismatch(
                "EVM same-network observations did not share one joint tip",
            ));
        }
    }
    Ok(())
}

fn verify_evm_joint_tip_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = ResolveEvmJointTipState::kind().map_err(replay_adapter_error)?;
    let state_version = ResolveEvmJointTipState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let config: ResolveEvmJointTipConfig = replay_node_config(broker, &frame.node)?;
        let evidence: EvmJointTipReadEvidence = replay_external_read_evidence(broker, frame)?;
        if evidence.request_block_selector != "latest"
            || !evm_source_matches(&evidence.source, &config.network, config.chain_id)
        {
            return Err(replay_evm_mismatch(
                "EVM joint-tip read evidence did not match certified request or source binding",
            ));
        }
        let block_hash =
            parse_block_hash(&evidence.response_block_hash).map_err(replay_adapter_error)?;
        let expected = EvmJointTip::new(
            config.network,
            config.chain_id,
            evidence.response_block_number,
            format!("{block_hash:#x}"),
        )
        .map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn verify_evm_native_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveEvmNativeBalanceConfig = replay_node_config(broker, &frame.node)?;
    let input_tip = replay_joint_tip_input(broker, &frame.node)?;
    let evidence: EvmNativeBalanceReadEvidence = replay_external_read_evidence(broker, frame)?;
    let account = parse_account(&config.account).map_err(replay_adapter_error)?;
    let expected_selector = format!(
        "hash:{:#x}",
        parse_block_hash(input_tip.block_hash()).map_err(replay_adapter_error)?
    );
    let (network, chain_id, _) = config.evm_network_parts().map_err(replay_adapter_error)?;
    let verified_tip = EvmJointTip::new(
        network,
        chain_id,
        evidence.verification_response_block_number,
        &evidence.verification_response_block_hash,
    )
    .map_err(replay_adapter_error)?;
    if evidence.balance_request_account != format!("{account:#x}")
        || evidence.balance_request_block_selector != expected_selector
        || evidence.verification_request_block_selector != expected_selector
        || evidence.verification_response_block_number != input_tip.block_number()
        || verified_tip.block_hash() != input_tip.block_hash()
        || !evm_source_matches(&evidence.balance_source, network, chain_id)
        || !evm_source_matches(&evidence.verification_source, network, chain_id)
    {
        return Err(replay_evm_mismatch(
            "EVM native-balance read evidence did not match certified requests or source binding",
        ));
    }
    let expected = normalize_evm_native_balance_observation(
        &config,
        &input_tip,
        &verified_tip,
        &evidence.balance_response_wei,
        evidence.balance_source.observed_chain_id,
        &evidence.balance_source.network_id,
    )
    .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn verify_erc20_token_metadata_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveErc20TokenMetadataConfig = replay_node_config(broker, &frame.node)?;
    let joint_tip = replay_joint_tip_input(broker, &frame.node)?;
    let (request, response, verification_request, verification) =
        replay_erc20_call_evidence(broker, frame)?;
    if verification_request.block() != request.block() {
        return Err(replay_evm_mismatch(
            "ERC-20 metadata anchor re-verification did not use the call hash selector",
        ));
    }
    let (network, chain_id) = config.evm_network_parts().map_err(replay_adapter_error)?;
    if !evm_source_matches(
        &EvmSourceReadEvidence::from_capability(&response.evidence),
        network,
        chain_id,
    ) || !evm_source_matches(
        &EvmSourceReadEvidence::from_capability(&verification.evidence),
        network,
        chain_id,
    ) {
        return Err(replay_evm_mismatch(
            "ERC-20 metadata read evidence did not match certified source binding",
        ));
    }
    let expected = normalize_erc20_token_metadata_from_capability(
        &config,
        &joint_tip,
        &request,
        &response,
        &verification,
    )
    .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn verify_erc20_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveErc20BalanceConfig = replay_node_config(broker, &frame.node)?;
    let joint_tip = replay_joint_tip_input(broker, &frame.node)?;
    let metadata = replay_erc20_metadata_input(broker, &frame.node)?;
    let input = ObserveErc20BalanceInput {
        joint_tip,
        metadata,
    };
    let (request, response, verification_request, verification) =
        replay_erc20_call_evidence(broker, frame)?;
    if verification_request.block() != request.block() {
        return Err(replay_evm_mismatch(
            "ERC-20 balance anchor re-verification did not use the call hash selector",
        ));
    }
    let (network, chain_id) = config.evm_network_parts().map_err(replay_adapter_error)?;
    if !evm_source_matches(
        &EvmSourceReadEvidence::from_capability(&response.evidence),
        network,
        chain_id,
    ) || !evm_source_matches(
        &EvmSourceReadEvidence::from_capability(&verification.evidence),
        network,
        chain_id,
    ) {
        return Err(replay_evm_mismatch(
            "ERC-20 balance read evidence did not match certified source binding",
        ));
    }
    let expected = normalize_erc20_balance_from_capability(
        &config,
        &input,
        &request,
        &response,
        &verification,
    )
    .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn replay_erc20_call_evidence(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<(
    EvmCallReadRequest,
    EvmCallReadResponse,
    EvmBlockReadRequest,
    mfm_evm_capabilities::EvmBlockReadResponse,
)> {
    let evidence: EvmErc20CallReadEvidence = replay_external_read_evidence(broker, frame)?;
    evidence.replay_material().map_err(replay_adapter_error)
}

fn verify_evm_native_balance_fact_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = RecordEvmNativeBalanceFactState::kind().map_err(replay_adapter_error)?;
    let state_version = RecordEvmNativeBalanceFactState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let observation_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmAddressNativeBalanceObservation::semantic_id().map_err(replay_adapter_error)?,
            &EvmAddressNativeBalanceObservation::schema_id().map_err(replay_adapter_error)?,
        )?;
        if observation_frames.len() != 1 {
            return Err(replay_evm_mismatch(
                "EVM native-balance fact input was incomplete",
            ));
        }
        let observation: EvmAddressNativeBalanceObservation =
            decode_replay_value(&observation_frames[0])?;
        let fact = observation.to_fact();
        ensure_canonical_value_matches(&fact, &frame.artifact_bytes)?;
        verify_recorded_fact_evidence(
            broker,
            frame,
            &EvmAddressNativeBalanceSnapshotFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_erc20_balance_fact_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let frames = replay_frames_for_state::<RecordErc20BalanceFactState>(broker)?;
    for frame in &frames {
        let observation_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmAddressErc20BalanceObservation::semantic_id().map_err(replay_adapter_error)?,
            &EvmAddressErc20BalanceObservation::schema_id().map_err(replay_adapter_error)?,
        )?;
        if observation_frames.len() != 1 {
            return Err(replay_evm_mismatch(
                "ERC-20 balance fact input was incomplete",
            ));
        }
        let observation: EvmAddressErc20BalanceObservation =
            decode_replay_value(&observation_frames[0])?;
        let fact = observation.try_to_fact().map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&fact, &frame.artifact_bytes)?;
        verify_recorded_fact_evidence(
            broker,
            frame,
            &EvmAddressErc20BalanceSnapshotFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_recorded_fact_evidence<S: serde::Serialize, R: serde::Serialize>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    descriptor: &mfm_facts::FactDescriptor,
    subject: &S,
    response_value: &R,
) -> replay::Result<()> {
    let records = broker
        .events()
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::FactRecorded(payload)
                if payload.node_id == frame.produced.node_id
                    && payload.attempt_id == frame.produced.attempt_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(replay_evm_mismatch(
            "EVM fact output did not have exactly one FactRecorded event",
        ));
    }
    let claim = &records[0].claim;
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(descriptor).map_err(replay_adapter_error)?;
    if claim.fact_descriptor_hash() != &descriptor_hash
        || claim.fact_kind() != descriptor.fact_kind()
    {
        return Err(replay_evm_mismatch(
            "EVM FactRecorded descriptor authority did not match the recomputed fact",
        ));
    }
    let subject_json = serde_json::to_value(subject).map_err(replay_json_error)?;
    let expected_subject = mfm_facts::typed_fact_subject_evidence(descriptor, &subject_json)
        .map_err(replay_adapter_error)?;
    if claim.subject() != &expected_subject {
        return Err(replay_evm_mismatch(
            "EVM FactRecorded subject authority did not match the recomputed fact",
        ));
    }
    let expected_bytes = canonical_json_bytes(response_value)?;
    let response = claim.response();
    if response.response_schema_id() != descriptor.response_schema_id()
        || response.response_hash() != &expected_bytes.content_digest()
    {
        return Err(replay_evm_mismatch(
            "EVM FactRecorded response authority did not match the recomputed fact",
        ));
    }
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::FactResponse,
        artifact_id: response.artifact_id().clone(),
        evidence_hash: response.artifact_evidence_hash().clone(),
        digest: Some(response.response_hash().clone()),
        byte_len: Some(
            u64::try_from(expected_bytes.as_bytes().len())
                .map_err(|_| replay_evm_mismatch("EVM fact response length overflowed u64"))?,
        ),
        media_type: None,
        schema_id: Some(response.response_schema_id().clone()),
        semantic_type_id: None,
        producer_node_id: Some(frame.produced.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::FactResponse),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    if artifact.artifact_bytes != expected_bytes.as_bytes() {
        return Err(replay_evm_mismatch(
            "EVM FactRecorded response bytes did not match the recomputed fact",
        ));
    }
    Ok(())
}

fn canonical_json_bytes<T: serde::Serialize>(value: &T) -> replay::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(replay_json_error)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(replay_adapter_error)
}

fn verify_evm_native_balance_batch_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = AssembleEvmNativeBalanceBatchState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleEvmNativeBalanceBatchState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let _config: AssembleEvmNativeBalanceBatchConfig = replay_node_config(broker, &frame.node)?;
        let joint_tip_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmJointTip::semantic_id().map_err(replay_adapter_error)?,
            &EvmJointTip::schema_id().map_err(replay_adapter_error)?,
        )?;
        let fact_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmAddressNativeBalanceSnapshotFact::semantic_id().map_err(replay_adapter_error)?,
            &EvmAddressNativeBalanceSnapshotFact::schema_id().map_err(replay_adapter_error)?,
        )?;
        if joint_tip_frames.len() != 1 || fact_frames.is_empty() {
            return Err(replay_evm_mismatch(
                "EVM native-balance batch inputs were incomplete",
            ));
        }
        let joint_tip: EvmJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let facts = fact_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<EvmAddressNativeBalanceSnapshotFact>>>()?;
        let input = AssembleEvmNativeBalanceBatchInput {
            joint_tip,
            balance_facts: NonEmpty::try_from_vec(facts).map_err(replay_adapter_error)?,
        };
        let expected = assemble_evm_native_balance_batch(input).map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn replay_joint_tip_input(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<EvmJointTip> {
    let frames = replay_input_frames(
        broker,
        node,
        &EvmJointTip::semantic_id().map_err(replay_adapter_error)?,
        &EvmJointTip::schema_id().map_err(replay_adapter_error)?,
    )?;
    if frames.len() != 1 {
        return Err(replay_evm_mismatch(
            "EVM observation did not consume exactly one joint tip",
        ));
    }
    let resolve_kind = ResolveEvmJointTipState::kind().map_err(replay_adapter_error)?;
    let resolve_version = ResolveEvmJointTipState::version().map_err(replay_adapter_error)?;
    if frames[0].node.state_kind != resolve_kind || frames[0].node.state_version != resolve_version
    {
        return Err(replay_evm_mismatch(
            "EVM observation input was not produced by joint-tip resolution",
        ));
    }
    decode_replay_value(&frames[0])
}

fn replay_erc20_metadata_input(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<EvmErc20TokenMetadata> {
    let frames = replay_input_frames(
        broker,
        node,
        &EvmErc20TokenMetadata::semantic_id().map_err(replay_adapter_error)?,
        &EvmErc20TokenMetadata::schema_id().map_err(replay_adapter_error)?,
    )?;
    if frames.len() != 1 {
        return Err(replay_evm_mismatch(
            "ERC-20 balance observation did not consume exactly one token metadata value",
        ));
    }
    let state_kind = ObserveErc20TokenMetadataState::kind().map_err(replay_adapter_error)?;
    let state_version = ObserveErc20TokenMetadataState::version().map_err(replay_adapter_error)?;
    if frames[0].node.state_kind != state_kind || frames[0].node.state_version != state_version {
        return Err(replay_evm_mismatch(
            "ERC-20 balance metadata input was not produced by metadata observation",
        ));
    }
    decode_replay_value(&frames[0])
}

fn replay_input_frames(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
    semantic_type_id: &mfm_ids::SemanticTypeId,
    schema_id: &mfm_ids::SchemaId,
) -> replay::Result<Vec<replay::ProducedCellReplayFrame>> {
    let mut cells = Vec::new();
    collect_input_cells(
        &node.input_bindings.root,
        semantic_type_id,
        schema_id,
        &mut cells,
    );
    let mut frames = Vec::with_capacity(cells.len());
    for input_cell in cells {
        let matches = broker.produced_cell_frames_matching(|_node, cell, _produced| {
            Ok(cell.cell_id == input_cell.cell_id)
        })?;
        if matches.len() != 1 {
            return Err(replay_evm_mismatch(
                "certified EVM collector input cell did not have exactly one produced value",
            ));
        }
        let frame = matches.into_iter().next().expect("one frame");
        if frame.cell.semantic_type_id != input_cell.semantic_type_id
            || frame.cell.schema_id != input_cell.schema_id
            || frame.cell.value_lineage != input_cell.value_lineage
        {
            return Err(replay_evm_mismatch(
                "EVM collector input cell metadata did not match its produced value",
            ));
        }
        frames.push(frame);
    }
    Ok(frames)
}

fn collect_input_cells<'a>(
    input: &'a mfm_spec::v1::InputBindingNodeSpec,
    semantic_type_id: &mfm_ids::SemanticTypeId,
    schema_id: &mfm_ids::SchemaId,
    cells: &mut Vec<&'a mfm_spec::v1::InputBindingCellSpec>,
) {
    match input {
        mfm_spec::v1::InputBindingNodeSpec::Unit => {}
        mfm_spec::v1::InputBindingNodeSpec::Cell(cell) => {
            if &cell.semantic_type_id == semantic_type_id && &cell.schema_id == schema_id {
                cells.push(cell);
            }
        }
        mfm_spec::v1::InputBindingNodeSpec::Tuple(elements)
        | mfm_spec::v1::InputBindingNodeSpec::Vec { elements, .. }
        | mfm_spec::v1::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element, semantic_type_id, schema_id, cells);
            }
        }
        mfm_spec::v1::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells(&field.node, semantic_type_id, schema_id, cells);
            }
        }
    }
}

fn decode_replay_value<T>(frame: &replay::ProducedCellReplayFrame) -> replay::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)
}

fn replay_external_read_evidence<T>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let references = broker
        .events()
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(reference)
                if reference.node_id.as_ref() == Some(&frame.produced.node_id)
                    && reference.attempt_id.as_ref() == Some(&frame.produced.attempt_id)
                    && reference.artifact_ref.role
                        == events::ArtifactRole::ExternalReadEvidence
                    && reference.artifact_ref.schema_id == T::schema_id().ok()? =>
            {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if references.len() != 1 {
        return Err(replay_evm_mismatch(
            "EVM external read did not have exactly one retained evidence artifact",
        ));
    }
    let reference = references[0];
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference.artifact_ref.artifact_id.clone(),
        evidence_hash: reference.artifact_ref.evidence_hash.clone(),
        digest: Some(reference.artifact_ref.content_digest.clone()),
        byte_len: Some(reference.artifact_ref.byte_len),
        media_type: Some(reference.artifact_ref.media_type.clone()),
        schema_id: Some(reference.artifact_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(frame.produced.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::ExternalReadEvidence),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)
}

fn evm_source_matches(source: &EvmSourceReadEvidence, network: &str, chain_id: u64) -> bool {
    source.network_id == network
        && source.expected_chain_id == chain_id
        && source.observed_chain_id == chain_id
        && !source.source_ref.trim().is_empty()
        && !source.policy_id.trim().is_empty()
}

fn ensure_canonical_value_matches<T: serde::Serialize>(
    expected: &T,
    actual: &[u8],
) -> replay::Result<()> {
    let json = serde_json::to_string(expected).map_err(replay_json_error)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(replay_adapter_error)?;
    if canonical.as_bytes() != actual {
        return Err(replay_evm_mismatch(
            "EVM collector output did not match recomputed state output",
        ));
    }
    Ok(())
}

fn replay_node_config<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let config_evidence = store::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        evidence_hash: config_evidence
            .evidence_hash()
            .map_err(replay_adapter_error)?,
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    let config: T = serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(replay_adapter_error)
}

fn replay_json_error(error: serde_json::Error) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_adapter_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_evm_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const TOKEN: &str = "0x0000000000000000000000000000000000000001";

    fn source_evidence() -> RedactedEvmSourceEvidence {
        let binding =
            EvmNetworkBinding::new(EvmNetworkId::new("ethereum-mainnet").expect("net"), 1)
                .expect("binding");
        RedactedEvmSourceEvidence::from_binding(
            &binding,
            1,
            EvmSourceRef::new("primary").expect("source"),
            EvmSourcePolicyId::new("default").expect("policy"),
        )
        .expect("source evidence")
    }

    fn hash() -> B256 {
        B256::from_str(HASH.strip_prefix("0x").expect("prefix")).expect("hash")
    }

    fn evidence() -> EvmErc20CallReadEvidence {
        let request = EvmCallReadRequest::new(
            Address::from_str(TOKEN).expect("token"),
            vec![0x31, 0x3c, 0xe5, 0x67],
            EvmBlockSelector::Hash(hash()),
        );
        let response = EvmCallReadResponse {
            evidence: source_evidence(),
            return_data: vec![0; 32],
        };
        let verification_request = EvmBlockReadRequest::new(EvmBlockSelector::Hash(hash()));
        let verification_response = mfm_evm_capabilities::EvmBlockReadResponse {
            evidence: source_evidence(),
            block_number: 100,
            block_hash: hash(),
        };
        EvmErc20CallReadEvidence::from_capability(
            &request,
            &response,
            &verification_request,
            &verification_response,
        )
        .expect("evidence")
    }

    #[test]
    fn erc20_evidence_retains_exact_hash_bound_call_and_redacted_source() {
        let evidence = evidence();
        assert_eq!(evidence.destination, TOKEN);
        assert_eq!(evidence.calldata, "0x313ce567");
        assert_eq!(evidence.block_selector.block_hash, HASH);
        assert!(evidence.block_selector.require_canonical);
        assert_eq!(evidence.return_data, format!("0x{}", "00".repeat(32)));
        assert_eq!(evidence.verification_block_hash, HASH);
        assert_eq!(evidence.verification_response_block_number, 100);
        assert_eq!(evidence.verification_response_block_hash, HASH);
        assert_eq!(evidence.source.network_id, "ethereum-mainnet");
        assert_eq!(evidence.source.source_ref, "primary");
        assert_eq!(evidence.source.policy_id, "default");
        let rendered = serde_json::to_string(&evidence).expect("json");
        for forbidden in ["http://", "https://", "authorization", "password"] {
            assert!(!rendered.contains(forbidden), "leaked {forbidden}");
        }

        let (request, response, verification_request, verification_response) =
            evidence.replay_material().expect("replay material");
        assert_eq!(format!("{:#x}", request.to()), TOKEN);
        assert_eq!(request.calldata(), &[0x31, 0x3c, 0xe5, 0x67]);
        assert!(
            matches!(request.block(), EvmBlockSelector::Hash(value) if format!("{value:#x}") == HASH)
        );
        assert_eq!(response.return_data, vec![0; 32]);
        assert_eq!(verification_request.block(), request.block());
        assert_eq!(verification_response.block_number, 100);
        assert_eq!(format!("{:#x}", verification_response.block_hash), HASH);
    }

    #[test]
    fn erc20_evidence_rejects_noncanonical_or_noncanonicality_tampering() {
        let mut missing_canonicality = evidence();
        missing_canonicality.block_selector.require_canonical = false;
        assert!(missing_canonicality.replay_material().is_err());

        let mut noncanonical_hex = evidence();
        noncanonical_hex.calldata = "0x313CE567".to_owned();
        assert!(noncanonical_hex.replay_material().is_err());

        let mut latest_verification = evidence();
        latest_verification.verification_block_hash = "0x".to_owned();
        assert!(latest_verification.replay_material().is_err());
    }
}
