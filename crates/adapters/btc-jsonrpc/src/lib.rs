#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! this crate owns request mapping, runner registration, fact recording, and replay helpers over
//! recorded evidence.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_artifact_capabilities::{ArtifactReadProvider, ArtifactReadRequest};
use mfm_btc_capabilities::{
    BtcBlockHash, BtcCapabilityError, BtcCapabilityFuture, BtcChain, BtcChainHeadReadCapability,
    BtcChainHeadReadProvider, BtcChainHeadRequest, BtcChainHeadResponse, BtcFinality, BtcNetworkId,
    BtcSourceIdentity, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_collectors_btc_jsonrpc_http::{
    BlockHeaderInfo, BlockchainInfo, BtcJsonRpcClient, BtcRpcError,
};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{
    FactIndexReadEvidence, FactIndexReadProvider, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_ids::ContentDigest;
use mfm_program::{MfmFactType, PureState, StateSpec, ValidatedConfig};
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode,
    RunnerArtifactBuilder, RunnerCapabilityBinding, RunnerOutputBuilder, RunnerPayloadBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_btc::{
    btc_jsonrpc_adapter_kind, btc_jsonrpc_adapter_version, chain_head_fact_visibility,
    collector_checkpoint_fact_visibility, normalize_chain_head_response, BtcChainHeadFact,
    CollectorCheckpointFact, CollectorCheckpointResponse, CollectorCheckpointSubject,
    LoadedCollectorCheckpoint, ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput,
    ObserveBtcChainHeadState, QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointState, RecordBtcChainHeadFactConfig, RecordBtcChainHeadFactInput,
    RecordBtcChainHeadFactState, RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
    RecordCollectorCheckpointState,
};
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::Serialize;

const READ_FACTORY: &str = "read_external";
const PURE_FACTORY: &str = "pure";
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
}

impl BtcChainHeadReadProvider for BtcJsonRpcChainHeadProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move { self.read_chain_head_inner(request).await })
    }
}

/// Runtime capabilities used by Bitcoin JSON-RPC adapter runners.
#[derive(Clone)]
pub struct BtcJsonRpcRunnerCapabilities {
    artifacts: Arc<dyn ArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl BtcJsonRpcRunnerCapabilities {
    /// Creates runner capabilities from artifact and Bitcoin providers.
    pub fn new(
        artifacts: Arc<dyn ArtifactReadProvider>,
        btc: Arc<dyn BtcChainHeadReadProvider>,
        fact_index: Arc<dyn FactIndexReadProvider>,
    ) -> Self {
        Self {
            artifacts,
            btc,
            fact_index,
        }
    }

    fn artifacts(&self) -> Arc<dyn ArtifactReadProvider> {
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
    let read_factory = events::RunnerFactoryId::new(READ_FACTORY)?;
    let pure_factory = events::RunnerFactoryId::new(PURE_FACTORY)?;
    let adapter_factory = events::RunnerFactoryId::new(ADAPTER_FACTORY)?;
    registrations.register_adapter_executable(
        btc_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        btc_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        executable(adapter_factory)?,
    )?;
    let observe = mfm_program::registered_state_descriptor::<ObserveBtcChainHeadState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        observe.descriptor_id().clone(),
        observe.capabilities(),
        read_factory.clone(),
        executable(read_factory.clone())?,
        Arc::new(ObserveChainHeadRunner {
            artifacts: artifacts.clone(),
            btc,
        }),
    )?;
    let record_chain_head =
        mfm_program::registered_state_descriptor::<RecordBtcChainHeadFactState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        record_chain_head.descriptor_id().clone(),
        record_chain_head.capabilities(),
        pure_factory.clone(),
        executable(pure_factory.clone())?,
        Arc::new(RecordChainHeadFactRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    let query_checkpoint =
        mfm_program::registered_state_descriptor::<QueryCollectorCheckpointState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        query_checkpoint.descriptor_id().clone(),
        query_checkpoint.capabilities(),
        read_factory.clone(),
        executable(read_factory)?,
        Arc::new(QueryCheckpointRunner {
            artifacts: artifacts.clone(),
            fact_index,
        }),
    )?;
    let record_checkpoint =
        mfm_program::registered_state_descriptor::<RecordCollectorCheckpointState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        record_checkpoint.descriptor_id().clone(),
        record_checkpoint.capabilities(),
        pure_factory.clone(),
        executable(pure_factory)?,
        Arc::new(RecordCheckpointFactRunner { artifacts }),
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

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-btc-jsonrpc",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-btc-jsonrpc",
            "runner": "typed-bitcoin-jsonrpc",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct ObserveChainHeadRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadReadProvider>,
}

impl ErasedNodeRunner for ObserveChainHeadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<ObserveBtcChainHeadConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input = load_struct_input::<ObserveBtcChainHeadInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let request = config.as_ref().request().map_err(btc_state_runtime_error)?;
            let response = self
                .btc
                .read_chain_head(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            let fact = normalize_chain_head_response(&request, &response, &input)
                .map_err(btc_state_runtime_error)?;
            state_output(ctx, &fact).await
        })
    }
}

