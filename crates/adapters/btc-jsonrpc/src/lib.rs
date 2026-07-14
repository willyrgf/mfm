#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! this crate owns request mapping, runner registration, fact recording, and recorded provider
//! backends over replay evidence.

use std::marker::PhantomData;
use std::sync::Arc;

#[path = "replay.rs"]
mod replay_verification;
pub use self::replay_verification::{
    replay_loaded_checkpoint_from_evidence, verify_btc_jsonrpc_replay,
};

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcAddress, BtcBalanceReadCapability, BtcBalanceReadProvider,
    BtcBalanceReadRequest, BtcBalanceReadResponse, BtcBlockHash, BtcCapabilityError,
    BtcCapabilityFuture, BtcChainHeadReadCapability, BtcChainHeadReadProvider, BtcChainHeadRequest,
    BtcChainHeadResponse, BtcNetworkId, BtcSourceBinding, BtcSourceIdentity, BtcSourceStatus,
    RedactedBtcSourceEvidence,
};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{FactIndexReadProvider, FactRecordCapability};
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    load_launch_config_for_node, load_materialized_struct_input, load_runner_config_for_node,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_btc::{
    address_balance_record_visibility, assemble_btc_address_balance_batch,
    btc_jsonrpc_adapter_kind, btc_jsonrpc_adapter_version, chain_head_fact_visibility,
    collector_checkpoint_fact_visibility, normalize_btc_address_balance_observation,
    record_collector_checkpoint_from_outputs, AssembleBtcAddressBalanceBatchConfig,
    AssembleBtcAddressBalanceBatchInput, AssembleBtcAddressBalanceBatchState,
    BtcAddressBalanceObservation, BtcAddressBalanceSnapshotFact, BtcChainHeadFact,
    BtcChainHeadObservation, BtcJointTip, CollectorCheckpointFact, CollectorCheckpointResponse,
    LoadedCollectorCheckpoint, ObserveBtcAddressBalanceConfig, ObserveBtcAddressBalanceInput,
    ObserveBtcAddressBalanceState, ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput,
    ObserveBtcChainHeadState, QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointState, RecordBtcAddressBalanceFactState, RecordBtcChainHeadFactState,
    RecordCollectorCheckpointState, ResolveBtcJointTipConfig, ResolveBtcJointTipInput,
    ResolveBtcJointTipState,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::{Deserialize, Serialize};

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "btc_jsonrpc_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.bitcoin.jsonrpc.runtime.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin.jsonrpc",
    name = "chain_head_read_evidence",
    version = "1",
    schema = "mfm.bitcoin.jsonrpc.external_read.chain_head"
)]
struct BtcChainHeadReadEvidence {
    request_head_kind: String,
    request_finality: String,
    request_confirmation_depth: Option<u64>,
    response_head_kind: String,
    response_finality: String,
    response_confirmation_depth: Option<u64>,
    block_height: u64,
    block_hash: String,
    provider_time_unix_ms: Option<u64>,
    network_id: String,
    source_identity: String,
    bitcoin_network: String,
    observed_bitcoin_network: String,
    source_status: String,
}

