#![warn(missing_docs)]
//! Typed portfolio workflow runners.
//!
//! This crate binds certified portfolio state descriptors to concrete typed runners. It receives
//! only certified node specs, store-verified input cell evidence, and typed artifact handles from
//! the kernel runtime.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, U256};
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_events::v1 as events;
use mfm_evm_core::encoding::{
    encode_erc20_balance_of, encode_erc20_decimals, parse_u256_hex_value, parse_u8_u256,
    u64_hex_quantity,
};
use mfm_ids::{ArtifactId, ContentDigest, DescriptorId, NodeId};
use mfm_program::StateSpec;
use mfm_runtime::{
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode, StagedArtifact,
    StagedRetentionRefs,
};
use mfm_spec::v1 as spec;
use mfm_state_portfolio::{
    balance_reader_kind, evm_block_number_for, observe_batch_with_backend, pin_views_with_backend,
    portfolio_adapter_kind, portfolio_adapter_version, prepare_sources_from_config,
    resolve_subjects_from_config, resolve_valuations_from_config, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotState, MergeObservationsConfig, MergeObservationsState,
    ObservationBatch, ObservationRequest, ObservationResponse, ObserveBatchConfig,
    ObserveBatchInput, ObserveBatchState, PinViewsConfig, PinViewsState, PortfolioReadBackend,
    PortfolioReadCapability, PortfolioReadError, PortfolioReadFuture, PrepareSourcesConfig,
    PrepareSourcesState, ProjectReportConfig, ProjectReportInput, ProjectReportState,
    PublishSnapshotConfig, PublishSnapshotInput, PublishSnapshotState, ResolveSubjectsConfig,
    ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
    SourcePreparationRequest, SourcePreparationResponse, ViewPinRequest, ViewPinResponse,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue, NonEmpty};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const READ_FACTORY: &str = "read_external";
const PURE_FACTORY: &str = "pure";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

/// Future returned by typed portfolio artifact readers.
pub type PortfolioArtifactReaderFuture<'a, T> =
    Pin<Box<dyn Future<Output = mfm_runtime::Result<T>> + Send + 'a>>;

/// Read-only artifact boundary used by typed portfolio runners.
pub trait PortfolioArtifactReader: Send + Sync {
    /// Loads verified typed artifact bytes by id.
    fn get_artifact_by_id<'a>(
        &'a self,
        artifact_id: &'a ArtifactId,
    ) -> PortfolioArtifactReaderFuture<'a, (Vec<u8>, store::ArtifactEvidenceRef)>;
}

impl PortfolioArtifactReader for FsTypedArtifactStore {
    fn get_artifact_by_id<'a>(
        &'a self,
        artifact_id: &'a ArtifactId,
    ) -> PortfolioArtifactReaderFuture<'a, (Vec<u8>, store::ArtifactEvidenceRef)> {
        Box::pin(async move {
            self.get_artifact_by_id(artifact_id)
                .await
                .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))
        })
    }
}

