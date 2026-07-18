#![warn(missing_docs)]
//! EVM collector adapter runners.
//!
//! Binds reusable EVM native-balance and ERC-20 state contracts to block, balance, and generic
//! call-read capabilities. Protocol IO stays in the transport crate; this crate owns request
//! mapping, redacted evidence recording, replay recomputation, runner registration, and managed
//! fact recording.

use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, Bytes, B256, U256};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlock, EvmBlockSelector, EvmCall, EvmCapabilityError, EvmNetworkBinding, EvmReadCapability,
    EvmReadSession, EvmSessionEvidence, ProviderDiagnosticCode,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::LocalPublicId;
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec};
use mfm_program_derive::MfmValue;
use mfm_replay::v1::{
    self as replay, decode_produced_value as decode_replay_value,
    external_read_evidence as replay_external_read_evidence,
    load_node_config as replay_node_config, produced_input_frames as replay_input_frames,
};
use mfm_runtime::{
    load_launch_config_for_node, load_materialized_struct_input, load_runner_config_for_node,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_evm::{
    assemble_evm_erc20_balance_batch_receipt, assemble_evm_native_balance_batch_receipt,
    assemble_evm_network_collection_receipt, erc20_balance_record_visibility,
    evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version, materialize_evm_joint_tip,
    native_balance_record_visibility, normalize_erc20_balance_from_capability,
    normalize_erc20_token_metadata_from_capability, AssembleEvmErc20BalanceBatchReceiptConfig,
    AssembleEvmErc20BalanceBatchReceiptInput, AssembleEvmErc20BalanceBatchReceiptState,
    AssembleEvmNativeBalanceBatchReceiptConfig, AssembleEvmNativeBalanceBatchReceiptInput,
    AssembleEvmNativeBalanceBatchReceiptState, AssembleEvmNetworkCollectionReceiptConfig,
    AssembleEvmNetworkCollectionReceiptInput, AssembleEvmNetworkCollectionReceiptState,
    EvmAddressErc20BalanceObservation, EvmAddressErc20BalanceSnapshotFact,
    EvmAddressNativeBalanceObservation, EvmAddressNativeBalanceSnapshotFact,
    EvmErc20BalanceBatchReceipt, EvmErc20TokenMetadata, EvmJointTip, EvmNativeBalanceBatchReceipt,
    ObserveErc20BalanceConfig, ObserveErc20BalanceInput, ObserveErc20BalanceState,
    ObserveErc20TokenMetadataConfig, ObserveErc20TokenMetadataInput,
    ObserveErc20TokenMetadataState, ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceState, RecordErc20BalanceFactState, RecordEvmNativeBalanceFactState,
    RedactedEvmSessionEvidence, ResolveEvmJointTipConfig, ResolveEvmJointTipInput,
    ResolveEvmJointTipState, EVM_JOINT_TIP_SOURCE_READS,
};
use mfm_store::v1 as store;
use mfm_values::{MfmValue, NonEmpty};
use serde::{Deserialize, Serialize};

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";

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
    source: RedactedEvmSessionEvidence,
}

impl EvmJointTipReadEvidence {
    fn from_session(
        selector: &EvmBlockSelector,
        block: &EvmBlock,
        source: &EvmSessionEvidence,
    ) -> Result<Self> {
        Ok(Self {
            request_block_selector: evm_block_selector_text(selector),
            response_block_number: block_number_u64(block)?,
            response_block_hash: format!("{:#x}", block.hash),
            source: source_binding_from_capability(source)?,
        })
    }

