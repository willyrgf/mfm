#![warn(missing_docs)]
//! Portfolio adapter runners.
//!
//! This crate binds certified portfolio state descriptors to typed runners over explicit artifact
//! and EVM capability contracts. Concrete artifact stores and live EVM transports are supplied by
//! app assembly.

use std::sync::Arc;

use alloy_primitives::{Address, U256};
use mfm_artifact_capabilities::{ArtifactReadProvider, ArtifactReadRequest};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilitySetDescriptor, CapabilitySpec};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBlockReadProvider, EvmBlockReadRequest,
    EvmBlockSelector, EvmCallReadProvider, EvmCallReadRequest, EvmCapabilityError,
    EvmCapabilityFuture, EvmSourcePolicyId, EvmSourceRef,
};
use mfm_evm_core::encoding::{encode_erc20_balance_of, encode_erc20_decimals, parse_u8_u256};
use mfm_evm_core::hex::hex_to_bytes;
use mfm_ids::{ArtifactId, ContentDigest, DescriptorId, NodeId, SemanticTypeId};
use mfm_program::{StateSpec, ValidatedConfig};
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCellTerminal,
    MaterializedInputNode, RunnerEventPayload, StagedArtifact, StagedRetentionRefs,
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
    ResolveSubjectsConfig, ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
    SourcePreparationRequest, SourcePreparationResponse, ViewPinRequest, ViewPinResponse,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue, NonEmpty};
use serde::de::DeserializeOwned;
use serde::Serialize;

const READ_FACTORY: &str = "read_external";
const PURE_FACTORY: &str = "pure";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.portfolio.runtime.v1";

/// EVM read capabilities required by portfolio adapter runners.
pub trait PortfolioEvmProvider:
    EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

impl<T> PortfolioEvmProvider for T where
    T: EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

/// EVM provider used when portfolio runtime sources are not configured.
#[derive(Debug, Clone, Default)]
pub struct UnavailablePortfolioEvmProvider;

impl EvmBlockReadProvider for UnavailablePortfolioEvmProvider {
    fn read_block<'a>(
        &'a self,
        _request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmBlockReadResponse> {
        unavailable_evm()
    }
}

impl EvmBalanceReadProvider for UnavailablePortfolioEvmProvider {
    fn read_balance<'a>(
        &'a self,
        _request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmBalanceReadResponse> {
        unavailable_evm()
    }
}

impl EvmCallReadProvider for UnavailablePortfolioEvmProvider {
    fn read_call<'a>(
        &'a self,
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmCallReadResponse> {
        unavailable_evm()
    }
}

fn unavailable_evm<'a, T>() -> EvmCapabilityFuture<'a, T> {
    Box::pin(async {
        Err(EvmCapabilityError::redacted_provider_failure(
            "portfolio EVM provider unavailable",
        ))
    })
}

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn ArtifactReadProvider>,
    evm: Arc<dyn PortfolioEvmProvider>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and EVM providers.
    pub fn new(
        artifacts: Arc<dyn ArtifactReadProvider>,
        evm: Arc<dyn PortfolioEvmProvider>,
    ) -> Self {
        Self { artifacts, evm }
    }

    fn artifacts(&self) -> Arc<dyn ArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn evm(&self) -> Arc<dyn PortfolioEvmProvider> {
        Arc::clone(&self.evm)
    }
}

