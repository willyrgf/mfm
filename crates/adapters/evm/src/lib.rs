#![warn(missing_docs)]
//! EVM native collector adapter runners.
//!
//! Binds reusable EVM native-balance state contracts to block/balance read capabilities.
//! Protocol IO stays in the transport crate; this crate owns request mapping, runner
//! registration, and managed fact recording.

use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, B256};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBlockReadProvider, EvmBlockReadRequest,
    EvmBlockSelector, EvmCapabilityError, EvmNetworkBinding, EvmNetworkId,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec, ValidatedConfig};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config, load_materialized_struct_input, load_runner_config,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerOutputBuilder,
    RunnerRegistrationBuilder,
};
use mfm_states_evm::{
    assemble_evm_native_balance_batch, evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version,
    materialize_evm_joint_tip, native_balance_record_visibility,
    AssembleEvmNativeBalanceBatchConfig, AssembleEvmNativeBalanceBatchInput,
    AssembleEvmNativeBalanceBatchState, EvmAddressNativeBalanceObservation,
    ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput, ObserveEvmNativeBalanceState,
    RecordEvmNativeBalanceFactState, ResolveEvmJointTipConfig, ResolveEvmJointTipInput,
    ResolveEvmJointTipState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.evm.jsonrpc.runtime.v1";

/// Result type for EVM adapter operations.
pub type Result<T> = std::result::Result<T, EvmAdapterError>;

/// Factory for EVM network providers bound to a certified network binding.
pub trait EvmProviderFactory: Send + Sync {
    /// Validates that the binding can resolve without live network IO.
    fn validate_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()>;

    /// Binds a checked network binding to a provider that supports block and balance reads.
    fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmBoundProvider>>;
}

/// Bound EVM provider exposing the collector-needed read surfaces.
pub trait EvmBoundProvider: EvmBlockReadProvider + EvmBalanceReadProvider {}

impl<T> EvmBoundProvider for T where T: EvmBlockReadProvider + EvmBalanceReadProvider {}

/// Runtime capabilities used by EVM native collector runners.
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

/// Registers typed EVM native collector runners.
pub fn register_evm_collectors_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let evm = capabilities.evm();
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
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
    registrations.register_state_descriptor_with_factory::<ResolveEvmJointTipState>(
        &read_factory,
        Arc::new(ResolveJointTipRunner {
            artifacts: artifacts.clone(),
            evm: evm.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ObserveEvmNativeBalanceState>(
        &read_factory,
        Arc::new(ObserveNativeBalanceRunner {
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
    registrations.register_state_descriptor_with_factory::<AssembleEvmNativeBalanceBatchState>(
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
    network_binding(&config.network, config.chain_id)
}

fn parse_block_hash(value: &str) -> Result<B256> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
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
        let config = load_launch_config::<ResolveEvmJointTipConfig>(&ctx)?;
        let binding = joint_tip_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveEvmJointTipConfig>(&ctx, self.artifacts.as_ref())
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
            ErasedRunnerOutput::state_output(&ctx, &tip)
        })
    }
}

struct ObserveNativeBalanceRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn EvmProviderFactory>,
}

impl ErasedNodeRunner for ObserveNativeBalanceRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<ObserveEvmNativeBalanceConfig>(&ctx)?;
        let binding = balance_binding(config.as_ref()).map_err(evm_adapter_runtime_error)?;
        self.evm
            .validate_network_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ObserveEvmNativeBalanceConfig>(&ctx, self.artifacts.as_ref())
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
                &ResolveEvmJointTipConfig {
                    network: state.config().network.clone(),
                    chain_id: state.config().chain_id,
                    max_source_reads: state.config().max_source_reads,
                },
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
            ErasedRunnerOutput::state_output(&ctx, &observation)
        })
    }
}

struct AssembleNativeBalanceBatchRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleNativeBalanceBatchRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config::<AssembleEvmNativeBalanceBatchConfig>(
                &ctx,
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
            let config = load_runner_config::<S::Config>(&ctx, self.artifacts.as_ref()).await?;
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

/// Verifies EVM native-balance observation cell outputs when present in a broker stream.
///
/// Returns `Ok(false)` when the stream contains no matching observe outputs.
pub fn verify_evm_native_balance_replay(broker: &replay::ReplayBroker) -> replay::Result<bool> {
    let state_kind = ObserveEvmNativeBalanceState::kind().map_err(replay_adapter_error)?;
    let state_version = ObserveEvmNativeBalanceState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        verify_evm_native_balance_observation_replay(broker, frame)?;
    }
    Ok(!frames.is_empty())
}

fn verify_evm_native_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveEvmNativeBalanceConfig = replay_node_config(broker, &frame.node)?;
    let binding = balance_binding(&config).map_err(replay_adapter_error)?;
    let output: EvmAddressNativeBalanceObservation =
        serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    let subject = output.subject();
    let response = output.response();
    if subject.network() != config.network.as_str()
        || subject.chain_id() != config.chain_id
        || subject.account() != config.account.as_str()
        || output.source_read_count() != config.max_source_reads.get()
    {
        return Err(replay_evm_mismatch(
            "EVM native-balance observation did not match certified config binding",
        ));
    }
    if binding.network_id().as_str() != config.network.as_str()
        || binding.expected_chain_id() != config.chain_id
    {
        return Err(replay_evm_mismatch(
            "EVM native-balance observation binding did not match certified network",
        ));
    }
    // Response constructor already enforces 32-byte hex; re-check for decoded cell bytes.
    parse_block_hash(response.block_hash()).map_err(replay_adapter_error)?;
    parse_account(subject.account()).map_err(replay_adapter_error)?;
    if response.raw_wei().is_empty() || !response.raw_wei().chars().all(|c| c.is_ascii_digit()) {
        return Err(replay_evm_mismatch(
            "EVM native-balance observation raw_wei was invalid",
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

fn replay_evm_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
