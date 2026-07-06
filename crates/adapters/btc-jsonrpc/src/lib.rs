#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! this crate owns request mapping, runner registration, fact recording, and replay helpers over
//! recorded evidence.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_btc_capabilities::{
    BtcBalanceReadProvider, BtcBalanceReadRequest, BtcBalanceReadResponse, BtcBlockHash,
    BtcCapabilityError, BtcCapabilityFuture, BtcChainHeadReadProvider, BtcChainHeadRequest,
    BtcChainHeadResponse, BtcFinality, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_collectors_btc_jsonrpc_http::{
    BlockHeaderInfo, BlockchainInfo, BtcJsonRpcClient, BtcRpcError, ScanTxOutSetResult,
};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{
    FactIndexReadEvidence, FactIndexReadProvider, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    RunnerCapabilityBinding, RunnerExecutableIdentityTemplate, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_btc::{
    btc_jsonrpc_adapter_kind, btc_jsonrpc_adapter_version, chain_head_fact_visibility,
    collector_checkpoint_fact_visibility, normalize_chain_head_response, BtcChainHeadFact,
    BtcFactRecordCapability, CollectorCheckpointFact, CollectorCheckpointResponse,
    LoadedCollectorCheckpoint, ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput,
    ObserveBtcChainHeadState, QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointState, RecordBtcChainHeadFactState, RecordCollectorCheckpointState,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;

const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "btc_jsonrpc_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.bitcoin.jsonrpc.runtime.v1";

/// Result type for Bitcoin JSON-RPC adapter operations.
pub type Result<T> = std::result::Result<T, BtcJsonRpcAdapterError>;

/// Boxed future returned by adapter transport abstractions.
pub type BtcTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BtcRpcError>> + Send + 'a>>;

/// Minimal Bitcoin JSON-RPC transport surface needed by this adapter.
pub trait BtcJsonRpcChainHeadTransport: Send + Sync {
    /// Reads current blockchain summary information.
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo>;

    /// Reads the block hash for a selected height.
    fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String>;

    /// Reads verbose block-header metadata for a block hash.
    fn get_block_header<'a>(
        &'a self,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo>;

    /// Scans the UTXO set for a single address.
    fn scan_tx_out_set<'a>(
        &'a self,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult>;
}

impl BtcJsonRpcChainHeadTransport for BtcJsonRpcClient {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move { BtcJsonRpcClient::get_blockchain_info(self).await })
    }

    fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String> {
        Box::pin(async move { BtcJsonRpcClient::get_block_hash(self, height).await })
    }

    fn get_block_header<'a>(
        &'a self,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
        Box::pin(async move { BtcJsonRpcClient::get_block_header(self, block_hash).await })
    }

    fn scan_tx_out_set<'a>(
        &'a self,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
        Box::pin(async move { BtcJsonRpcClient::scan_tx_out_set(self, address).await })
    }
}

/// Bitcoin chain-head provider backed by a redacting JSON-RPC transport.
#[derive(Clone)]
pub struct BtcJsonRpcChainHeadProvider {
    transport: Arc<dyn BtcJsonRpcChainHeadTransport>,
}

impl BtcJsonRpcChainHeadProvider {
    /// Creates a provider over a concrete JSON-RPC transport.
    pub fn new(transport: Arc<dyn BtcJsonRpcChainHeadTransport>) -> Self {
        Self { transport }
    }

    async fn read_chain_head_inner(
        &self,
        request: &BtcChainHeadRequest,
    ) -> mfm_btc_capabilities::Result<BtcChainHeadResponse> {
        let info = self
            .transport
            .get_blockchain_info()
            .await
            .map_err(redacted_provider_error)?;
        let status = source_status(&info);
        let (height, hash) = selected_head(&*self.transport, &info, request).await?;
        let header = self
            .transport
            .get_block_header(hash.as_str())
            .await
            .map_err(redacted_provider_error)?;
        verify_header(&header, height, hash.as_str())?;
        let provider_time_unix_ms = header.time.checked_mul(1000);
        let evidence = RedactedBtcSourceEvidence::from_request(request, Some(info.chain), status);
        let response = BtcChainHeadResponse {
            evidence,
            block_height: height,
            block_hash: hash,
            provider_time_unix_ms,
        };
        response.verify_request(request)?;
        Ok(response)
    }

