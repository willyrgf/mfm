#![warn(missing_docs)]
//! Bitcoin JSON-RPC adapter runners.
//!
//! This crate binds reusable Bitcoin state contracts to explicit Bitcoin read capabilities and
//! the reusable Bitcoin Core JSON-RPC HTTP transport. Protocol IO stays in the transport crate;
//! states own deterministic plans and reducers, while this crate owns live capability binding,
//! runner registration, and fact recording.

use std::marker::PhantomData;
use std::sync::Arc;

#[path = "replay.rs"]
mod replay_verification;
pub use self::replay_verification::verify_btc_jsonrpc_replay;

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcBalanceReadCapability, BtcBalanceReadProvider, BtcCapabilityError,
    BtcChainHeadReadCapability, BtcChainHeadReadProvider, BtcNetworkId, BtcSourceBinding,
    BtcSourceIdentity, ProviderDiagnosticCode,
};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{FactIndexReadProvider, FactRecordCapability};
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    ExternalReadExecution, ExternalReadExecutionFuture, ExternalReadPlanExecutor,
    ExternalReadRunner, RunnerCapabilityBinding, RunnerExecutableIdentityTemplate,
    RunnerIngressContext, RunnerOutputBuilder, RunnerRegistrationBuilder,
};
use mfm_states_btc::{
    address_balance_record_visibility, assemble_btc_network_collection_receipt,
    btc_jsonrpc_adapter_kind, btc_jsonrpc_adapter_version, chain_head_fact_visibility,
    collector_checkpoint_fact_visibility, AssembleBtcNetworkCollectionReceiptConfig,
    AssembleBtcNetworkCollectionReceiptInput, AssembleBtcNetworkCollectionReceiptState,
    BtcAddressBalanceReadEvidence, BtcAddressBalanceReadPlan, BtcAddressBalanceSnapshotFact,
    BtcChainHeadFact, BtcChainHeadReadEvidence, BtcChainHeadReadPlan, CollectorCheckpointResponse,
    ObserveBtcAddressBalanceConfig, ObserveBtcAddressBalanceState, ObserveBtcChainHeadConfig,
    ObserveBtcChainHeadState, QueryCollectorCheckpointReadEvidence,
    QueryCollectorCheckpointReadPlan, QueryCollectorCheckpointState,
    RecordBtcAddressBalanceFactState, RecordBtcChainHeadFactState, RecordCollectorCheckpointState,
    ResolveBtcJointTipConfig, ResolveBtcJointTipState,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "btc_jsonrpc_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.bitcoin.jsonrpc.runtime.v1";

/// Result type for Bitcoin JSON-RPC adapter operations.
pub type Result<T> = std::result::Result<T, BtcJsonRpcAdapterError>;

/// Future returned by asynchronous Bitcoin source-route validation.
pub type BtcSourceBindingValidationFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = mfm_btc_capabilities::Result<()>> + Send + 'a>,
>;

/// Factory for Bitcoin chain-head and balance providers bound to a certified source binding.
pub trait BtcChainHeadProviderFactory: Send + Sync {
    /// Asynchronously validates that the binding can resolve without live network IO.
    fn validate_source_binding<'a>(
        &'a self,
        binding: BtcSourceBinding,
    ) -> BtcSourceBindingValidationFuture<'a>;

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
        Arc::new(ExternalReadRunner::<ObserveBtcChainHeadState, _>::new(
            artifacts.clone(),
            ChainHeadExecutor { btc: btc.clone() },
        )),
    )?;
    registrations.register_state_runner_with_factory::<ResolveBtcJointTipState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ResolveBtcJointTipState, _>::new(
            artifacts.clone(),
            JointTipExecutor { btc: btc.clone() },
        )),
    )?;
    registrations.register_state_runner_with_factory::<ObserveBtcAddressBalanceState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ObserveBtcAddressBalanceState, _>::new(
            artifacts.clone(),
            AddressBalanceExecutor { btc },
        )),
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
        Arc::new(ExternalReadRunner::<QueryCollectorCheckpointState, _>::new(
            artifacts.clone(),
            QueryCheckpointExecutor {
                artifacts: artifacts.clone(),
                fact_index,
            },
        )),
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
    registrations.register_state_runner_with_factory::<AssembleBtcNetworkCollectionReceiptState>(
        &pure_factory,
        Arc::new(AssembleNetworkCollectionReceiptRunner { artifacts }),
    )?;
    Ok(())
}

struct AssembleNetworkCollectionReceiptRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleNetworkCollectionReceiptRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<AssembleBtcNetworkCollectionReceiptConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleBtcNetworkCollectionReceiptInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let receipt = assemble_btc_network_collection_receipt(input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &receipt)
        })
    }
}

/// Redaction-safe Bitcoin JSON-RPC adapter error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcJsonRpcAdapterError {
    /// Adapter could not build a capability request from certified state config.
    #[error("Bitcoin adapter could not build capability request")]
    InvalidCapabilityRequest,
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

