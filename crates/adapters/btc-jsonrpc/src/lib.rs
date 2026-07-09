#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! this crate owns request mapping, runner registration, fact recording, and recorded provider
//! backends over replay evidence.

use std::marker::PhantomData;
use std::sync::Arc;

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcAddress, BtcBalanceReadProvider, BtcBalanceReadRequest,
    BtcBalanceReadResponse, BtcBlockHash, BtcCapabilityError, BtcCapabilityFuture,
    BtcChainHeadReadProvider, BtcChainHeadRequest, BtcChainHeadResponse, BtcFinality, BtcHeadKind,
    BtcNetworkId, BtcSourceBinding, BtcSourceIdentity, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{
    FactIndexReadEvidence, FactIndexReadProvider, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial, FactRecordCapability,
};
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
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
    collector_checkpoint_fact_visibility, AssembleBtcAddressBalanceBatchConfig,
    AssembleBtcAddressBalanceBatchInput, AssembleBtcAddressBalanceBatchState,
    BtcAddressBalanceObservation, BtcChainHeadObservation, BtcJointTip, CollectorCheckpointFact,
    CollectorCheckpointResponse, LoadedCollectorCheckpoint, ObserveBtcAddressBalanceConfig,
    ObserveBtcAddressBalanceInput, ObserveBtcAddressBalanceState, ObserveBtcChainHeadConfig,
    ObserveBtcChainHeadInput, ObserveBtcChainHeadState, QueryCollectorCheckpointConfig,
    QueryCollectorCheckpointInput, QueryCollectorCheckpointState, RecordBtcAddressBalanceFactState,
    RecordBtcChainHeadFactState, RecordCollectorCheckpointState, ResolveBtcJointTipConfig,
    ResolveBtcJointTipInput, ResolveBtcJointTipState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "btc_jsonrpc_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.bitcoin.jsonrpc.runtime.v1";

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
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let btc = capabilities.btc();
    let fact_index = capabilities.fact_index();
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
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
    registrations.register_state_descriptor_with_factory::<ObserveBtcChainHeadState>(
        &read_factory,
        Arc::new(ObserveChainHeadRunner {
            artifacts: artifacts.clone(),
            btc: btc.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ResolveBtcJointTipState>(
        &read_factory,
        Arc::new(ResolveJointTipRunner {
            artifacts: artifacts.clone(),
            btc: btc.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ObserveBtcAddressBalanceState>(
        &read_factory,
        Arc::new(ObserveAddressBalanceRunner {
            artifacts: artifacts.clone(),
            btc,
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<RecordBtcChainHeadFactState>(
        &managed_write_factory,
        Arc::new(ManagedFactRecordRunner::<RecordBtcChainHeadFactState>::new(
            artifacts.clone(),
            chain_head_fact_visibility(),
        )),
    )?;
    registrations.register_state_descriptor_with_factory::<RecordBtcAddressBalanceFactState>(
        &managed_write_factory,
        Arc::new(
            ManagedFactRecordRunner::<RecordBtcAddressBalanceFactState>::new(
                artifacts.clone(),
                address_balance_record_visibility(),
            ),
        ),
    )?;
    registrations.register_state_descriptor_with_factory::<QueryCollectorCheckpointState>(
        &read_factory,
        Arc::new(QueryCheckpointRunner {
            artifacts: artifacts.clone(),
            fact_index,
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<RecordCollectorCheckpointState>(
        &managed_write_factory,
        Arc::new(
            ManagedFactRecordRunner::<RecordCollectorCheckpointState>::new(
                artifacts.clone(),
                collector_checkpoint_fact_visibility(),
            ),
        ),
    )?;
    registrations.register_state_descriptor_with_factory::<AssembleBtcAddressBalanceBatchState>(
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

fn joint_tip_request(config: &ResolveBtcJointTipConfig) -> Result<BtcChainHeadRequest> {
    let selection = config
        .selection()
        .map_err(|_| BtcJsonRpcAdapterError::InvalidCapabilityRequest)?;
    Ok(BtcChainHeadRequest::new(selection))
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
            ErasedRunnerOutput::state_output(&ctx, &fact)
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
            let request = joint_tip_request(config.as_ref()).map_err(btc_adapter_runtime_error)?;
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
            ErasedRunnerOutput::state_output(&ctx, &tip)
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
            ErasedRunnerOutput::state_output(&ctx, &observation)
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

/// Verifies Bitcoin JSON-RPC observation cell outputs when present in a broker stream.
///
/// Covers chain-head and address-balance observe states. Returns `Ok(false)` when the stream
/// contains no matching Bitcoin observation outputs.
pub fn verify_btc_jsonrpc_replay(broker: &replay::ReplayBroker) -> replay::Result<bool> {
    let mut found = false;

    let chain_head_kind = ObserveBtcChainHeadState::kind().map_err(replay_adapter_error)?;
    let chain_head_version = ObserveBtcChainHeadState::version().map_err(replay_adapter_error)?;
    let chain_head_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == chain_head_kind && node.state_version == chain_head_version)
    })?;
    for frame in &chain_head_frames {
        verify_btc_chain_head_observation_replay(broker, frame)?;
        found = true;
    }

    let balance_kind = ObserveBtcAddressBalanceState::kind().map_err(replay_adapter_error)?;
    let balance_version = ObserveBtcAddressBalanceState::version().map_err(replay_adapter_error)?;
    let balance_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == balance_kind && node.state_version == balance_version)
    })?;
    for frame in &balance_frames {
        verify_btc_address_balance_observation_replay(broker, frame)?;
        found = true;
    }

    Ok(found)
}

fn verify_btc_chain_head_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcChainHeadConfig = replay_node_config(broker, &frame.node)?;
    let binding = chain_head_binding(&config).map_err(replay_adapter_error)?;
    let request = chain_head_request(&config).map_err(replay_adapter_error)?;
    let output: BtcChainHeadObservation =
        serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    let response = replay_capability_response_from_observation(&binding, &output)?;
    recorded_chain_head_response(&binding, &request, &response).map_err(replay_adapter_error)?;
    Ok(())
}

fn verify_btc_address_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcAddressBalanceConfig = replay_node_config(broker, &frame.node)?;
    let binding = address_balance_binding(&config).map_err(replay_adapter_error)?;
    let output: BtcAddressBalanceObservation =
        serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    let subject = output.subject();
    let response = output.response();
    if subject.network() != config.network.as_str()
        || subject.bitcoin_network() != config.bitcoin_network.as_str()
        || subject.semantic_source_identity() != config.semantic_source_identity.as_str()
        || subject.address() != config.address.as_str()
        || output.source_read_count() == 0
        || output.source_read_count() > config.max_source_reads.get()
    {
        return Err(replay_btc_mismatch(
            "Bitcoin address-balance observation did not match certified config binding",
        ));
    }
    let address = BtcAddress::new(subject.address()).map_err(replay_adapter_error)?;
    let block_hash = BtcBlockHash::new(response.anchor_hash()).map_err(replay_adapter_error)?;
    let request = BtcBalanceReadRequest::new(address.clone(), response.anchor_height(), block_hash);
    let evidence = RedactedBtcSourceEvidence::from_binding(
        &binding,
        subject.bitcoin_network(),
        BtcSourceStatus::Synced,
    )
    .map_err(replay_adapter_error)?;
    let capability_response = BtcBalanceReadResponse {
        evidence,
        address,
        balance_sats: response.balance_sats(),
        block_height: response.anchor_height(),
        block_hash: BtcBlockHash::new(response.anchor_hash()).map_err(replay_adapter_error)?,
    };
    recorded_balance_response(&binding, &request, &capability_response)
        .map_err(replay_adapter_error)?;
    Ok(())
}

fn replay_capability_response_from_observation(
    binding: &BtcSourceBinding,
    output: &BtcChainHeadObservation,
) -> replay::Result<BtcChainHeadResponse> {
    let subject = output.subject();
    let response = output.response();
    if subject.network() != binding.network_id().as_str()
        || subject.bitcoin_network() != binding.bitcoin_network().as_str()
        || subject.semantic_source_identity() != binding.source_identity().as_str()
        || output.source_read_count() != 1
    {
        return Err(replay_btc_mismatch(
            "Bitcoin observation output did not match certified source binding",
        ));
    }
    let head_kind = replay_head_kind(subject.head_kind())?;
    let finality = replay_finality(response.finality_policy(), response.confirmation_depth())?;
    let source_status = replay_source_status(response.observed_source_status())?;
    let evidence = RedactedBtcSourceEvidence::from_binding(
        binding,
        response.observed_bitcoin_network(),
        source_status,
    )
    .map_err(replay_adapter_error)?;
    Ok(BtcChainHeadResponse {
        evidence,
        head_kind,
        finality,
        block_height: response.block_height(),
        block_hash: BtcBlockHash::new(response.block_hash()).map_err(replay_adapter_error)?,
        provider_time_unix_ms: response.provider_time_unix_ms(),
    })
}

fn replay_head_kind(value: &str) -> replay::Result<BtcHeadKind> {
    match value {
        "best" => Ok(BtcHeadKind::Best),
        "confirmed" => Ok(BtcHeadKind::Confirmed),
        _ => Err(replay_btc_mismatch("Bitcoin replay head kind was invalid")),
    }
}

fn replay_finality(value: &str, confirmation_depth: Option<u64>) -> replay::Result<BtcFinality> {
    match (value, confirmation_depth) {
        ("best_available", None) => Ok(BtcFinality::BestAvailable),
        ("confirmations", Some(depth)) => {
            BtcFinality::confirmations(depth).map_err(replay_adapter_error)
        }
        _ => Err(replay_btc_mismatch("Bitcoin replay finality was invalid")),
    }
}

fn replay_source_status(value: &str) -> replay::Result<BtcSourceStatus> {
    match value {
        "synced" => Ok(BtcSourceStatus::Synced),
        "initial_block_download" => Ok(BtcSourceStatus::InitialBlockDownload),
        "unknown" => Ok(BtcSourceStatus::Unknown),
        _ => Err(replay_btc_mismatch(
            "Bitcoin replay source status was invalid",
        )),
    }
}

fn replay_node_config<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
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