impl BtcChainHeadReadEvidence {
    fn from_capability(request: &BtcChainHeadRequest, response: &BtcChainHeadResponse) -> Self {
        let request_selection = request.selection();
        Self {
            request_head_kind: request_selection.head_kind().as_str().to_owned(),
            request_finality: request_selection.finality().as_str().to_owned(),
            request_confirmation_depth: request_selection.finality().confirmation_depth(),
            response_head_kind: response.head_kind.as_str().to_owned(),
            response_finality: response.finality.as_str().to_owned(),
            response_confirmation_depth: response.finality.confirmation_depth(),
            block_height: response.block_height,
            block_hash: response.block_hash.as_str().to_owned(),
            provider_time_unix_ms: response.provider_time_unix_ms,
            network_id: response.evidence.network_id.as_str().to_owned(),
            source_identity: response.evidence.source_identity.as_str().to_owned(),
            bitcoin_network: response.evidence.bitcoin_network.clone(),
            observed_bitcoin_network: response.evidence.observed_bitcoin_network.clone(),
            source_status: response.evidence.source_status.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin.jsonrpc",
    name = "balance_read_evidence",
    version = "1",
    schema = "mfm.bitcoin.jsonrpc.external_read.balance"
)]
struct BtcBalanceReadEvidence {
    request_address: String,
    request_block_height: u64,
    request_block_hash: String,
    response_address: String,
    response_balance_sats: u64,
    response_block_height: u64,
    response_block_hash: String,
    network_id: String,
    source_identity: String,
    bitcoin_network: String,
    observed_bitcoin_network: String,
    source_status: String,
}

impl BtcBalanceReadEvidence {
    fn from_capability(request: &BtcBalanceReadRequest, response: &BtcBalanceReadResponse) -> Self {
        Self {
            request_address: request.address().as_str().to_owned(),
            request_block_height: request.block_height(),
            request_block_hash: request.block_hash().as_str().to_owned(),
            response_address: response.address.as_str().to_owned(),
            response_balance_sats: response.balance_sats,
            response_block_height: response.block_height,
            response_block_hash: response.block_hash.as_str().to_owned(),
            network_id: response.evidence.network_id.as_str().to_owned(),
            source_identity: response.evidence.source_identity.as_str().to_owned(),
            bitcoin_network: response.evidence.bitcoin_network.clone(),
            observed_bitcoin_network: response.evidence.observed_bitcoin_network.clone(),
            source_status: response.evidence.source_status.as_str().to_owned(),
        }
    }
}

/// Result type for Bitcoin JSON-RPC adapter operations.
pub type Result<T> = std::result::Result<T, BtcJsonRpcAdapterError>;

/// Factory for Bitcoin chain-head and balance providers bound to a certified source binding.
pub trait BtcChainHeadProviderFactory: Send + Sync {
    /// Validates that the binding can resolve without live network IO.
    fn validate_source_binding(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()>;

    /// Binds a checked source binding to a chain-head provider.
    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>>;

    /// Binds a checked source binding to an address-balance provider.
    fn bind_balance_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcBalanceReadProvider>>;
}

/// Runtime capabilities used by Bitcoin JSON-RPC adapter runners.
#[derive(Clone)]
pub struct BtcJsonRpcRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadProviderFactory>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl BtcJsonRpcRunnerCapabilities {
    /// Creates runner capabilities from artifact and Bitcoin provider factory.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        btc: Arc<dyn BtcChainHeadProviderFactory>,
        fact_index: Arc<dyn FactIndexReadProvider>,
    ) -> Self {
        Self {
            artifacts,
            btc,
            fact_index,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn btc(&self) -> Arc<dyn BtcChainHeadProviderFactory> {
        Arc::clone(&self.btc)
    }

    fn fact_index(&self) -> Arc<dyn FactIndexReadProvider> {
        Arc::clone(&self.fact_index)
    }
}

/// Registers typed Bitcoin JSON-RPC runners.
pub fn register_btc_jsonrpc_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: BtcJsonRpcRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    let btc = capabilities.btc();
    let fact_index = capabilities.fact_index();
    registry.register_capability_spec::<BtcChainHeadReadCapability>(
        CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?,
    )?;
    registry.register_capability_spec::<BtcBalanceReadCapability>(
        CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?,
    )?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-btc-jsonrpc",
        "typed-bitcoin-jsonrpc",
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
        btc_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        btc_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<ObserveBtcChainHeadState>(
        &read_factory,
        Arc::new(ObserveChainHeadRunner {
            artifacts: artifacts.clone(),
            btc: btc.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ResolveBtcJointTipState>(
        &read_factory,
        Arc::new(ResolveJointTipRunner {
            artifacts: artifacts.clone(),
            btc: btc.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ObserveBtcAddressBalanceState>(
        &read_factory,
        Arc::new(ObserveAddressBalanceRunner {
            artifacts: artifacts.clone(),
            btc,
        }),
    )?;
    registrations.register_state_runner_with_factory::<RecordBtcChainHeadFactState>(
        &managed_write_factory,
        Arc::new(ManagedFactRecordRunner::<RecordBtcChainHeadFactState>::new(
            artifacts.clone(),
            chain_head_fact_visibility(),
        )),
    )?;
    registrations.register_state_runner_with_factory::<RecordBtcAddressBalanceFactState>(
        &managed_write_factory,
        Arc::new(
            ManagedFactRecordRunner::<RecordBtcAddressBalanceFactState>::new(
                artifacts.clone(),
                address_balance_record_visibility(),
            ),
        ),
    )?;
    registrations.register_state_runner_with_factory::<QueryCollectorCheckpointState>(
        &read_factory,
        Arc::new(QueryCheckpointRunner {
            artifacts: artifacts.clone(),
            fact_index,
        }),
    )?;
    registrations.register_state_runner_with_factory::<RecordCollectorCheckpointState>(
        &managed_write_factory,
        Arc::new(
            ManagedFactRecordRunner::<RecordCollectorCheckpointState>::new(
                artifacts.clone(),
                collector_checkpoint_fact_visibility(),
            ),
        ),
    )?;
    registrations.register_state_runner_with_factory::<AssembleBtcAddressBalanceBatchState>(
        &pure_factory,
        Arc::new(AssembleAddressBalanceBatchRunner { artifacts }),
    )?;
    Ok(())
}

struct AssembleAddressBalanceBatchRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleAddressBalanceBatchRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<AssembleBtcAddressBalanceBatchConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleBtcAddressBalanceBatchInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let summary = assemble_btc_address_balance_batch(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &summary)
        })
    }
}

/// Recorded Bitcoin chain-head provider backend for replay.
#[derive(Debug, Clone)]
pub struct RecordedBtcChainHeadProvider {
    binding: BtcSourceBinding,
    response: BtcChainHeadResponse,
}

impl RecordedBtcChainHeadProvider {
    /// Creates a recorded provider bound to certified source binding and captured response.
    pub const fn new(binding: BtcSourceBinding, response: BtcChainHeadResponse) -> Self {
        Self { binding, response }
    }
}

impl BtcChainHeadReadProvider for RecordedBtcChainHeadProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(
            async move { recorded_chain_head_response(&self.binding, request, &self.response) },
        )
    }
}

/// Recorded Bitcoin address-balance provider backend for replay.
#[derive(Debug, Clone)]
pub struct RecordedBtcBalanceProvider {
    binding: BtcSourceBinding,
    response: BtcBalanceReadResponse,
}

impl RecordedBtcBalanceProvider {
    /// Creates a recorded balance provider bound to certified source binding and response.
    pub const fn new(binding: BtcSourceBinding, response: BtcBalanceReadResponse) -> Self {
        Self { binding, response }
    }
}

impl BtcBalanceReadProvider for RecordedBtcBalanceProvider {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        Box::pin(async move { recorded_balance_response(&self.binding, request, &self.response) })
    }
}

fn recorded_balance_response(
    binding: &BtcSourceBinding,
    request: &BtcBalanceReadRequest,
    response: &BtcBalanceReadResponse,
) -> mfm_btc_capabilities::Result<BtcBalanceReadResponse> {
    let evidence = &response.evidence;
    if &evidence.network_id == binding.network_id()
        && &evidence.source_identity == binding.source_identity()
        && evidence.bitcoin_network == binding.bitcoin_network().as_str()
        && evidence.observed_bitcoin_network == binding.bitcoin_network().as_str()
        && response.address.as_str() == request.address().as_str()
        && response.block_height == request.block_height()
        && response.block_hash.as_str() == request.block_hash().as_str()
    {
        Ok(response.clone())
    } else {
        Err(BtcCapabilityError::SourceMismatch {
            diagnostic: evidence.source_mismatch_diagnostic(),
        })
    }
}

/// Redaction-safe Bitcoin JSON-RPC adapter error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcJsonRpcAdapterError {
    /// Adapter could not build a capability request from certified state config.
    #[error("Bitcoin adapter could not build capability request")]
    InvalidCapabilityRequest,
    /// Replay evidence did not match the certified request.
    #[error("Bitcoin replay evidence did not match request")]
    ReplayEvidenceMismatch,
}