    fn replay_material(&self) -> Result<(EvmBlock, EvmSessionEvidence)> {
        if self.request_block_selector != "latest" {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        Ok((
            EvmBlock {
                number: U256::from(self.response_block_number),
                hash: parse_canonical_hash(&self.response_block_hash)?,
            },
            capability_source_from_binding(&self.source)?,
        ))
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
    anchor_block_hash: String,
    balance_response_wei: String,
    verification_response_block_number: u64,
    verification_response_block_hash: String,
    source: RedactedEvmSessionEvidence,
}

impl EvmNativeBalanceReadEvidence {
    fn from_session(
        account: Address,
        anchor_block_hash: B256,
        balance: U256,
        verification: &EvmBlock,
        source: &EvmSessionEvidence,
    ) -> Result<Self> {
        Ok(Self {
            balance_request_account: format!("{account:#x}"),
            anchor_block_hash: format!("{anchor_block_hash:#x}"),
            balance_response_wei: balance.to_string(),
            verification_response_block_number: block_number_u64(verification)?,
            verification_response_block_hash: format!("{:#x}", verification.hash),
            source: source_binding_from_capability(source)?,
        })
    }

    fn replay_material(&self) -> Result<(Address, B256, U256, EvmBlock, EvmSessionEvidence)> {
        let account = parse_account(&self.balance_request_account)?;
        if format!("{account:#x}") != self.balance_request_account {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let balance = U256::from_str(&self.balance_response_wei)
            .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
        Ok((
            account,
            parse_canonical_hash(&self.anchor_block_hash)?,
            balance,
            EvmBlock {
                number: U256::from(self.verification_response_block_number),
                hash: parse_canonical_hash(&self.verification_response_block_hash)?,
            },
            capability_source_from_binding(&self.source)?,
        ))
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
    anchor_block_hash: String,
    return_data: String,
    source: RedactedEvmSessionEvidence,
    verification_response_block_number: u64,
    verification_response_block_hash: String,
}

impl EvmErc20CallReadEvidence {
    fn from_session(
        request: &EvmCall,
        response: &Bytes,
        verification: &EvmBlock,
        source: &EvmSessionEvidence,
    ) -> Result<Self> {
        let EvmBlockSelector::ExactHash(anchor_block_hash) = request.block() else {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        };
        Ok(Self {
            destination: format!("{:#x}", request.to()),
            calldata: encode_hex(request.input()),
            anchor_block_hash: format!("{anchor_block_hash:#x}"),
            return_data: encode_hex(response),
            source: source_binding_from_capability(source)?,
            verification_response_block_number: block_number_u64(verification)?,
            verification_response_block_hash: format!("{:#x}", verification.hash),
        })
    }

    fn replay_material(&self) -> Result<(EvmCall, Bytes, EvmBlock, EvmSessionEvidence)> {
        let destination = parse_account(&self.destination)?;
        if format!("{destination:#x}") != self.destination {
            return Err(EvmAdapterError::InvalidCapabilityRequest);
        }
        let calldata = decode_hex(&self.calldata)?;
        let return_data = decode_hex(&self.return_data)?;
        let block_hash = parse_canonical_hash(&self.anchor_block_hash)?;
        Ok((
            EvmCall::new(
                destination,
                calldata.into(),
                EvmBlockSelector::ExactHash(block_hash),
            ),
            return_data.into(),
            EvmBlock {
                number: U256::from(self.verification_response_block_number),
                hash: parse_canonical_hash(&self.verification_response_block_hash)?,
            },
            capability_source_from_binding(&self.source)?,
        ))
    }
}

fn block_number_u64(block: &EvmBlock) -> Result<u64> {
    u64::try_from(block.number).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

fn encode_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let body = value
        .strip_prefix("0x")
        .ok_or(EvmAdapterError::InvalidCapabilityRequest)?;
    let bytes = hex::decode(body).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
    if encode_hex(&bytes) != value {
        return Err(EvmAdapterError::InvalidCapabilityRequest);
    }
    Ok(bytes)
}

fn source_binding_from_capability(
    evidence: &EvmSessionEvidence,
) -> Result<RedactedEvmSessionEvidence> {
    RedactedEvmSessionEvidence::from_session(evidence)
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

fn capability_source_from_binding(
    binding: &RedactedEvmSessionEvidence,
) -> Result<EvmSessionEvidence> {
    binding
        .to_session()
        .map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

fn evm_block_selector_text(selector: &EvmBlockSelector) -> String {
    match selector {
        EvmBlockSelector::Latest => "latest".to_owned(),
        EvmBlockSelector::Number(number) => format!("number:{number}"),
        EvmBlockSelector::ExactHash(hash) => format!("hash:{hash:#x}"),
    }
}

/// Result type for EVM adapter operations.
pub type Result<T> = std::result::Result<T, EvmAdapterError>;

/// Future returned by the application-owned asynchronous session binder.
pub type EvmReadSessionBindFuture = Pin<
    Box<
        dyn Future<Output = mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>>> + Send + 'static,
    >,
>;

type ValidateEvmBinding =
    dyn Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync;
type BindEvmReadSession = dyn Fn(EvmNetworkBinding) -> EvmReadSessionBindFuture + Send + Sync;

/// Runtime capabilities used by EVM collector runners.
#[derive(Clone)]
pub struct EvmRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    validate_evm_binding: Arc<ValidateEvmBinding>,
    bind_evm_read_session: Arc<BindEvmReadSession>,
}

impl EvmRunnerCapabilities {
    /// Creates runner capabilities from retained artifacts and direct session bindings.
    pub fn new<V, B>(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        validate_evm_binding: V,
        bind_evm_read_session: B,
    ) -> Self
    where
        V: Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync + 'static,
        B: Fn(EvmNetworkBinding) -> EvmReadSessionBindFuture + Send + Sync + 'static,
    {
        Self {
            artifacts,
            validate_evm_binding: Arc::new(validate_evm_binding),
            bind_evm_read_session: Arc::new(bind_evm_read_session),
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn validate_evm_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        (self.validate_evm_binding)(binding)
    }

    async fn bind_evm_read_session(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>> {
        let session = (self.bind_evm_read_session)(binding.clone()).await?;
        let evidence = session.evidence();
        if !evidence.matches_binding(&binding)
            || evidence.implementation_id().as_str() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            return Err(EvmCapabilityError::provider_failure(
                mfm_evm_capabilities::evm_diagnostic(
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                ),
            ));
        }
        Ok(session)
    }
}

/// Registers typed EVM collector runners.
pub fn register_evm_collectors_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    registry.register_capability_spec::<EvmReadCapability>(CapabilityImplementationId::new(
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
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
            capabilities: capabilities.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveEvmNativeBalanceState>(
        &read_factory,
        Arc::new(ObserveNativeBalanceRunner {
            artifacts: artifacts.clone(),
            capabilities: capabilities.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20TokenMetadataState>(
        &read_factory,
        Arc::new(ObserveErc20TokenMetadataRunner {
            artifacts: artifacts.clone(),
            capabilities: capabilities.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20BalanceState>(
        &read_factory,
        Arc::new(ObserveErc20BalanceRunner {
            artifacts: artifacts.clone(),
            capabilities,
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
    registrations.register_state_runner_with_factory::<AssembleEvmNativeBalanceBatchReceiptState>(
        &pure_factory,
        Arc::new(AssembleNativeBalanceReceiptRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<AssembleEvmErc20BalanceBatchReceiptState>(
        &pure_factory,
        Arc::new(AssembleErc20BalanceReceiptRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<AssembleEvmNetworkCollectionReceiptState>(
        &pure_factory,
        Arc::new(AssembleNetworkCollectionReceiptRunner { artifacts }),
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
        LocalPublicId::new(network).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)?;
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

fn parse_canonical_hash(value: &str) -> Result<B256> {
    let hash = parse_block_hash(value)?;
    if format!("{hash:#x}") != value {
        return Err(EvmAdapterError::InvalidCapabilityRequest);
    }
    Ok(hash)
}

fn parse_account(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|_| EvmAdapterError::InvalidCapabilityRequest)
}

struct ResolveJointTipRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    capabilities: EvmRunnerCapabilities,
}

impl ErasedNodeRunner for ResolveJointTipRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ResolveEvmJointTipConfig>(&ctx, ctx.node())?;
        let binding = joint_tip_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
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
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let selector = EvmBlockSelector::Latest;
            let block = session
                .read_block(&selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let tip = state
                .materialize_response(&block, session.evidence())
                .map_err(evm_state_runtime_error)?;
            let evidence =
                EvmJointTipReadEvidence::from_session(&selector, &block, session.evidence())
                    .map_err(evm_adapter_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&evidence)?;
            output.state_output(&tip)?;
            Ok(output.finish())
        })
    }
}

struct ObserveNativeBalanceRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    capabilities: EvmRunnerCapabilities,
}

impl ErasedNodeRunner for ObserveNativeBalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config =
            load_launch_config_for_node::<ObserveEvmNativeBalanceConfig>(&ctx, ctx.node())?;
        let binding = balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
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
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let balance_selector = EvmBlockSelector::ExactHash(tip_hash);
            let balance = session
                .read_balance(account, &balance_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification_selector =
                EvmBlockSelector::Number(U256::from(input.joint_tip.block_number()));
            let verified_block = session
                .read_block(&verification_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verified_tip = materialize_evm_joint_tip(
                &joint_tip_config_for_observation(state.config())
                    .map_err(evm_adapter_runtime_error)?,
                &verified_block,
                session.evidence(),
            )
            .map_err(evm_state_runtime_error)?;
            // Explicit tip equality check (number must match joint tip as well).
            if verified_tip.block_number() != input.joint_tip.block_number()
                || verified_tip.block_hash() != input.joint_tip.block_hash()
                || verified_tip.source_binding() != input.joint_tip.source_binding()
            {
                return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "tip drift, hash, or provider-source mismatch before Platform write".to_owned(),
                ));
            }
            let observation = state
                .materialize_response(&input, &verified_tip, &balance, session.evidence())
                .map_err(evm_state_runtime_error)?;
            let evidence = EvmNativeBalanceReadEvidence::from_session(
                account,
                tip_hash,
                balance,
                &verified_block,
                session.evidence(),
            )
            .map_err(evm_adapter_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&evidence)?;
            output.state_output(&observation)?;
            Ok(output.finish())
        })
    }
}

struct ObserveErc20TokenMetadataRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    capabilities: EvmRunnerCapabilities,
}

impl ErasedNodeRunner for ObserveErc20TokenMetadataRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config =
            load_launch_config_for_node::<ObserveErc20TokenMetadataConfig>(&ctx, ctx.node())?;
        let binding = erc20_metadata_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
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
            let verification_selector =
                EvmBlockSelector::Number(U256::from(input.joint_tip.block_number()));
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let response = session
                .call(&request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification = session
                .read_block(&verification_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let metadata = state
                .materialize_response(
                    &input,
                    &request,
                    &response,
                    &verification,
                    session.evidence(),
                )
                .map_err(evm_state_runtime_error)?;
            let evidence = EvmErc20CallReadEvidence::from_session(
                &request,
                &response,
                &verification,
                session.evidence(),
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
    capabilities: EvmRunnerCapabilities,
}

impl ErasedNodeRunner for ObserveErc20BalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ObserveErc20BalanceConfig>(&ctx, ctx.node())?;
        let binding = erc20_balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
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
            let verification_selector =
                EvmBlockSelector::Number(U256::from(input.joint_tip.block_number()));
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let response = session
                .call(&request)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification = session
                .read_block(&verification_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let observation = state
                .materialize_response(
                    &input,
                    &request,
                    &response,
                    &verification,
                    session.evidence(),
                )
                .map_err(evm_state_runtime_error)?;
            let evidence = EvmErc20CallReadEvidence::from_session(
                &request,
                &response,
                &verification,
                session.evidence(),
            )
            .map_err(evm_adapter_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&evidence)?;
            output.state_output(&observation)?;
            Ok(output.finish())
        })
    }
}

struct AssembleNativeBalanceReceiptRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleNativeBalanceReceiptRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config =
                load_runner_config_for_node::<AssembleEvmNativeBalanceBatchReceiptConfig>(
                    ctx.node(),
                    self.artifacts.as_ref(),
                )
                .await?;
            let input =
                load_materialized_struct_input::<AssembleEvmNativeBalanceBatchReceiptInput>(
                    ctx.inputs(),
                    self.artifacts.as_ref(),
                )
                .await?;
            let receipt = assemble_evm_native_balance_batch_receipt(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &receipt)
        })
    }
}

struct AssembleErc20BalanceReceiptRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleErc20BalanceReceiptRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<AssembleEvmErc20BalanceBatchReceiptConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleEvmErc20BalanceBatchReceiptInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let receipt = assemble_evm_erc20_balance_batch_receipt(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &receipt)
        })
    }
}

struct AssembleNetworkCollectionReceiptRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleNetworkCollectionReceiptRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<AssembleEvmNetworkCollectionReceiptConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleEvmNetworkCollectionReceiptInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let receipt = assemble_evm_network_collection_receipt(config.as_ref(), input).map_err(
                |error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()),
            )?;
            ErasedRunnerOutput::state_output(&ctx, &receipt)
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
    let Some(diagnostic) = error.redacted_diagnostic().cloned() else {
        return mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM capability request failed without provider diagnostics".to_owned(),
        );
    };
    let (code, message) = match diagnostic.code() {
        ProviderDiagnosticCode::ProviderConfigurationMissing
        | ProviderDiagnosticCode::RouteUnavailable => (
            "RuntimeConfigRequired",
            "EVM runtime configuration is required",
        ),
        ProviderDiagnosticCode::ProviderConfigurationInvalid => (
            "RuntimeConfigInvalid",
            "EVM runtime configuration is invalid",
        ),
        _ => ("EvmProviderFailure", "EVM provider capability failed"),
    };
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("EVM runtime failure code is checked public text"),
        events::ErrorCategory::Capability,
        message,
        vec![diagnostic],
    )
    .expect("EVM runtime failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
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
    verify_evm_native_balance_receipt_replay(broker)?;
    verify_evm_erc20_balance_receipt_replay(broker)?;
    verify_evm_network_collection_receipt_replay(broker)?;
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
    let mut anchors = BTreeMap::<String, (u64, String, RedactedEvmSessionEvidence)>::new();
    for frame in native_frames {
        let config: ObserveEvmNativeBalanceConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (
            tip.block_number(),
            tip.block_hash().to_owned(),
            tip.source_binding().clone(),
        );
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
        let anchor = (
            tip.block_number(),
            tip.block_hash().to_owned(),
            tip.source_binding().clone(),
        );
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
        let anchor = (
            tip.block_number(),
            tip.block_hash().to_owned(),
            tip.source_binding().clone(),
        );
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
        let (block, source) = evidence.replay_material().map_err(replay_adapter_error)?;
        if !evidence
            .source
            .is_bound_to(&config.network, config.chain_id)
        {
            return Err(replay_evm_mismatch(
                "EVM joint-tip read evidence did not match certified request or source binding",
            ));
        }
        let expected =
            materialize_evm_joint_tip(&config, &block, &source).map_err(replay_adapter_error)?;
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
    let (account, anchor_hash, balance, verification, source) =
        evidence.replay_material().map_err(replay_adapter_error)?;
    let expected_account = parse_account(&config.account).map_err(replay_adapter_error)?;
    let expected_anchor = parse_block_hash(input_tip.block_hash()).map_err(replay_adapter_error)?;
    let verified_tip = materialize_evm_joint_tip(
        &joint_tip_config_for_observation(&config).map_err(replay_adapter_error)?,
        &verification,
        &source,
    )
    .map_err(replay_adapter_error)?;
    if account != expected_account
        || anchor_hash != expected_anchor
        || verified_tip != input_tip
        || evidence.source != *input_tip.source_binding()
    {
        return Err(replay_evm_mismatch(
            "EVM native-balance read evidence did not match certified requests or source binding",
        ));
    }
    let expected = mfm_states_evm::normalize_evm_native_balance_from_capability(
        &config,
        &input_tip,
        &verified_tip,
        &balance,
        &source,
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
    let (request, response, verification, source) = replay_erc20_call_evidence(broker, frame)?;
    if !joint_tip.source_binding().matches_session(&source) {
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
        &source,
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
    let (request, response, verification, source) = replay_erc20_call_evidence(broker, frame)?;
    if !input.joint_tip.source_binding().matches_session(&source) {
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
        &source,
    )
    .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn replay_erc20_call_evidence(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<(EvmCall, Bytes, EvmBlock, EvmSessionEvidence)> {
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
        replay::verify_recorded_fact_evidence(
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
        replay::verify_recorded_fact_evidence(
            broker,
            frame,
            &EvmAddressErc20BalanceSnapshotFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_evm_native_balance_receipt_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind =
        AssembleEvmNativeBalanceBatchReceiptState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleEvmNativeBalanceBatchReceiptState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let _config: AssembleEvmNativeBalanceBatchReceiptConfig =
            replay_node_config(broker, &frame.node)?;
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
                "EVM native-balance receipt inputs were incomplete",
            ));
        }
        let joint_tip: EvmJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let facts = fact_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<EvmAddressNativeBalanceSnapshotFact>>>()?;
        let input = AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip,
            balance_facts: NonEmpty::try_from_vec(facts).map_err(replay_adapter_error)?,
        };
        let expected =
            assemble_evm_native_balance_batch_receipt(input).map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn verify_evm_erc20_balance_receipt_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind =
        AssembleEvmErc20BalanceBatchReceiptState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleEvmErc20BalanceBatchReceiptState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let _config: AssembleEvmErc20BalanceBatchReceiptConfig =
            replay_node_config(broker, &frame.node)?;
        let joint_tip_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmJointTip::semantic_id().map_err(replay_adapter_error)?,
            &EvmJointTip::schema_id().map_err(replay_adapter_error)?,
        )?;
        let fact_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmAddressErc20BalanceSnapshotFact::semantic_id().map_err(replay_adapter_error)?,
            &EvmAddressErc20BalanceSnapshotFact::schema_id().map_err(replay_adapter_error)?,
        )?;
        if joint_tip_frames.len() != 1 || fact_frames.is_empty() {
            return Err(replay_evm_mismatch(
                "EVM ERC-20 balance receipt inputs were incomplete",
            ));
        }
        let joint_tip: EvmJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let facts = fact_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<EvmAddressErc20BalanceSnapshotFact>>>()?;
        let input = AssembleEvmErc20BalanceBatchReceiptInput {
            joint_tip,
            balance_facts: NonEmpty::try_from_vec(facts).map_err(replay_adapter_error)?,
        };
        let expected =
            assemble_evm_erc20_balance_batch_receipt(input).map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn verify_evm_network_collection_receipt_replay(
    broker: &replay::ReplayBroker,
) -> replay::Result<()> {
    let state_kind =
        AssembleEvmNetworkCollectionReceiptState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleEvmNetworkCollectionReceiptState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let config: AssembleEvmNetworkCollectionReceiptConfig =
            replay_node_config(broker, &frame.node)?;
        let joint_tip_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmJointTip::semantic_id().map_err(replay_adapter_error)?,
            &EvmJointTip::schema_id().map_err(replay_adapter_error)?,
        )?;
        let native_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmNativeBalanceBatchReceipt::semantic_id().map_err(replay_adapter_error)?,
            &EvmNativeBalanceBatchReceipt::schema_id().map_err(replay_adapter_error)?,
        )?;
        let erc20_frames = replay_input_frames(
            broker,
            &frame.node,
            &EvmErc20BalanceBatchReceipt::semantic_id().map_err(replay_adapter_error)?,
            &EvmErc20BalanceBatchReceipt::schema_id().map_err(replay_adapter_error)?,
        )?;
        if joint_tip_frames.len() != 1 {
            return Err(replay_evm_mismatch(
                "EVM network collection receipt did not consume exactly one joint tip",
            ));
        }
        let joint_tip: EvmJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let native_balance_receipts = native_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<EvmNativeBalanceBatchReceipt>>>(
        )?;
        let erc20_balance_receipts = erc20_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<EvmErc20BalanceBatchReceipt>>>()?;
        let input = AssembleEvmNetworkCollectionReceiptInput {
            joint_tip,
            native_balance_receipts,
            erc20_balance_receipts,
        };
        let expected = assemble_evm_network_collection_receipt(&config, input)
            .map_err(replay_adapter_error)?;
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

fn ensure_canonical_value_matches<T: serde::Serialize>(
    expected: &T,
    actual: &[u8],
) -> replay::Result<()> {
    if replay::canonical_value_bytes(expected)?.as_bytes() != actual {
        return Err(replay_evm_mismatch(
            "EVM collector output did not match recomputed state output",
        ));
    }
    Ok(())
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

    fn source_evidence() -> EvmSessionEvidence {
        source_evidence_for("primary", "default")
    }

    fn source_evidence_for(source_ref: &str, implementation_id: &str) -> EvmSessionEvidence {
        let binding =
            EvmNetworkBinding::new(LocalPublicId::new("ethereum-mainnet").expect("net"), 1)
                .expect("binding");
        EvmSessionEvidence::new(
            &binding,
            LocalPublicId::new(source_ref).expect("source"),
            LocalPublicId::new(implementation_id).expect("implementation"),
        )
    }

    fn hash() -> B256 {
        B256::from_str(HASH.strip_prefix("0x").expect("prefix")).expect("hash")
    }

    fn evidence() -> EvmErc20CallReadEvidence {
        let request = EvmCall::new(
            Address::from_str(TOKEN).expect("token"),
            vec![0x31, 0x3c, 0xe5, 0x67].into(),
            EvmBlockSelector::ExactHash(hash()),
        );
        let response: Bytes = vec![0; 32].into();
        let verification = EvmBlock {
            number: U256::from(100),
            hash: hash(),
        };
        EvmErc20CallReadEvidence::from_session(
            &request,
            &response,
            &verification,
            &source_evidence(),
        )
        .expect("evidence")
    }

    #[test]
    fn erc20_evidence_retains_exact_hash_bound_call_and_redacted_source() {
        let evidence = evidence();
        assert_eq!(evidence.destination, TOKEN);
        assert_eq!(evidence.calldata, "0x313ce567");
        assert_eq!(evidence.anchor_block_hash, HASH);
        assert_eq!(evidence.return_data, format!("0x{}", "00".repeat(32)));
        assert_eq!(evidence.verification_response_block_number, 100);
        assert_eq!(evidence.verification_response_block_hash, HASH);
        assert_eq!(evidence.source.network(), "ethereum-mainnet");
        assert_eq!(evidence.source.chain_id(), 1);
        assert_eq!(evidence.source.source_ref(), "primary");
        assert_eq!(evidence.source.implementation_id(), "default");
        let value = serde_json::to_value(&evidence).expect("JSON value");
        assert_eq!(value["source"]["network"], "ethereum-mainnet");
        assert_eq!(value["source"]["chain_id"], 1);
        assert!(value["source"].get("observed_chain_id").is_none());
        let rendered = serde_json::to_string(&evidence).expect("json");
        for forbidden in ["http://", "https://", "authorization", "password"] {
            assert!(!rendered.contains(forbidden), "leaked {forbidden}");
        }

        let (request, response, verification, source) =
            evidence.replay_material().expect("replay material");
        assert_eq!(format!("{:#x}", request.to()), TOKEN);
        assert_eq!(request.input().as_ref(), &[0x31, 0x3c, 0xe5, 0x67]);
        assert!(
            matches!(request.block(), EvmBlockSelector::ExactHash(value) if format!("{value:#x}") == HASH)
        );
        assert_eq!(response.as_ref(), &[0; 32]);
        assert_eq!(verification.number, U256::from(100));
        assert_eq!(format!("{:#x}", verification.hash), HASH);
        assert_eq!(source, source_evidence());
    }

    #[test]
    fn erc20_evidence_rejects_noncanonical_or_noncanonicality_tampering() {
        let mut noncanonical_hash = evidence();
        noncanonical_hash.anchor_block_hash = HASH.to_uppercase();
        assert!(noncanonical_hash.replay_material().is_err());

        let mut noncanonical_hex = evidence();
        noncanonical_hex.calldata = "0x313CE567".to_owned();
        assert!(noncanonical_hex.replay_material().is_err());

        let mut malformed_verification = evidence();
        malformed_verification.verification_response_block_hash = "0x".to_owned();
        assert!(malformed_verification.replay_material().is_err());
    }

    #[test]
    fn erc20_evidence_has_one_checked_session_provenance() {
        let persisted = serde_json::to_value(evidence()).expect("persisted evidence");
        assert!(persisted.get("source").is_some());
        assert!(persisted.get("verification_source").is_none());
        assert!(persisted.get("policy_id").is_none());

        let mut malformed = persisted;
        malformed["source"]["source_ref"] = serde_json::json!("INVALID SOURCE");
        assert!(serde_json::from_value::<EvmErc20CallReadEvidence>(malformed).is_err());
    }
}