/// Registers typed portfolio runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let evm = capabilities.evm();
    let prepare_sources = registered_descriptor::<PrepareSourcesState>()?;
    registry.register_capability_set(&prepare_sources.capabilities, implementation_id.clone())?;
    registry.register(binding(
        prepare_sources.descriptor_id,
        READ_FACTORY,
        Arc::new(PrepareSourcesRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    let resolve_subjects = registered_descriptor::<ResolveSubjectsState>()?;
    registry.register_capability_set(&resolve_subjects.capabilities, implementation_id.clone())?;
    registry.register(binding(
        resolve_subjects.descriptor_id,
        PURE_FACTORY,
        Arc::new(ResolveSubjectsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    let pin_views = registered_descriptor::<PinViewsState>()?;
    registry.register_capability_set(&pin_views.capabilities, implementation_id.clone())?;
    registry.register(binding(
        pin_views.descriptor_id,
        READ_FACTORY,
        Arc::new(PinViewsRunner {
            artifacts: artifacts.clone(),
            evm: evm.clone(),
        }),
    )?)?;
    let resolve_valuations = registered_descriptor::<ResolveValuationsState>()?;
    registry
        .register_capability_set(&resolve_valuations.capabilities, implementation_id.clone())?;
    registry.register(binding(
        resolve_valuations.descriptor_id,
        PURE_FACTORY,
        Arc::new(ResolveValuationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    let observe_batch = registered_descriptor::<ObserveBatchState>()?;
    registry.register_capability_set(&observe_batch.capabilities, implementation_id.clone())?;
    registry.register(binding(
        observe_batch.descriptor_id,
        READ_FACTORY,
        Arc::new(ObserveBatchRunner {
            artifacts: artifacts.clone(),
            evm,
        }),
    )?)?;
    let merge_observations = registered_descriptor::<MergeObservationsState>()?;
    registry
        .register_capability_set(&merge_observations.capabilities, implementation_id.clone())?;
    registry.register(binding(
        merge_observations.descriptor_id,
        PURE_FACTORY,
        Arc::new(MergeObservationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    let assemble_snapshot = registered_descriptor::<AssembleSnapshotState>()?;
    registry.register_capability_set(&assemble_snapshot.capabilities, implementation_id.clone())?;
    registry.register(binding(
        assemble_snapshot.descriptor_id,
        PURE_FACTORY,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    let project_report = registered_descriptor::<ProjectReportState>()?;
    registry.register_capability_set(&project_report.capabilities, implementation_id)?;
    registry.register(binding(
        project_report.descriptor_id,
        PURE_FACTORY,
        Arc::new(ProjectReportRunner { artifacts }),
    )?)?;
    Ok(())
}

struct RegisteredRuntimeDescriptor {
    descriptor_id: DescriptorId,
    capabilities: CapabilitySetDescriptor,
}

fn registered_descriptor<S>() -> mfm_runtime::Result<RegisteredRuntimeDescriptor>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let mut states = mfm_program::StateRegistryBuilder::new();
    let registered = states
        .register::<S>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(RegisteredRuntimeDescriptor {
        descriptor_id: registered.descriptor().descriptor_id().clone(),
        capabilities: registered.descriptor().capabilities().clone(),
    })
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
        source_revision: events::SourceRevision::new("mfm-adapters-portfolio-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-adapters-portfolio")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-portfolio",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-portfolio",
            "runner": "typed-portfolio",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct PrepareSourcesRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for PrepareSourcesRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<PrepareSourcesConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let prepared = prepare_sources_from_config(config);
            let request = SourcePreparationRequest {
                network_ids: config
                    .networks()
                    .iter()
                    .map(|network| network.network_id().to_string())
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
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveSubjectsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<ResolveSubjectsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let _prepared = load_input_cell::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_subjects_from_config(config);
            state_output(ctx, &output).await
        })
    }
}

struct PinViewsRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
    evm: Arc<dyn PortfolioEvmProvider>,
}

impl ErasedNodeRunner for PinViewsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<PinViewsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let _prepared = load_input_cell::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let backend = EvmCapabilityPortfolioBackend::new(Arc::clone(&self.evm));
            let output = pin_views_with_backend(config, &backend)
                .await
                .map_err(portfolio_read_runtime_error)?;
            let request = ViewPinRequest {
                network_ids: config
                    .networks()
                    .iter()
                    .map(|network| network.network_id().to_string())
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
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveValuationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<ResolveValuationsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let views = load_input_cell::<mfm_state_portfolio::PinnedViews>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_valuations_from_config(config, &views);
            state_output(ctx, &output).await
        })
    }
}

struct ObserveBatchRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
    evm: Arc<dyn PortfolioEvmProvider>,
}

impl ErasedNodeRunner for ObserveBatchRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<ObserveBatchConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input =
                load_struct_input::<ObserveBatchInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let block_number = evm_block_number_for(&input.views, config.network().network_id());
            let backend = EvmCapabilityPortfolioBackend::new(Arc::clone(&self.evm));
            let output = observe_batch_with_backend(config, &input, &backend).await;
            let request = ObservationRequest {
                wallet_id: config.wallet().wallet_id.to_string(),
                symbol_id: config.symbol().symbol_id.to_string(),
                network_id: config.network().network_id().to_string(),
                balance_reader_kind: balance_reader_kind(&config.symbol().balance_reader)
                    .to_owned(),
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
    artifacts: Arc<dyn ArtifactReadProvider>,
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
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_config::<AssembleSnapshotConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input =
                load_struct_input::<AssembleSnapshotInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::assemble_snapshot(config, input, 0);
            state_output(ctx, &output).await
        })
    }
}

struct ProjectReportRunner {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl ErasedNodeRunner for ProjectReportRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_config::<ProjectReportConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input =
                load_struct_input::<ProjectReportInput>(ctx.inputs(), self.artifacts.as_ref())
                    .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(
                input.snapshot,
                config.report_version(),
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
            RunnerEventPayload::FactRecorded(events::FactRecorded {
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
        payloads: vec![cell_produced(&ctx, &artifact.evidence)],
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
            "portfolio config failed validation: {error}"
        ))
    })
}

async fn load_input_cell<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    load_value_from_node(&inputs.root, artifacts).await
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

async fn load_non_empty_input<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn ArtifactReadProvider,
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

async fn load_value_from_node<T>(
    node: &MaterializedInputNode,
    artifacts: &dyn ArtifactReadProvider,
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
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<Vec<u8>> {
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "portfolio input node was not a produced cell".to_owned(),
        ));
    };
    let request = match &cell.terminal {
        MaterializedCellTerminal::Produced {
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
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
                "portfolio input cell was skipped".to_owned(),
            ))
        }
    };
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    Ok(verified.into_bytes())
}