fn chain_head_binding(config: &ObserveBtcChainHeadConfig) -> Result<BtcSourceBinding> {
    source_binding_from_parts(
        &config.network,
        &config.bitcoin_network,
        &config.semantic_source_identity,
    )
}

fn joint_tip_binding(config: &ResolveBtcJointTipConfig) -> Result<BtcSourceBinding> {
    source_binding_from_parts(
        &config.network,
        &config.bitcoin_network,
        &config.semantic_source_identity,
    )
}

fn address_balance_binding(config: &ObserveBtcAddressBalanceConfig) -> Result<BtcSourceBinding> {
    source_binding_from_parts(
        &config.network,
        &config.bitcoin_network,
        &config.semantic_source_identity,
    )
}

fn source_binding_from_parts(
    network: &str,
    bitcoin_network: &str,
    semantic_source_identity: &str,
) -> Result<BtcSourceBinding> {
    let network_id =
        BtcNetworkId::new(network).map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    let source_identity = BtcSourceIdentity::new(semantic_source_identity)
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    let bitcoin_network = BitcoinNetworkTag::new(bitcoin_network)
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    Ok(BtcSourceBinding::new(
        network_id,
        source_identity,
        bitcoin_network,
    ))
}

fn chain_head_request(config: &ObserveBtcChainHeadConfig) -> Result<BtcChainHeadRequest> {
    let selection = config
        .selection()
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    Ok(BtcChainHeadRequest::new(selection))
}