/// Registers typed portfolio runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn PortfolioArtifactReader>,
) -> mfm_runtime::Result<()> {
    registry.register(binding(
        registered_descriptor::<PrepareSourcesState>()?,
        READ_FACTORY,
        Arc::new(PrepareSourcesRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ResolveSubjectsState>()?,
        PURE_FACTORY,
        Arc::new(ResolveSubjectsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<PinViewsState>()?,
        READ_FACTORY,
        Arc::new(PinViewsRunner {
            artifacts: artifacts.clone(),
            rpc: PortfolioRpcClient::from_env(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ResolveValuationsState>()?,
        PURE_FACTORY,
        Arc::new(ResolveValuationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ObserveBatchState>()?,
        READ_FACTORY,
        Arc::new(ObserveBatchRunner {
            artifacts: artifacts.clone(),
            rpc: PortfolioRpcClient::from_env(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<MergeObservationsState>()?,
        PURE_FACTORY,
        Arc::new(MergeObservationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<AssembleSnapshotState>()?,
        PURE_FACTORY,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<PublishSnapshotState>()?,
        PURE_FACTORY,
        Arc::new(PublishSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ProjectReportState>()?,
        PURE_FACTORY,
        Arc::new(ProjectReportRunner { artifacts }),
    )?)?;
    Ok(())
}

fn registered_descriptor<S>() -> mfm_runtime::Result<DescriptorId>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let mut states = mfm_program::StateRegistryBuilder::new();
    let registered = states
        .register::<S>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(registered.descriptor().descriptor_id().clone())
}

fn binding(
    descriptor_id: DescriptorId,
    factory: &'static str,
    runner: Arc<dyn ErasedNodeRunner>,
) -> mfm_runtime::Result<ErasedRunnerBinding> {
    let factory_id = events::RunnerFactoryId::new(factory)?;
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        executable(factory_id)?,
        runner,
    )
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-transports-portfolio-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-transports-portfolio")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-portfolio",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-portfolio",
            "runner": "typed-portfolio",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct PrepareSourcesRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for PrepareSourcesRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<PrepareSourcesConfig>(&ctx, self.artifacts.as_ref()).await?;
            let prepared = prepare_sources_from_config(&config);
            let request = SourcePreparationRequest {
                network_ids: config
                    .networks
                    .iter()
                    .map(|network| network.network_id.clone())
                    .collect(),
            };
            let response = SourcePreparationResponse {
                prepared: prepared.clone(),
            };
            read_output(ctx, request, response, prepared).await
        })
    }
}

struct ResolveSubjectsRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for ResolveSubjectsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<ResolveSubjectsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let _prepared = load_input_cell::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_subjects_from_config(&config);
            state_output(ctx, &output).await
        })
    }
}

struct PinViewsRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
    rpc: PortfolioRpcClient,
}

impl ErasedNodeRunner for PinViewsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<PinViewsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let _prepared = load_input_cell::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = pin_views_with_backend(&config, &self.rpc)
                .await
                .map_err(portfolio_read_runtime_error)?;
            let request = ViewPinRequest {
                network_ids: config
                    .networks
                    .iter()
                    .map(|network| network.network_id.clone())
                    .collect(),
            };
            let response = ViewPinResponse {
                views: output.clone(),
            };
            read_output(ctx, request, response, output).await
        })
    }
}

struct ResolveValuationsRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for ResolveValuationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<ResolveValuationsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let views = load_input_cell::<mfm_state_portfolio::PinnedViews>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_valuations_from_config(&config, &views);
            state_output(ctx, &output).await
        })
    }
}

struct ObserveBatchRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
    rpc: PortfolioRpcClient,
}

impl ErasedNodeRunner for ObserveBatchRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<ObserveBatchConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input =
                load_struct_input::<ObserveBatchInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let block_number = evm_block_number_for(&input.views, &config.network.network_id);
            let output = observe_batch_with_backend(&config, &input, &self.rpc).await;
            let request = ObservationRequest {
                wallet_id: config.wallet.wallet_id.clone(),
                symbol_id: config.symbol.symbol_id.clone(),
                network_id: config.network.network_id.clone(),
                balance_reader_kind: balance_reader_kind(&config.symbol.balance_reader).to_owned(),
                block_number: Some(block_number),
            };
            let response = ObservationResponse {
                batch: output.clone(),
            };
            read_output(ctx, request, response, output).await
        })
    }
}

struct MergeObservationsRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for MergeObservationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config =
                load_config::<MergeObservationsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let batches =
                load_non_empty_input::<ObservationBatch>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::merge_observation_batches(batches);
            state_output(ctx, &output).await
        })
    }
}

struct AssembleSnapshotRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<AssembleSnapshotConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input =
                load_struct_input::<AssembleSnapshotInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::assemble_snapshot(&config, input, 0);
            state_output(ctx, &output).await
        })
    }
}

struct PublishSnapshotRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for PublishSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<PublishSnapshotConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input =
                load_struct_input::<PublishSnapshotInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::PublishedPortfolioSnapshot {
                publish_version: config.publish_version,
                snapshot: input.snapshot,
            };
            state_output(ctx, &output).await
        })
    }
}

struct ProjectReportRunner {
    artifacts: Arc<dyn PortfolioArtifactReader>,
}

impl ErasedNodeRunner for ProjectReportRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<ProjectReportConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input =
                load_struct_input::<ProjectReportInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(
                input.snapshot.snapshot,
                config.report_version,
            )
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            state_output(ctx, &output).await
        })
    }
}