struct RecordChainHeadFactRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for RecordChainHeadFactRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<RecordBtcChainHeadFactConfig>(&ctx, self.artifacts.as_ref()).await?;
            let state = RecordBtcChainHeadFactState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_struct_input::<RecordBtcChainHeadFactInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let fact = state.run(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            record_fact_output(ctx, &fact, chain_head_fact_visibility()).await
        })
    }
}

struct QueryCheckpointRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ErasedNodeRunner for QueryCheckpointRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<QueryCollectorCheckpointConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let input = load_struct_input::<QueryCollectorCheckpointInput>(
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
            checkpoint_query_output(ctx, &loaded, &evidence).await
        })
    }
}

struct RecordCheckpointFactRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for RecordCheckpointFactRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<RecordCollectorCheckpointConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let state = RecordCollectorCheckpointState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_struct_input::<RecordCollectorCheckpointInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let checkpoint = state.run(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            record_fact_output(ctx, &checkpoint, collector_checkpoint_fact_visibility()).await
        })
    }
}

async fn record_fact_output<T>(
    ctx: ErasedRunCtx<'_>,
    fact: &T,
    visibility: mfm_program::facts::FactVisibility,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmFactType + MfmValue + Serialize + Clone,
{
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let output_artifact = artifacts.state_output(fact)?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.stage_attempt_artifact(&output_artifact)?;
    output.retain_runtime_evidence(&output_artifact);
    output.record_fact(
        mfm_runtime::FactRecordInput::new(fact.clone(), visibility),
        btc_capability_binding()?,
    )?;
    output.payload(payloads.cell_produced(&output_artifact)?);
    Ok(output.finish())
}

async fn state_output<T>(
    ctx: ErasedRunCtx<'_>,
    value: &T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let artifact = artifacts.state_output(value)?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.stage_attempt_artifact(&artifact)?;
    output.retain_runtime_evidence(&artifact);
    output.payload(payloads.cell_produced(&artifact)?);
    Ok(output.finish())
}

async fn checkpoint_query_output(
    ctx: ErasedRunCtx<'_>,
    value: &LoadedCollectorCheckpoint,
    evidence: &FactIndexReadEvidence,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let artifact = artifacts.state_output(value)?;
    let trust_root = fact_query_trust_root(evidence.trust_root())?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.stage_attempt_artifact(&artifact)?;
    output.retain_runtime_evidence(&artifact);
    output.record_fact_query_evidence(evidence.query_evidence().clone(), &trust_root)?;
    output.payload(payloads.cell_produced(&artifact)?);
    Ok(output.finish())
}

async fn query_collector_checkpoint(
    config: ValidatedConfig<QueryCollectorCheckpointConfig>,
    _input: QueryCollectorCheckpointInput,
    artifacts: &dyn ArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(LoadedCollectorCheckpoint, FactIndexReadEvidence)> {
    let config_value = config.as_ref().clone();
    let state = QueryCollectorCheckpointState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state.request().map_err(btc_state_runtime_error)?;
    let response = fact_index
        .read_fact_index(&request)
        .await
        .map_err(fact_index_runtime_error)?;
    let checkpoint = checkpoint_from_response(&config_value, &response, artifacts).await?;
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
    config: &QueryCollectorCheckpointConfig,
    response: &FactIndexReadResponse,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<Option<CollectorCheckpointFact>> {
    match response.rows() {
        [] => Ok(None),
        [row] => {
            let request = ArtifactReadRequest::from_internal_fact_response_ref(row.fact_ref());
            let bytes = artifacts
                .read_artifact(&request)
                .await
                .map_err(runtime_artifact_read_error)?;
            let checkpoint_response: CollectorCheckpointResponse =
                bytes.decode_json().map_err(runtime_artifact_read_error)?;
            checkpoint_fact_from_query_config(config, checkpoint_response)
                .map(Some)
                .map_err(btc_state_runtime_error)
        }
        _ => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "latest checkpoint query returned more than one row".to_owned(),
        )),
    }
}

/// Rebuilds loaded checkpoint material from recorded query evidence and retained response bytes.
pub fn replay_loaded_checkpoint_from_evidence(
    config: &QueryCollectorCheckpointConfig,
    evidence: &mfm_facts::FactQueryEvidence,
    response: Option<CollectorCheckpointResponse>,
) -> Result<LoadedCollectorCheckpoint> {
    mfm_facts::validate_fact_query_evidence(evidence)
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)?;
    let checkpoint = match (
        evidence.receipt().returned_refs().len(),
        evidence.selection().selected_indices(),
        response,
    ) {
        (0, [], None) => None,
        (1, [0], Some(response)) => Some(
            checkpoint_fact_from_query_config(config, response)
                .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)?,
        ),
        _ => return Err(BtcJsonRpcAdapterError::ReplayEvidenceMismatch),
    };
    Ok(LoadedCollectorCheckpoint::new(checkpoint))
}

