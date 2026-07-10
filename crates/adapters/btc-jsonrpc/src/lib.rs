#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! this crate owns request mapping, runner registration, fact recording, and recorded provider
//! backends over replay evidence.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcAddress, BtcBalanceReadCapability, BtcBalanceReadProvider,
    BtcBalanceReadRequest, BtcBalanceReadResponse, BtcBlockHash, BtcCapabilityError,
    BtcCapabilityFuture, BtcChainHeadReadCapability, BtcChainHeadReadProvider, BtcChainHeadRequest,
    BtcChainHeadResponse, BtcNetworkId, BtcSourceBinding, BtcSourceIdentity, BtcSourceStatus,
    RedactedBtcSourceEvidence,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_fact_capabilities::{
    FactIndexReadEvidence, FactIndexReadProvider, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial, FactRecordCapability,
};
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config, load_materialized_struct_input, load_runner_config,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_btc::{
    address_balance_record_visibility, assemble_btc_address_balance_batch,
    btc_jsonrpc_adapter_kind, btc_jsonrpc_adapter_version, chain_head_fact_visibility,
    collector_checkpoint_fact_visibility, normalize_btc_address_balance_observation,
    AssembleBtcAddressBalanceBatchConfig, AssembleBtcAddressBalanceBatchInput,
    AssembleBtcAddressBalanceBatchState, BtcAddressBalanceSnapshotFact, BtcChainHeadObservation,
    BtcJointTip, CollectorCheckpointFact, CollectorCheckpointResponse, LoadedCollectorCheckpoint,
    ObserveBtcAddressBalanceConfig, ObserveBtcAddressBalanceInput, ObserveBtcAddressBalanceState,
    ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput, ObserveBtcChainHeadState,
    QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput, QueryCollectorCheckpointState,
    RecordBtcAddressBalanceFactState, RecordBtcChainHeadFactState, RecordCollectorCheckpointState,
    ResolveBtcJointTipConfig, ResolveBtcJointTipInput, ResolveBtcJointTipState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue, NonEmpty};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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
            let _config = load_runner_config::<AssembleBtcAddressBalanceBatchConfig>(
                &ctx,
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
        let config = load_launch_config::<ObserveBtcChainHeadConfig>(&ctx)?;
        let binding = chain_head_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ObserveBtcChainHeadConfig>(&ctx, self.artifacts.as_ref())
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
        let config = load_launch_config::<ResolveBtcJointTipConfig>(&ctx)?;
        let binding = joint_tip_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveBtcJointTipConfig>(&ctx, self.artifacts.as_ref())
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
        let config = load_launch_config::<ObserveBtcAddressBalanceConfig>(&ctx)?;
        let binding =
            address_balance_binding(config.as_ref()).map_err(btc_adapter_runtime_error)?;
        self.btc
            .validate_source_binding(&binding)
            .map_err(btc_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ObserveBtcAddressBalanceConfig>(&ctx, self.artifacts.as_ref())
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
            let config =
                load_runner_config::<QueryCollectorCheckpointConfig>(&ctx, self.artifacts.as_ref())
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
    let config = load_runner_config::<S::Config>(&ctx, artifacts).await?;
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
    evidence: &FactIndexReadEvidence,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let trust_root = fact_query_trust_root(evidence.trust_root())?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output_and_record_fact_query_evidence(
        value,
        evidence.query_evidence().clone(),
        &trust_root,
    )?;
    Ok(output.finish())
}

async fn query_collector_checkpoint(
    config: ValidatedConfig<QueryCollectorCheckpointConfig>,
    _input: QueryCollectorCheckpointInput,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(LoadedCollectorCheckpoint, FactIndexReadEvidence)> {
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
    let evidence = response
        .into_evidence(&request, selection)
        .map_err(fact_index_runtime_error)?;
    Ok((loaded, evidence))
}

async fn checkpoint_from_response(
    state: &QueryCollectorCheckpointState,
    response: &FactIndexReadResponse,
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

/// Rebuilds loaded checkpoint material from recorded query evidence and retained response bytes.
pub fn replay_loaded_checkpoint_from_evidence(
    config: &QueryCollectorCheckpointConfig,
    evidence: &mfm_facts::FactQueryEvidence,
    response: Option<CollectorCheckpointResponse>,
) -> Result<LoadedCollectorCheckpoint> {
    let request = config
        .request()
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)?;
    if evidence.plan() != request.plan() {
        return Err(BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
    }
    config
        .loaded_checkpoint_from_replay_evidence(evidence, response)
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)
}

/// Verifies Bitcoin JSON-RPC observation cell outputs from retained capability read evidence.
pub fn verify_btc_jsonrpc_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    verify_btc_collector_checkpoint_replay(broker)?;
    verify_btc_joint_tip_replay(broker)?;
    let chain_head_kind = ObserveBtcChainHeadState::kind().map_err(replay_adapter_error)?;
    let chain_head_version = ObserveBtcChainHeadState::version().map_err(replay_adapter_error)?;
    let chain_head_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == chain_head_kind && node.state_version == chain_head_version)
    })?;
    for frame in &chain_head_frames {
        verify_btc_chain_head_observation_replay(broker, frame)?;
    }

    let balance_kind = ObserveBtcAddressBalanceState::kind().map_err(replay_adapter_error)?;
    let balance_version = ObserveBtcAddressBalanceState::version().map_err(replay_adapter_error)?;
    let balance_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == balance_kind && node.state_version == balance_version)
    })?;
    for frame in &balance_frames {
        verify_btc_address_balance_observation_replay(broker, frame)?;
    }
    verify_btc_shared_joint_tips(broker, &balance_frames)?;
    verify_btc_address_balance_batch_replay(broker)?;
    Ok(())
}