    async fn read_balance_inner(
        &self,
        request: &BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::Result<BtcBalanceReadResponse> {
        if request.selection != mfm_btc_capabilities::BtcHeadSelection::best() {
            return Err(BtcCapabilityError::redacted_provider_failure(
                "bitcoin balance reads support only best-available UTXO scans",
            ));
        }
        let info = self
            .transport
            .get_blockchain_info()
            .await
            .map_err(redacted_provider_error)?;
        let status = source_status(&info);
        let result = self
            .transport
            .scan_tx_out_set(request.address.as_str())
            .await
            .map_err(redacted_provider_error)?;
        if !result.success {
            return Err(BtcCapabilityError::redacted_provider_failure(
                "bitcoin UTXO scan did not complete successfully",
            ));
        }
        let evidence = RedactedBtcSourceEvidence::from_request(
            &BtcChainHeadRequest {
                guard: request.guard.clone(),
                selection: request.selection,
            },
            Some(info.chain),
            status,
        );
        let response = BtcBalanceReadResponse {
            evidence,
            address: request.address.clone(),
            balance_sats: result.total_amount_sats,
            block_height: result.height,
            block_hash: provider_block_hash(&result.bestblock)?,
        };
        response.verify_request(request)?;
        Ok(response)
    }
}

impl BtcChainHeadReadProvider for BtcJsonRpcChainHeadProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move { self.read_chain_head_inner(request).await })
    }
}

impl BtcBalanceReadProvider for BtcJsonRpcChainHeadProvider {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        Box::pin(async move { self.read_balance_inner(request).await })
    }
}

/// Runtime capabilities used by Bitcoin JSON-RPC adapter runners.
#[derive(Clone)]
pub struct BtcJsonRpcRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl BtcJsonRpcRunnerCapabilities {
    /// Creates runner capabilities from artifact and Bitcoin providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        btc: Arc<dyn BtcChainHeadReadProvider>,
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

    fn btc(&self) -> Arc<dyn BtcChainHeadReadProvider> {
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
                artifacts,
                collector_checkpoint_fact_visibility(),
            ),
        ),
    )?;
    Ok(())
}

/// Rebuilds a chain-head fact from recorded request, response, and observe input evidence.
pub fn replay_chain_head_fact_from_evidence(
    request: &BtcChainHeadRequest,
    response: &BtcChainHeadResponse,
    input: &ObserveBtcChainHeadInput,
) -> Result<BtcChainHeadFact> {
    normalize_chain_head_response(request, response, input)
        .map(|observation| observation.into_fact())
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)
}

/// Verifies recorded Bitcoin source evidence against the original request.
pub fn verify_recorded_chain_head_evidence(
    request: &BtcChainHeadRequest,
    response: &BtcChainHeadResponse,
) -> Result<()> {
    response
        .verify_request(request)
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)
}

/// Redaction-safe Bitcoin JSON-RPC adapter error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcJsonRpcAdapterError {
    /// Replay evidence did not match the certified request.
    #[error("Bitcoin replay evidence did not match request")]
    ReplayEvidenceMismatch,
}

struct ObserveChainHeadRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadReadProvider>,
}