fn cell_produced(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::ArtifactEvidenceRef,
) -> RunnerEventPayload {
    RunnerEventPayload::CellProduced(events::CellProduced {
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
        semantic_type_id: artifact_semantic_type_id_for_role::<T>(role)?,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(PortfolioArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn artifact_semantic_type_id_for_role<T>(
    role: events::ArtifactRole,
) -> mfm_runtime::Result<Option<SemanticTypeId>>
where
    T: MfmValue,
{
    match role.contract().semantic {
        events::ArtifactSemanticPolicy::OptionalLaunchSemantic
        | events::ArtifactSemanticPolicy::ExactSeedSemantic
        | events::ArtifactSemanticPolicy::ExactValueSemantic => {
            Ok(Some(T::semantic_id().map_err(runtime_value_error)?))
        }
        events::ArtifactSemanticPolicy::Absent => Ok(None),
    }
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

#[derive(Clone)]
struct EvmCapabilityPortfolioBackend {
    evm: Arc<dyn PortfolioEvmProvider>,
}

impl EvmCapabilityPortfolioBackend {
    fn new(evm: Arc<dyn PortfolioEvmProvider>) -> Self {
        Self { evm }
    }

    fn route(
        &self,
        network_id: &str,
    ) -> Result<(EvmSourceRef, EvmSourcePolicyId), PortfolioReadError> {
        Ok((
            EvmSourceRef::new(network_id).map_err(portfolio_evm_invalid_route)?,
            EvmSourcePolicyId::new(network_id).map_err(portfolio_evm_invalid_route)?,
        ))
    }

    async fn read_evm_block_number(&self, network_id: &str) -> Result<u64, PortfolioReadError> {
        let (source_ref, policy_id) = self.route(network_id)?;
        let response = self
            .evm
            .read_block(&EvmBlockReadRequest {
                source_ref,
                policy_id,
                block: EvmBlockSelector::Latest,
            })
            .await
            .map_err(portfolio_evm_capability_error)?;
        Ok(response.block_number)
    }

    async fn read_raw_balance(
        &self,
        config: &ObserveBatchConfig,
        block_number: u64,
    ) -> Result<(U256, u8), PortfolioReadError> {
        match &config.symbol().balance_reader {
            mfm_portfolio_model::symbol::BalanceReaderConfig::NativeBalance {} => {
                let wallet = wallet_evm_address(config)?;
                let (source_ref, policy_id) = self.route(config.network().network_id())?;
                let response = self
                    .evm
                    .read_balance(&EvmBalanceReadRequest {
                        source_ref,
                        policy_id,
                        account: wallet,
                        block: EvmBlockSelector::Number(block_number),
                    })
                    .await
                    .map_err(portfolio_evm_capability_error)?;
                Ok((response.balance_wei, config.symbol().decimals.unwrap_or(18)))
            }
            mfm_portfolio_model::symbol::BalanceReaderConfig::Erc20Balance { token_address } => {
                let decimals = match config.symbol().decimals {
                    Some(decimals) => decimals,
                    None => {
                        self.erc20_decimals(
                            config.network().network_id(),
                            token_address,
                            block_number,
                        )
                        .await?
                    }
                };
                let wallet = wallet_evm_address(config)?;
                let token = parse_address(token_address, "token address")?;
                let raw = self
                    .evm_call_u256(
                        config.network().network_id(),
                        token,
                        encode_erc20_balance_of(&wallet),
                        block_number,
                    )
                    .await?;
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
        let token = parse_address(token_address, "token address")?;
        let raw = self
            .evm_call_u256(network_id, token, encode_erc20_decimals(), block_number)
            .await?;
        parse_u8_u256(raw).map_err(|_| {
            PortfolioReadError::new(
                "evm_response_invalid",
                "token decimals response was out of range",
            )
        })
    }

    async fn evm_call_u256(
        &self,
        network_id: &str,
        to: Address,
        calldata_hex: String,
        block_number: u64,
    ) -> Result<U256, PortfolioReadError> {
        let (source_ref, policy_id) = self.route(network_id)?;
        let calldata = hex_to_bytes(&calldata_hex).map_err(|_| {
            PortfolioReadError::new("invalid_call_data", "portfolio EVM call data was invalid")
        })?;
        let response = self
            .evm
            .read_call(&EvmCallReadRequest {
                source_ref,
                policy_id,
                to,
                calldata,
                block: EvmBlockSelector::Number(block_number),
            })
            .await
            .map_err(portfolio_evm_capability_error)?;
        decode_u256_return(&response.return_data)
    }
}

impl PortfolioReadBackend for EvmCapabilityPortfolioBackend {
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

fn decode_u256_return(data: &[u8]) -> Result<U256, PortfolioReadError> {
    if data.len() != 32 {
        return Err(PortfolioReadError::new(
            "evm_response_invalid",
            "EVM call response was not a single uint256 word",
        ));
    }
    Ok(U256::from_be_slice(data))
}

fn parse_address(value: &str, label: &'static str) -> Result<Address, PortfolioReadError> {
    value.parse().map_err(|_| {
        PortfolioReadError::new(
            "invalid_evm_address",
            format!("{label} was invalid for portfolio EVM read"),
        )
    })
}

fn wallet_evm_address(config: &ObserveBatchConfig) -> Result<Address, PortfolioReadError> {
    let Some(address) = config.wallet().subject.evm_address() else {
        return Err(PortfolioReadError::new(
            "invalid_wallet_subject",
            "wallet subject was not an EVM address for portfolio EVM read",
        ));
    };
    parse_address(address, "wallet address")
}

fn portfolio_evm_invalid_route(error: EvmCapabilityError) -> PortfolioReadError {
    PortfolioReadError::new(
        "evm_route_invalid",
        format!("portfolio EVM source route was invalid: {error}"),
    )
}

fn portfolio_evm_capability_error(error: EvmCapabilityError) -> PortfolioReadError {
    PortfolioReadError::new(
        "evm_capability_failed",
        format!("portfolio EVM read capability failed: {error}"),
    )
}

fn runtime_artifact_read_error(
    error: mfm_artifact_capabilities::ArtifactReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn portfolio_read_runtime_error(error: PortfolioReadError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_identity_summary_matches_golden() {
        assert_eq!(
            executable_identity_summary([READ_FACTORY, PURE_FACTORY]),
            [
                "factory=read_external;source=mfm-adapters-portfolio-built-in;package=mfm-adapters-portfolio;version=0.1.0;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
                "factory=pure;source=mfm-adapters-portfolio-built-in;package=mfm-adapters-portfolio;version=0.1.0;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
            ]
        );
    }

    #[test]
    fn decode_config_bytes_rejects_invalid_serialized_config() {
        let error = decode_config_bytes::<ProjectReportConfig>(br#"{"report_version":0}"#)
            .expect_err("zero report version must fail decoding");
        let mfm_runtime::RuntimeError::InvalidRunnerOutput(message) = error else {
            panic!("unexpected error: {error}");
        };
        assert!(message.contains("nonzero u64"));
    }

    fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
        factories
            .into_iter()
            .map(|factory| {
                let identity =
                    executable(events::RunnerFactoryId::new(factory).expect("factory id"))
                        .expect("executable identity");
                format!(
                    "factory={};source={};package={};version={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                    identity.factory_id,
                    identity.source_revision,
                    identity.cargo_package_name,
                    identity.cargo_package_version,
                    identity.cargo_package_digest,
                    identity.binary_digest,
                    identity.nix_derivation_hash.is_some(),
                    identity.nix_output_hash.is_some()
                )
            })
            .collect()
    }
}