fn verify_btc_collector_checkpoint_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = QueryCollectorCheckpointState::kind().map_err(replay_adapter_error)?;
    let state_version = QueryCollectorCheckpointState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        verify_btc_collector_checkpoint_frame(broker, frame)?;
    }
    Ok(())
}

fn verify_btc_collector_checkpoint_frame(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: QueryCollectorCheckpointConfig = replay_node_config(broker, &frame.node)?;
    let evidences = replay_fact_query_evidence_for_attempt(
        broker,
        &frame.node.node_id,
        &frame.produced.attempt_id,
    )?;
    if evidences.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin collector checkpoint query must retain exactly one fact-query evidence",
        ));
    }
    let evidence = &evidences[0];
    let response = checkpoint_response_from_query_evidence(broker, evidence)?;
    let expected =
        replay_loaded_checkpoint_from_evidence(&config, evidence, response).map_err(|_| {
            replay_btc_mismatch("Bitcoin collector checkpoint replay evidence was rejected")
        })?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn checkpoint_response_from_query_evidence(
    broker: &replay::ReplayBroker,
    evidence: &mfm_facts::FactQueryEvidence,
) -> replay::Result<Option<CollectorCheckpointResponse>> {
    let rows = mfm_facts::fact_query_result_rows_from_receipt(evidence.receipt());
    let selected = evidence.selection().selected_indices();
    if selected.is_empty() {
        return Ok(None);
    }
    if selected.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin collector checkpoint selection was not a single index",
        ));
    }
    let index = selected[0] as usize;
    let row = rows.get(index).ok_or_else(|| {
        replay_btc_mismatch("Bitcoin collector checkpoint selection index was out of range")
    })?;
    let fact_ref = row.fact_ref();
    let requirement = fact_response_artifact_requirement(fact_ref);
    let artifact = broker.retained_artifact(&requirement)?;
    let response: CollectorCheckpointResponse = hydrate_fact_response_json(
        fact_ref,
        &artifact.artifact_bytes,
    )
    .map_err(|_| {
        replay_btc_mismatch(
            "Bitcoin collector checkpoint response could not be hydrated from retained evidence",
        )
    })?;
    Ok(Some(response))
}

fn replay_fact_query_evidence_for_attempt(
    broker: &replay::ReplayBroker,
    node_id: &mfm_ids::NodeId,
    attempt_id: &mfm_ids::AttemptId,
) -> replay::Result<Vec<mfm_facts::FactQueryEvidence>> {
    let mut evidence = Vec::new();
    for event in broker.events() {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::FactQueryEvidence
            || payload.node_id.as_ref() != Some(node_id)
            || payload.attempt_id.as_ref() != Some(attempt_id)
        {
            continue;
        }
        let requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ArtifactReferenced,
            artifact_id: payload.artifact_ref.artifact_id.clone(),
            evidence_hash: payload.artifact_ref.evidence_hash.clone(),
            digest: Some(payload.artifact_ref.content_digest.clone()),
            byte_len: Some(payload.artifact_ref.byte_len),
            media_type: Some(payload.artifact_ref.media_type.clone()),
            schema_id: Some(payload.artifact_ref.schema_id.clone()),
            semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
            producer_node_id: payload.node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(payload.artifact_ref.role),
        };
        let artifact = broker.retained_artifact(&requirement)?;
        let parsed = mfm_facts::parse_canonical_fact_query_evidence_bytes(&artifact.artifact_bytes)
            .map_err(replay_adapter_error)?;
        evidence.push(parsed);
    }
    Ok(evidence)
}