async fn read_output<Request, Response, Output>(
    ctx: ErasedRunCtx<'_>,
    request: Request,
    response: Response,
    output: Output,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Request: MfmValue + Serialize,
    Response: MfmValue + Serialize,
    Output: MfmValue + Serialize,
{
    let request_hash = digest_value(&request)?;
    let response_artifact = artifact_for_value(
        &response,
        events::ArtifactRole::FactResponse,
        Some(ctx.node().node_id.clone()),
    )?;
    let output_artifact = artifact_for_value(
        &output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_response = staged_attempt_artifact(&ctx, &response_artifact)?;
    let staged_output = staged_attempt_artifact(&ctx, &output_artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_response, staged_output],
        staged_retention_refs: vec![
            retention(&response_artifact.evidence),
            retention(&output_artifact.evidence),
        ],
        payloads: vec![
            events::KernelEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                capability_kind: PortfolioReadCapability::kind()
                    .map_err(runtime_capability_error)?,
                capability_version: PortfolioReadCapability::version()
                    .map_err(runtime_capability_error)?,
                adapter_kind: portfolio_adapter_kind()?,
                adapter_version: portfolio_adapter_version()?,
                request_schema_id: Request::schema_id().map_err(runtime_value_error)?,
                request_hash,
                response_schema_id: Response::schema_id().map_err(runtime_value_error)?,
                response_hash: response_artifact.evidence.digest.clone(),
                fact_key: events::FactKey::new(format!(
                    "mfm.portfolio.fact.{}",
                    ctx.node().node_id.as_str()
                ))?,
                artifact_id: response_artifact.evidence.artifact_id.clone(),
            }),
            cell_produced(&ctx, &output_artifact.evidence),
            completed(&ctx),
        ],
    })
}

async fn state_output<T>(
    ctx: ErasedRunCtx<'_>,
    value: &T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        value,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact = staged_attempt_artifact(&ctx, &artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence), completed(&ctx)],
    })
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &PortfolioArtifact,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_attempt_artifact(ctx, artifact.bytes.clone(), artifact.evidence.clone())
}

async fn load_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let (bytes, evidence) = artifacts
        .get_artifact_by_id(&ctx.node().config_ref.artifact_id)
        .await?;
    if evidence.digest != ctx.node().config_ref.digest
        || evidence.byte_len != ctx.node().config_ref.byte_len
        || evidence.media_type != ctx.node().config_ref.media_type
        || evidence.schema_id.as_ref() != Some(&ctx.node().config_ref.schema_id)
        || evidence.artifact_role != events::ArtifactRole::TypedConfig
    {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "portfolio config artifact did not match certified config ref for node {}",
            ctx.node().node_id
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_input_cell<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    load_value_from_node(&inputs.root, artifacts).await
}

async fn load_struct_input<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<T>
where
    T: DeserializeOwned,
{
    let value = materialized_node_json(&inputs.root, artifacts).await?;
    serde_json::from_value(value)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_non_empty_input<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<NonEmpty<T>>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::NonEmptyVec(elements) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "portfolio merge input was not a non-empty vector".to_owned(),
        ));
    };
    let mut values = Vec::with_capacity(elements.len());
    for element in elements {
        values.push(load_value_from_node(element, artifacts).await?);
    }
    NonEmpty::try_from_vec(values)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn materialized_node_json(
    node: &MaterializedInputNode,
    artifacts: &dyn PortfolioArtifactReader,
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

async fn load_value_from_node<T>(
    node: &MaterializedInputNode,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let bytes = load_cell_bytes(node, artifacts).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_cell_bytes(
    node: &MaterializedInputNode,
    artifacts: &dyn PortfolioArtifactReader,
) -> mfm_runtime::Result<Vec<u8>> {
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "portfolio input node was not a produced cell".to_owned(),
        ));
    };
    let (artifact_id, content_digest) = match &cell.terminal {
        MaterializedCellTerminal::Produced {
            artifact_id,
            content_digest,
        }
        | MaterializedCellTerminal::Seed {
            artifact_id,
            content_digest,
            ..
        } => (artifact_id, content_digest),
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "portfolio input cell was skipped".to_owned(),
            ))
        }
    };
    let (bytes, evidence) = artifacts.get_artifact_by_id(artifact_id).await?;
    if &evidence.digest != content_digest {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "portfolio input artifact digest did not match cell terminal".to_owned(),
        ));
    }
    Ok(bytes)
}