impl ErasedNodeRunner for ObserveChainHeadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ObserveBtcChainHeadConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let state = ObserveBtcChainHeadState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<ObserveBtcChainHeadInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let request = state.request().map_err(btc_state_runtime_error)?;
            let response = self
                .btc
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
    S: ManagedWriteState<Caps = (BtcFactRecordCapability,)>,
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
    S: ManagedWriteState<Caps = (BtcFactRecordCapability,)>,
    S::Input: serde::de::DeserializeOwned,
    S::Output: MfmFactType + MfmValue,
{
    let config = load_runner_config::<S::Config>(&ctx, artifacts).await?;
    let state = S::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let input = load_materialized_struct_input::<S::Input>(ctx.inputs(), artifacts).await?;
    let context = ctx.certified_context::<S::Context>()?;
    let fact = state
        .run(input, &(BtcFactRecordCapability,), &context)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output_and_record_fact(
        mfm_runtime::FactRecordInput::new(fact, visibility),
        btc_fact_record_capability_binding()?,
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
                serde_json::from_slice(artifact.bytes()).map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            state
                .checkpoint_fact_from_response(checkpoint_response)
                .map(Some)
                .map_err(btc_state_runtime_error)
        }
    }
}

fn fact_response_artifact_requirement(
    fact_ref: &mfm_facts::InternalFactRef,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::FactResponse,
        artifact_id: fact_ref.artifact_id().clone(),
        digest: Some(fact_ref.response_hash().clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(fact_ref.response_schema_id().clone()),
        semantic_type_id: None,
        producer_node_id: Some(fact_ref.producer_node_id().clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::FactResponse),
    }
}

/// Rebuilds loaded checkpoint material from recorded query evidence and retained response bytes.
pub fn replay_loaded_checkpoint_from_evidence(
    config: &QueryCollectorCheckpointConfig,
    evidence: &mfm_facts::FactQueryEvidence,
    response: Option<CollectorCheckpointResponse>,
) -> Result<LoadedCollectorCheckpoint> {
    config
        .loaded_checkpoint_from_replay_evidence(evidence, response)
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)
}

fn fact_query_trust_root(
    material: &FactQueryReceiptTrustRootMaterial,
) -> mfm_runtime::Result<mfm_store::v1::FactQueryReceiptTrustRoot> {
    mfm_store::v1::FactQueryReceiptTrustRoot::from_material(material)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn selected_head(
    transport: &dyn BtcJsonRpcChainHeadTransport,
    info: &BlockchainInfo,
    request: &BtcChainHeadRequest,
) -> mfm_btc_capabilities::Result<(u64, BtcBlockHash)> {
    match request.selection.finality() {
        BtcFinality::BestAvailable => {
            let hash = provider_block_hash(&info.bestblockhash)?;
            Ok((info.blocks, hash))
        }
        BtcFinality::Confirmations(confirmations) => {
            let confirmations = confirmations.get();
            if info.blocks < confirmations {
                return Err(BtcCapabilityError::redacted_provider_failure(
                    "bitcoin source tip is below requested confirmation depth",
                ));
            }
            let height = info.blocks - confirmations;
            let hash = transport
                .get_block_hash(height)
                .await
                .map_err(redacted_provider_error)?;
            Ok((height, provider_block_hash(&hash)?))
        }
    }
}

fn source_status(info: &BlockchainInfo) -> BtcSourceStatus {
    match info.initialblockdownload {
        Some(true) => BtcSourceStatus::InitialBlockDownload,
        Some(false) => BtcSourceStatus::Synced,
        None => BtcSourceStatus::Unknown,
    }
}

fn verify_header(
    header: &BlockHeaderInfo,
    height: u64,
    hash: &str,
) -> mfm_btc_capabilities::Result<()> {
    if header.height == height && header.hash.eq_ignore_ascii_case(hash) {
        Ok(())
    } else {
        Err(BtcCapabilityError::redacted_provider_failure(
            "bitcoin block header did not match selected head",
        ))
    }
}

fn provider_block_hash(hash: &str) -> mfm_btc_capabilities::Result<BtcBlockHash> {
    BtcBlockHash::new(hash).map_err(|_| {
        BtcCapabilityError::redacted_provider_failure(
            "bitcoin provider returned invalid block hash",
        )
    })
}

fn redacted_provider_error(error: BtcRpcError) -> BtcCapabilityError {
    BtcCapabilityError::redacted_provider_failure(error)
}

fn btc_fact_record_capability_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<BtcFactRecordCapability>(
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

fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
mod tests;