fn joint_tip_request(config: &ResolveBtcJointTipConfig) -> BtcChainHeadRequest {
    BtcChainHeadRequest::new(config.selection())
}

fn address_balance_request(
    config: &ObserveBtcAddressBalanceConfig,
    joint_tip: &BtcJointTip,
) -> Result<BtcBalanceReadRequest> {
    let address = BtcAddress::new(&config.address)
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    let block_hash = BtcBlockHash::new(joint_tip.block_hash())
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    Ok(BtcBalanceReadRequest::new(
        address,
        joint_tip.block_height(),
        block_hash,
    ))
}

fn recorded_chain_head_response(
    binding: &BtcSourceBinding,
    request: &BtcChainHeadRequest,
    response: &BtcChainHeadResponse,
) -> mfm_btc_capabilities::Result<BtcChainHeadResponse> {
    let evidence = &response.evidence;
    if &evidence.network_id == binding.network_id()
        && &evidence.source_identity == binding.source_identity()
        && evidence.bitcoin_network == binding.bitcoin_network().as_str()
        && evidence.observed_bitcoin_network == binding.bitcoin_network().as_str()
        && response.head_kind == request.selection().head_kind()
        && response.finality == request.selection().finality()
    {
        Ok(response.clone())
    } else {
        Err(BtcCapabilityError::SourceMismatch {
            diagnostic: evidence.source_mismatch_diagnostic(),
        })
    }
}

struct ObserveChainHeadRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ErasedNodeRunner for ObserveChainHeadRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ObserveBtcChainHeadConfig>(&ctx, ctx.node())?;
        let binding = chain_head_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ObserveBtcChainHeadConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding = chain_head_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
            let request = chain_head_request(config.as_ref()).map_err(btc_adapter_runtime_error)?;
            let state = ObserveBtcChainHeadState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveBtcChainHeadInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let btc = self
                .btc
                .bind_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_chain_head(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            let fact = state
                .materialize_response(&input, &response)
                .map_err(btc_state_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&BtcChainHeadReadEvidence::from_capability(
                &request, &response,
            ))?;
            output.state_output(&fact)?;
            Ok(output.finish())
        })
    }
}

struct ResolveJointTipRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ErasedNodeRunner for ResolveJointTipRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config_for_node::<ResolveBtcJointTipConfig>(&ctx, ctx.node())?;
        let binding = joint_tip_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ResolveBtcJointTipConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding = joint_tip_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
            let request = joint_tip_request(config.as_ref());
            let state = ResolveBtcJointTipState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let _input = load_materialized_struct_input::<ResolveBtcJointTipInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let btc = self
                .btc
                .bind_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_chain_head(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            let tip = state
                .materialize_response(&response)
                .map_err(btc_state_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&BtcChainHeadReadEvidence::from_capability(
                &request, &response,
            ))?;
            output.state_output(&tip)?;
            Ok(output.finish())
        })
    }
}