fn verify_btc_shared_joint_tips(
    broker: &replay::ReplayBroker,
    frames: &[replay::ProducedCellReplayFrame],
) -> replay::Result<()> {
    let mut anchors = BTreeMap::<String, (u64, String)>::new();
    for frame in frames {
        let config: ObserveBtcAddressBalanceConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (tip.block_height(), tip.block_hash().to_owned());
        if anchors
            .insert(config.network.clone(), anchor.clone())
            .is_some_and(|existing| existing != anchor)
        {
            return Err(replay_btc_mismatch(
                "Bitcoin same-network observations did not share one joint tip",
            ));
        }
    }
    Ok(())
}

fn verify_btc_joint_tip_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = ResolveBtcJointTipState::kind().map_err(replay_adapter_error)?;
    let state_version = ResolveBtcJointTipState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let config: ResolveBtcJointTipConfig = replay_node_config(broker, &frame.node)?;
        let evidence: BtcChainHeadReadEvidence = replay_external_read_evidence(broker, frame)?;
        let selection = config.selection();
        if evidence.request_head_kind != selection.head_kind().as_str()
            || evidence.request_finality != selection.finality().as_str()
            || evidence.request_confirmation_depth != selection.finality().confirmation_depth()
            || evidence.response_head_kind != evidence.request_head_kind
            || evidence.response_finality != evidence.request_finality
            || evidence.response_confirmation_depth != evidence.request_confirmation_depth
            || evidence.network_id != config.network
            || evidence.source_identity != config.semantic_source_identity
            || evidence.bitcoin_network != config.bitcoin_network
            || evidence.observed_bitcoin_network != config.bitcoin_network
            || evidence.source_status != BtcSourceStatus::Synced.as_str()
        {
            return Err(replay_btc_mismatch(
                "Bitcoin joint-tip read evidence did not match certified request or source binding",
            ));
        }
        let expected = BtcJointTip::new(
            config.network,
            config.bitcoin_network,
            config.semantic_source_identity,
            evidence.block_height,
            evidence.block_hash,
            evidence.source_status,
            evidence.observed_bitcoin_network,
        )
        .map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn verify_btc_chain_head_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcChainHeadConfig = replay_node_config(broker, &frame.node)?;
    let request = chain_head_request(&config).map_err(replay_adapter_error)?;
    let evidence: BtcChainHeadReadEvidence = replay_external_read_evidence(broker, frame)?;
    let output: BtcChainHeadObservation =
        serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    let selection = request.selection();
    if evidence.request_head_kind != selection.head_kind().as_str()
        || evidence.request_finality != selection.finality().as_str()
        || evidence.request_confirmation_depth != selection.finality().confirmation_depth()
        || evidence.response_head_kind != evidence.request_head_kind
        || evidence.response_finality != evidence.request_finality
        || evidence.response_confirmation_depth != evidence.request_confirmation_depth
        || evidence.network_id != config.network
        || evidence.source_identity != config.semantic_source_identity
        || evidence.bitcoin_network != config.bitcoin_network
        || evidence.observed_bitcoin_network != config.bitcoin_network
        || evidence.source_status != output.response().observed_source_status()
        || output.source_read_count() != 1
        || output.subject().network() != evidence.network_id
        || output.subject().bitcoin_network() != evidence.bitcoin_network
        || output.subject().semantic_source_identity() != evidence.source_identity
        || output.subject().head_kind() != evidence.response_head_kind
        || output.response().block_height() != evidence.block_height
        || output.response().block_hash() != evidence.block_hash
        || output.response().observed_bitcoin_network() != evidence.observed_bitcoin_network
        || output.response().finality_policy() != evidence.response_finality
        || output.response().confirmation_depth() != evidence.response_confirmation_depth
        || output.response().provider_time_unix_ms() != evidence.provider_time_unix_ms
    {
        return Err(replay_btc_mismatch(
            "Bitcoin chain-head output did not match retained capability read evidence",
        ));
    }
    Ok(())
}