fn checkpoint_fact_from_query_config(
    config: &QueryCollectorCheckpointConfig,
    response: CollectorCheckpointResponse,
) -> std::result::Result<CollectorCheckpointFact, mfm_states_btc::BtcStateError> {
    mfm_states_btc::validate_query_collector_checkpoint_config(config)
        .map_err(|reason| mfm_states_btc::BtcStateError::InvalidInput { reason })?;
    let source_identity = BtcSourceIdentity::new(&config.semantic_source_identity)?;
    let network = BtcNetworkId::new(&config.network)?;
    let subject = CollectorCheckpointSubject::new(
        config.collector_kind.clone(),
        &source_identity,
        config.partition.clone(),
        BtcChain::Bitcoin,
        &network,
    );
    Ok(CollectorCheckpointFact::new(subject, response))
}

fn fact_query_trust_root(
    material: &FactQueryReceiptTrustRootMaterial,
) -> mfm_runtime::Result<mfm_store::v1::FactQueryReceiptTrustRoot> {
    mfm_store::v1::FactQueryReceiptTrustRoot::new(
        material.store_identity().clone(),
        material.scheme(),
        material.key_id().clone(),
        *material.verifying_key(),
    )
    .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let request = ArtifactReadRequest::from_certified_config_ref(&ctx.node().config_ref);
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    decode_config_bytes(verified.bytes())
}
fn decode_config_bytes<T>(bytes: &[u8]) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let config: T = serde_json::from_slice(bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    ValidatedConfig::new(config).map_err(|error| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "Bitcoin config failed validation: {error}"
        ))
    })
}

async fn load_struct_input<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: DeserializeOwned,
{
    let value = materialized_node_json(&inputs.root, artifacts).await?;
    serde_json::from_value(value)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn materialized_node_json(
    node: &MaterializedInputNode,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<serde_json::Value> {
    match node {
        MaterializedInputNode::Unit => Ok(serde_json::Value::Null),
        MaterializedInputNode::Cell(_) => {
            let bytes = load_cell_bytes(node, artifacts).await?;
            serde_json::from_slice(&bytes)
                .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
        }
        MaterializedInputNode::Tuple(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(Box::pin(materialized_node_json(element, artifacts)).await?);
            }
            Ok(serde_json::Value::Array(values))
        }
        MaterializedInputNode::Struct(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.field_path.as_str().to_owned(),
                    Box::pin(materialized_node_json(&field.node, artifacts)).await?,
                );
            }
            Ok(serde_json::Value::Object(object))
        }
        MaterializedInputNode::Vec(elements) | MaterializedInputNode::NonEmptyVec(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(Box::pin(materialized_node_json(element, artifacts)).await?);
            }
            Ok(serde_json::Value::Array(values))
        }
    }
}

async fn load_cell_bytes(
    node: &MaterializedInputNode,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<Vec<u8>> {
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "Bitcoin input node was not a produced cell".to_owned(),
        ));
    };
    let request = match &cell.terminal {
        MaterializedCellTerminal::Produced {
            producer_node_id,
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
            producer_node_id.clone(),
        ),
        MaterializedCellTerminal::Seed {
            seed_id,
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_seed_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
            seed_id.clone(),
        ),
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "Bitcoin input cell was skipped".to_owned(),
            ))
        }
    };
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    Ok(verified.into_bytes())
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

fn btc_capability_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    Ok(RunnerCapabilityBinding {
        capability_kind: BtcChainHeadReadCapability::kind()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?,
        capability_version: BtcChainHeadReadCapability::version()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?,
        adapter_kind: btc_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        adapter_version: btc_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
    })
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

fn runtime_artifact_read_error(
    error: mfm_artifact_capabilities::ArtifactReadError,
) -> mfm_runtime::RuntimeError {
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