struct ObserveAddressBalanceRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ErasedNodeRunner for ObserveAddressBalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config =
            load_launch_config_for_node::<ObserveBtcAddressBalanceConfig>(&ctx, ctx.node())?;
        let binding =
            address_balance_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<ObserveBtcAddressBalanceConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let binding =
                address_balance_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
            let state = ObserveBtcAddressBalanceState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveBtcAddressBalanceInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let request = address_balance_request(state.config(), &input.joint_tip)
                .map_err(btc_adapter_runtime_error)?;
            let btc = self
                .btc
                .bind_balance_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_balance(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            let observation = state
                .materialize_response(&input, &response)
                .map_err(btc_state_runtime_error)?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(&BtcBalanceReadEvidence::from_capability(
                &request, &response,
            ))?;
            output.state_output(&observation)?;
            Ok(output.finish())
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

struct QueryCheckpointRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ErasedNodeRunner for QueryCheckpointRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<QueryCollectorCheckpointConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<QueryCollectorCheckpointInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let (loaded, evidence) = query_collector_checkpoint(
                config,
                input,
                self.artifacts.as_ref(),
                self.fact_index.as_ref(),
            )
            .await?;
            checkpoint_query_output(ctx, &loaded, &evidence)
        })
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
            run_managed_fact_record::<S>(ctx, self.artifacts.as_ref(), self.visibility.clone())
                .await
        })
    }
}

async fn run_managed_fact_record<S>(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    visibility: mfm_program::facts::FactVisibility,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    S: ManagedWriteState<Caps = (FactRecordCapability,)>,
    S::Input: serde::de::DeserializeOwned,
    S::Output: MfmFactType + MfmValue,
{
    let config = load_runner_config_for_node::<S::Config>(ctx.node(), artifacts).await?;
    let state = S::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let input = load_materialized_struct_input::<S::Input>(ctx.inputs(), artifacts).await?;
    let context = ctx.certified_context::<S::Context>()?;
    let fact = state
        .run(input, &(FactRecordCapability,), &context)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output_and_record_fact(
        mfm_runtime::FactRecordInput::new(fact, visibility),
        fact_record_capability_binding()?,
    )?;
    Ok(output.finish())
}

fn checkpoint_query_output(
    ctx: ErasedRunCtx<'_>,
    value: &LoadedCollectorCheckpoint,
    evidence: &mfm_facts::FactQueryEvidence,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output_and_record_fact_query_evidence(value, evidence.clone())?;
    Ok(output.finish())
}

async fn query_collector_checkpoint(
    config: ValidatedConfig<QueryCollectorCheckpointConfig>,
    _input: QueryCollectorCheckpointInput,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(LoadedCollectorCheckpoint, mfm_facts::FactQueryEvidence)> {
    let state = QueryCollectorCheckpointState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state.request().map_err(btc_state_runtime_error)?;
    let response = fact_index
        .read_fact_index(&request)
        .await
        .map_err(fact_index_runtime_error)?;
    let checkpoint = checkpoint_from_response(&state, &response, artifacts).await?;
    let selection = state
        .selection_evidence(&response)
        .map_err(btc_state_runtime_error)?;
    let loaded = state
        .materialize_response(&response, checkpoint)
        .map_err(btc_state_runtime_error)?;
    let evidence = mfm_facts::FactQueryEvidence::new(
        request.plan().clone(),
        response.receipt().clone(),
        selection,
    );
    Ok((loaded, evidence))
}

async fn checkpoint_from_response(
    state: &QueryCollectorCheckpointState,
    response: &mfm_facts::FactQueryResult,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<Option<CollectorCheckpointFact>> {
    match state
        .selected_checkpoint_response_ref(response)
        .map_err(btc_state_runtime_error)?
    {
        None => Ok(None),
        Some(fact_ref) => {
            let requirement = fact_response_artifact_requirement(fact_ref);
            let artifact = artifacts
                .read_retained_artifact(&requirement)
                .await
                .map_err(runtime_artifact_read_error)?;
            let checkpoint_response: CollectorCheckpointResponse =
                hydrate_fact_response_json(fact_ref, artifact.bytes()).map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            state
                .checkpoint_fact_from_response(checkpoint_response)
                .map(Some)
                .map_err(btc_state_runtime_error)
        }
    }
}

fn fact_record_capability_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<FactRecordCapability>(
        btc_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        btc_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
    )
}

fn runtime_artifact_read_error(error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn btc_capability_runtime_error(error: BtcCapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn btc_state_runtime_error(error: mfm_states_btc::BtcStateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn btc_adapter_runtime_error(error: BtcJsonRpcAdapterError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
mod tests;