struct ChainHeadExecutor {
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ExternalReadPlanExecutor<ObserveBtcChainHeadState> for ChainHeadExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a ObserveBtcChainHeadState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move {
            let binding = chain_head_binding(state.config()).map_err(btc_adapter_runtime_error)?;
            self.btc
                .validate_source_binding(binding)
                .await
                .map_err(btc_capability_runtime_error)
        })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a BtcChainHeadReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, BtcChainHeadReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(btc_state_runtime_error)?;
            let request = plan.request().map_err(btc_state_runtime_error)?;
            let btc = self
                .btc
                .bind_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_chain_head(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            Ok(ExternalReadExecution::primary(
                BtcChainHeadReadEvidence::from_response(&response),
            ))
        })
    }
}

struct JointTipExecutor {
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ExternalReadPlanExecutor<ResolveBtcJointTipState> for JointTipExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a ResolveBtcJointTipState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move {
            let binding = joint_tip_binding(state.config()).map_err(btc_adapter_runtime_error)?;
            self.btc
                .validate_source_binding(binding)
                .await
                .map_err(btc_capability_runtime_error)
        })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a BtcChainHeadReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, BtcChainHeadReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(btc_state_runtime_error)?;
            let request = plan.request().map_err(btc_state_runtime_error)?;
            let btc = self
                .btc
                .bind_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_chain_head(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            Ok(ExternalReadExecution::primary(
                BtcChainHeadReadEvidence::from_response(&response),
            ))
        })
    }
}

struct AddressBalanceExecutor {
    btc: Arc<dyn BtcChainHeadProviderFactory>,
}

impl ExternalReadPlanExecutor<ObserveBtcAddressBalanceState> for AddressBalanceExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a ObserveBtcAddressBalanceState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move {
            let binding =
                address_balance_binding(state.config()).map_err(btc_adapter_runtime_error)?;
            self.btc
                .validate_source_binding(binding)
                .await
                .map_err(btc_capability_runtime_error)
        })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a BtcAddressBalanceReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, BtcAddressBalanceReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(btc_state_runtime_error)?;
            let request = plan.request().map_err(btc_state_runtime_error)?;
            let btc = self
                .btc
                .bind_balance_source(binding)
                .map_err(btc_capability_runtime_error)?;
            let response = btc
                .read_balance(&request)
                .await
                .map_err(btc_capability_runtime_error)?;
            Ok(ExternalReadExecution::primary(
                BtcAddressBalanceReadEvidence::from_response(&response),
            ))
        })
    }
}

struct QueryCheckpointExecutor {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ExternalReadPlanExecutor<QueryCollectorCheckpointState> for QueryCheckpointExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        _state: &'a QueryCollectorCheckpointState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a QueryCollectorCheckpointReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, QueryCollectorCheckpointReadEvidence> {
        Box::pin(async move {
            let request = plan.request().map_err(btc_state_runtime_error)?;
            let response = self
                .fact_index
                .read_fact_index(&request)
                .await
                .map_err(fact_index_runtime_error)?;
            let checkpoint = match plan
                .selected_response_ref(&response)
                .map_err(btc_state_runtime_error)?
            {
                None => None,
                Some(fact_ref) => {
                    let requirement = fact_response_artifact_requirement(fact_ref);
                    let artifact = self
                        .artifacts
                        .read_retained_artifact(&requirement)
                        .await
                        .map_err(runtime_artifact_read_error)?;
                    Some(
                        hydrate_fact_response_json::<CollectorCheckpointResponse>(
                            fact_ref,
                            artifact.bytes(),
                        )
                        .map_err(|error| {
                            mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                        })?,
                    )
                }
            };
            let selection = plan
                .selection_evidence(&response)
                .map_err(btc_state_runtime_error)?;
            let query = mfm_facts::FactQueryEvidence::new(
                request.plan().clone(),
                response.receipt().clone(),
                selection,
            );
            let primary = QueryCollectorCheckpointReadEvidence::new(&query, checkpoint)
                .map_err(btc_state_runtime_error)?;
            Ok(ExternalReadExecution::new(primary, vec![query]))
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
    let Some(diagnostic) = error.redacted_diagnostic().cloned() else {
        return mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "Bitcoin capability request failed without provider diagnostics".to_owned(),
        );
    };
    let (code, message) = match diagnostic.code() {
        ProviderDiagnosticCode::ProviderConfigurationMissing
        | ProviderDiagnosticCode::RouteUnavailable => (
            "RuntimeConfigRequired",
            "Bitcoin runtime configuration is required",
        ),
        ProviderDiagnosticCode::ProviderConfigurationInvalid => (
            "RuntimeConfigInvalid",
            "Bitcoin runtime configuration is invalid",
        ),
        _ => (
            "BitcoinProviderFailure",
            "Bitcoin provider capability failed",
        ),
    };
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("Bitcoin runtime failure code is checked public text"),
        events::ErrorCategory::Capability,
        message,
        vec![diagnostic],
    )
    .expect("Bitcoin runtime failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
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