fn cell_produced(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::ArtifactEvidenceRef,
) -> events::KernelEventPayload {
    events::KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.output_cell().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
        schema_id: ctx.output_cell().schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: artifact.artifact_id.clone(),
        content_digest: artifact.digest.clone(),
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })
}

fn completed(ctx: &ErasedRunCtx<'_>) -> events::KernelEventPayload {
    events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        output_cell_id: ctx.node().output_cell.clone(),
    })
}

struct PortfolioArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<PortfolioArtifact>
where
    T: MfmValue + Serialize,
{
    let bytes = canonical_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(T::schema_id().map_err(runtime_value_error)?),
        semantic_type_id: Some(T::semantic_id().map_err(runtime_value_error)?),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(PortfolioArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn retention(artifact: &store::ArtifactEvidenceRef) -> StagedRetentionRefs {
    StagedRetentionRefs::runtime_evidence(vec![events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }])
}

fn canonical_value<T: Serialize>(value: &T) -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))
}

fn digest_value<T: Serialize>(value: &T) -> mfm_runtime::Result<ContentDigest> {
    Ok(canonical_value(value)?.content_digest())
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

fn runtime_value_error(error: mfm_values::ValueError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
struct EnvRpcSource {
    id: String,
    network_id: Option<String>,
    rpc_url: String,
    authorization: Option<String>,
}

#[derive(Clone)]
struct RpcSource {
    network_id: String,
    rpc_url: String,
    authorization: Option<String>,
}

#[derive(Clone)]
struct PortfolioRpcClient {
    client: reqwest::Client,
    sources: Vec<RpcSource>,
}

impl PortfolioRpcClient {
    fn from_env() -> Self {
        let sources = std::env::var(ENV_EVM_RPC_SOURCES_JSON)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<EnvRpcSource>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|source| {
                let network_id = source.network_id?;
                if source.id.trim().is_empty()
                    || network_id.trim().is_empty()
                    || source.rpc_url.trim().is_empty()
                {
                    return None;
                }
                Some(RpcSource {
                    network_id,
                    rpc_url: source.rpc_url,
                    authorization: source.authorization,
                })
            })
            .collect();
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            sources,
        }
    }

    async fn read_evm_block_number(&self, network_id: &str) -> Result<u64, PortfolioReadError> {
        let value = self
            .call(network_id, "eth_blockNumber", serde_json::json!([]))
            .await?;
        let raw = value.as_str().ok_or_else(|| {
            PortfolioReadError::new(
                "evm_response_invalid",
                "eth_blockNumber response was not a hex string",
            )
        })?;
        parse_hex_u64(raw).map_err(|message| {
            PortfolioReadError::new(
                "evm_response_invalid",
                format!("eth_blockNumber response was invalid: {message}"),
            )
        })
    }

    async fn read_raw_balance(
        &self,
        config: &ObserveBatchConfig,
        block_number: u64,
    ) -> Result<(U256, u8), PortfolioReadError> {
        match &config.symbol.balance_reader {
            mfm_portfolio_model::symbol::BalanceReaderConfig::NativeBalance {} => {
                let value = self
                    .call(
                        &config.network.network_id,
                        "eth_getBalance",
                        serde_json::json!([config.wallet.address, u64_hex_quantity(block_number)]),
                    )
                    .await?;
                let raw = parse_u256_hex_value(&value).map_err(|_| {
                    PortfolioReadError::new(
                        "evm_response_invalid",
                        "native balance response was invalid",
                    )
                })?;
                Ok((raw, config.symbol.decimals.unwrap_or(18)))
            }
            mfm_portfolio_model::symbol::BalanceReaderConfig::Erc20Balance { token_address } => {
                let decimals = match config.symbol.decimals {
                    Some(decimals) => decimals,
                    None => {
                        self.erc20_decimals(&config.network.network_id, token_address, block_number)
                            .await?
                    }
                };
                let wallet: Address = config.wallet.address.parse().map_err(|_| {
                    PortfolioReadError::new(
                        "invalid_wallet_address",
                        "wallet address was invalid for ERC-20 balance read",
                    )
                })?;
                let value = self
                    .call(
                        &config.network.network_id,
                        "eth_call",
                        serde_json::json!([
                            {"to": token_address, "data": encode_erc20_balance_of(&wallet)},
                            u64_hex_quantity(block_number)
                        ]),
                    )
                    .await?;
                let raw = parse_u256_hex_value(&value).map_err(|_| {
                    PortfolioReadError::new(
                        "evm_response_invalid",
                        "token balance response was invalid",
                    )
                })?;
                Ok((raw, decimals))
            }
            mfm_portfolio_model::symbol::BalanceReaderConfig::ProtocolPosition { .. } => {
                Err(PortfolioReadError::new(
                    "unsupported_balance_reader",
                    "protocol position reads are not enabled in the typed portfolio runner",
                ))
            }
        }
    }

    async fn erc20_decimals(
        &self,
        network_id: &str,
        token_address: &str,
        block_number: u64,
    ) -> Result<u8, PortfolioReadError> {
        let value = self
            .call(
                network_id,
                "eth_call",
                serde_json::json!([
                    {"to": token_address, "data": encode_erc20_decimals()},
                    u64_hex_quantity(block_number)
                ]),
            )
            .await?;
        let raw = parse_u256_hex_value(&value).map_err(|_| {
            PortfolioReadError::new(
                "evm_response_invalid",
                "token decimals response was invalid",
            )
        })?;
        parse_u8_u256(raw).map_err(|_| {
            PortfolioReadError::new(
                "evm_response_invalid",
                "token decimals response was out of range",
            )
        })
    }

    async fn call(
        &self,
        network_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PortfolioReadError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.network_id == network_id)
            .ok_or_else(|| {
                PortfolioReadError::new(
                    "rpc_source_missing",
                    format!("no typed EVM RPC source configured for network `{network_id}`"),
                )
            })?;
        let mut request = self.client.post(&source.rpc_url).json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1u64,
            "method": method,
            "params": params,
        }));
        if let Some(authorization) = &source.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        let response = request.send().await.map_err(|_| {
            PortfolioReadError::new(
                "evm_http_request_failed",
                format!("EVM JSON-RPC request `{method}` failed"),
            )
        })?;
        if !response.status().is_success() {
            return Err(PortfolioReadError::new(
                "evm_http_status",
                format!("EVM JSON-RPC request `{method}` returned non-success status"),
            ));
        }
        let body = response.json::<serde_json::Value>().await.map_err(|_| {
            PortfolioReadError::new(
                "evm_response_invalid_json",
                format!("EVM JSON-RPC response for `{method}` was invalid JSON"),
            )
        })?;
        if body.get("error").is_some() {
            return Err(PortfolioReadError::new(
                "evm_jsonrpc_error",
                format!("EVM JSON-RPC method `{method}` returned an error"),
            ));
        }
        body.get("result").cloned().ok_or_else(|| {
            PortfolioReadError::new(
                "evm_jsonrpc_missing_result",
                format!("EVM JSON-RPC response for `{method}` had no result"),
            )
        })
    }
}

impl PortfolioReadBackend for PortfolioRpcClient {
    fn evm_block_number<'a>(&'a self, network_id: &'a str) -> PortfolioReadFuture<'a, u64> {
        Box::pin(async move { self.read_evm_block_number(network_id).await })
    }

    fn observe_raw_balance<'a>(
        &'a self,
        config: &'a ObserveBatchConfig,
        block_number: u64,
    ) -> PortfolioReadFuture<'a, (U256, u8)> {
        Box::pin(async move { self.read_raw_balance(config, block_number).await })
    }
}

fn parse_hex_u64(raw: &str) -> Result<u64, &'static str> {
    let Some(rest) = raw.strip_prefix("0x") else {
        return Err("missing 0x prefix");
    };
    if rest.is_empty() || rest.len() > 16 || !rest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("not a u64 hex quantity");
    }
    u64::from_str_radix(rest, 16).map_err(|_| "not a u64 hex quantity")
}

fn portfolio_read_runtime_error(error: PortfolioReadError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}