fn verify_btc_address_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcAddressBalanceConfig = replay_node_config(broker, &frame.node)?;
    let input_tip = replay_joint_tip_input(broker, &frame.node)?;
    let binding = address_balance_binding(&config).map_err(replay_adapter_error)?;
    let evidence: BtcBalanceReadEvidence = replay_external_read_evidence(broker, frame)?;
    let request = address_balance_request(&config, &input_tip).map_err(replay_adapter_error)?;
    if evidence.request_address != request.address().as_str()
        || evidence.request_block_height != request.block_height()
        || evidence.request_block_hash != request.block_hash().as_str()
        || evidence.response_address != evidence.request_address
        || evidence.response_block_height != evidence.request_block_height
        || evidence.response_block_hash != evidence.request_block_hash
        || evidence.network_id != config.network
        || evidence.source_identity != config.semantic_source_identity
        || evidence.bitcoin_network != config.bitcoin_network
        || evidence.observed_bitcoin_network != config.bitcoin_network
        || evidence.source_status != BtcSourceStatus::Synced.as_str()
    {
        return Err(replay_btc_mismatch(
            "Bitcoin address-balance read evidence did not match certified request or source binding",
        ));
    }
    let capability_response = BtcBalanceReadResponse {
        evidence: RedactedBtcSourceEvidence::from_binding(
            &binding,
            config.bitcoin_network.clone(),
            BtcSourceStatus::Synced,
        )
        .map_err(replay_adapter_error)?,
        address: BtcAddress::new(&evidence.response_address).map_err(replay_adapter_error)?,
        balance_sats: evidence.response_balance_sats,
        block_height: evidence.response_block_height,
        block_hash: BtcBlockHash::new(&evidence.response_block_hash)
            .map_err(replay_adapter_error)?,
    };
    let expected =
        normalize_btc_address_balance_observation(&config, &input_tip, &capability_response)
            .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn verify_btc_address_balance_batch_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = AssembleBtcAddressBalanceBatchState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleBtcAddressBalanceBatchState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let _config: AssembleBtcAddressBalanceBatchConfig =
            replay_node_config(broker, &frame.node)?;
        let joint_tip_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcJointTip::semantic_id().map_err(replay_adapter_error)?,
            &BtcJointTip::schema_id().map_err(replay_adapter_error)?,
        )?;
        let fact_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcAddressBalanceSnapshotFact::semantic_id().map_err(replay_adapter_error)?,
            &BtcAddressBalanceSnapshotFact::schema_id().map_err(replay_adapter_error)?,
        )?;
        if joint_tip_frames.len() != 1 || fact_frames.is_empty() {
            return Err(replay_btc_mismatch(
                "Bitcoin balance batch inputs were incomplete",
            ));
        }
        let joint_tip: BtcJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let facts = fact_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<BtcAddressBalanceSnapshotFact>>>()?;
        let input = AssembleBtcAddressBalanceBatchInput {
            joint_tip,
            balance_facts: NonEmpty::try_from_vec(facts).map_err(replay_adapter_error)?,
        };
        let expected = assemble_btc_address_balance_batch(input).map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn replay_joint_tip_input(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<BtcJointTip> {
    let frames = replay_input_frames(
        broker,
        node,
        &BtcJointTip::semantic_id().map_err(replay_adapter_error)?,
        &BtcJointTip::schema_id().map_err(replay_adapter_error)?,
    )?;
    if frames.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin balance observation did not consume exactly one joint tip",
        ));
    }
    let resolve_kind = ResolveBtcJointTipState::kind().map_err(replay_adapter_error)?;
    let resolve_version = ResolveBtcJointTipState::version().map_err(replay_adapter_error)?;
    if frames[0].node.state_kind != resolve_kind || frames[0].node.state_version != resolve_version
    {
        return Err(replay_btc_mismatch(
            "Bitcoin balance observation input was not produced by joint-tip resolution",
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
            return Err(replay_btc_mismatch(
                "certified collector input cell did not have exactly one produced value",
            ));
        }
        let frame = matches.into_iter().next().expect("one frame");
        if frame.cell.semantic_type_id != input_cell.semantic_type_id
            || frame.cell.schema_id != input_cell.schema_id
            || frame.cell.value_lineage != input_cell.value_lineage
        {
            return Err(replay_btc_mismatch(
                "collector input cell metadata did not match its produced value",
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
        return Err(replay_btc_mismatch(
            "Bitcoin external read did not have exactly one retained evidence artifact",
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

fn decode_replay_value<T>(frame: &replay::ProducedCellReplayFrame) -> replay::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)
}

fn ensure_canonical_value_matches<T: serde::Serialize>(
    expected: &T,
    actual: &[u8],
) -> replay::Result<()> {
    let json = serde_json::to_string(expected).map_err(replay_json_error)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(replay_adapter_error)?;
    if canonical.as_bytes() != actual {
        return Err(replay_btc_mismatch(
            "Bitcoin collector output did not match recomputed state output",
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

fn replay_btc_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}

fn fact_query_trust_root(
    material: &FactQueryReceiptTrustRootMaterial,
) -> mfm_runtime::Result<mfm_store::v1::FactQueryReceiptTrustRoot> {
    mfm_store::v1::FactQueryReceiptTrustRoot::from_material(material)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
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
