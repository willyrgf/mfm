#![warn(missing_docs)]
//! EVM transaction, exact-anchor validation, and collector adapter runners.
//!
//! Binds reusable EVM native-balance and ERC-20 state contracts to block, balance, and generic
//! call-read capabilities. Read states own immutable plans and deterministic reducers; this crate
//! binds those plans to live, source-stable sessions. Protocol IO stays in the transport crate.
//! The transaction binding owns one-transaction prepare, sign, submit, recovery,
//! receipt/finality observation, and evidence-only replay.

use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmCapabilityError, EvmNetworkBinding, EvmReadCapability, EvmReadSession,
    ProviderDiagnosticCode, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::LocalPublicId;
use mfm_program::{ManagedWriteState, MfmFactType, StateSpec};
use mfm_replay::v1::{
    self as replay, decode_produced_value as decode_replay_value,
    load_node_config as replay_node_config, produced_input_frames as replay_input_frames,
};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    ExternalReadExecution, ExternalReadExecutionFuture, ExternalReadPlanExecutor,
    ExternalReadRunner, RunnerCapabilityBinding, RunnerExecutableIdentityTemplate,
    RunnerIngressContext, RunnerOutputBuilder, RunnerRegistrationBuilder,
};
use mfm_states_evm::{
    assemble_evm_erc20_balance_batch_receipt, assemble_evm_native_balance_batch_receipt,
    assemble_evm_network_collection_receipt, erc20_balance_record_visibility,
    evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version, native_balance_record_visibility,
    AssembleEvmErc20BalanceBatchReceiptConfig, AssembleEvmErc20BalanceBatchReceiptInput,
    AssembleEvmErc20BalanceBatchReceiptState, AssembleEvmNativeBalanceBatchReceiptConfig,
    AssembleEvmNativeBalanceBatchReceiptInput, AssembleEvmNativeBalanceBatchReceiptState,
    AssembleEvmNetworkCollectionReceiptConfig, AssembleEvmNetworkCollectionReceiptInput,
    AssembleEvmNetworkCollectionReceiptState, EvmAddressErc20BalanceObservation,
    EvmAddressErc20BalanceSnapshotFact, EvmAddressNativeBalanceObservation,
    EvmAddressNativeBalanceSnapshotFact, EvmCollectorCallReadEvidence, EvmCollectorCallReadPlan,
    EvmContractValidationEvidence, EvmContractValidationPlan, EvmErc20BalanceBatchReceipt,
    EvmJointTip, EvmJointTipReadEvidence, EvmJointTipReadPlan, EvmNativeBalanceBatchReceipt,
    EvmNativeBalanceReadEvidence, EvmNativeBalanceReadPlan, ObserveErc20BalanceConfig,
    ObserveErc20BalanceState, ObserveErc20TokenMetadataConfig, ObserveErc20TokenMetadataState,
    ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceState, RecordErc20BalanceFactState,
    RecordEvmNativeBalanceFactState, RedactedEvmSessionEvidence, ResolveEvmJointTipConfig,
    ResolveEvmJointTipState, ValidateEvmContractState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmValue, NonEmpty};

mod transaction;

pub use transaction::{
    is_evm_transaction_replay_intent, register_evm_transaction_runner,
    verify_evm_transaction_replay, EvmSigningProviderBindFuture, EvmTransactionRunnerCapabilities,
    EvmTransactionSessionBindFuture,
};

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";

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
        Arc::new(ExternalReadRunner::<ResolveEvmJointTipState, _>::new(
            artifacts.clone(),
            ResolveJointTipExecutor {
                capabilities: capabilities.clone(),
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<ObserveEvmNativeBalanceState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ObserveEvmNativeBalanceState, _>::new(
            artifacts.clone(),
            ObserveNativeBalanceExecutor {
                capabilities: capabilities.clone(),
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20TokenMetadataState>(
        &read_factory,
        Arc::new(
            ExternalReadRunner::<ObserveErc20TokenMetadataState, _>::new(
                artifacts.clone(),
                EvmCollectorCallExecutor {
                    capabilities: capabilities.clone(),
                },
            ),
        ),
    )?;
    registrations.register_state_runner_with_factory::<ObserveErc20BalanceState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ObserveErc20BalanceState, _>::new(
            artifacts.clone(),
            EvmCollectorCallExecutor {
                capabilities: capabilities.clone(),
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<ValidateEvmContractState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ValidateEvmContractState, _>::new(
            artifacts.clone(),
            ValidateContractExecutor { capabilities },
        )),
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

struct ResolveJointTipExecutor {
    capabilities: EvmRunnerCapabilities,
}

impl ExternalReadPlanExecutor<ResolveEvmJointTipState> for ResolveJointTipExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &ResolveEvmJointTipState,
    ) -> mfm_runtime::Result<()> {
        let binding = joint_tip_binding(state.config()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmJointTipReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmJointTipReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(evm_state_runtime_error)?;
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let selector = plan.selector();
            let block = session
                .read_block(&selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            EvmJointTipReadEvidence::from_response(&block, session.evidence())
                .map(ExternalReadExecution::primary)
                .map_err(evm_state_runtime_error)
        })
    }
}

struct ObserveNativeBalanceExecutor {
    capabilities: EvmRunnerCapabilities,
}

impl ExternalReadPlanExecutor<ObserveEvmNativeBalanceState> for ObserveNativeBalanceExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &ObserveEvmNativeBalanceState,
    ) -> mfm_runtime::Result<()> {
        let binding = balance_binding(state.config()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmNativeBalanceReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmNativeBalanceReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(evm_state_runtime_error)?;
            let account = plan.account_address().map_err(evm_state_runtime_error)?;
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;
            let balance_selector = plan.balance_selector().map_err(evm_state_runtime_error)?;
            let balance = session
                .read_balance(account, &balance_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            let verification_selector = plan
                .canonicality_selector()
                .map_err(evm_state_runtime_error)?;
            let verified_block = session
                .read_block(&verification_selector)
                .await
                .map_err(evm_capability_runtime_error)?;
            EvmNativeBalanceReadEvidence::from_responses(
                balance,
                &verified_block,
                session.evidence(),
            )
            .map(ExternalReadExecution::primary)
            .map_err(evm_state_runtime_error)
        })
    }
}

struct EvmCollectorCallExecutor {
    capabilities: EvmRunnerCapabilities,
}

impl ExternalReadPlanExecutor<ObserveErc20TokenMetadataState> for EvmCollectorCallExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &ObserveErc20TokenMetadataState,
    ) -> mfm_runtime::Result<()> {
        let binding = erc20_metadata_binding(state.config()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmCollectorCallReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmCollectorCallReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(evm_state_runtime_error)?;
            let request = plan.request().map_err(evm_state_runtime_error)?;
            let verification_selector = plan
                .canonicality_selector()
                .map_err(evm_state_runtime_error)?;
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
            EvmCollectorCallReadEvidence::from_responses(
                &response,
                &verification,
                session.evidence(),
            )
            .map(ExternalReadExecution::primary)
            .map_err(evm_state_runtime_error)
        })
    }
}

impl ExternalReadPlanExecutor<ObserveErc20BalanceState> for EvmCollectorCallExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &ObserveErc20BalanceState,
    ) -> mfm_runtime::Result<()> {
        let binding = erc20_balance_binding(state.config()).map_err(evm_adapter_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmCollectorCallReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmCollectorCallReadEvidence> {
        Box::pin(async move {
            let binding = plan.binding().map_err(evm_state_runtime_error)?;
            let request = plan.request().map_err(evm_state_runtime_error)?;
            let verification_selector = plan
                .canonicality_selector()
                .map_err(evm_state_runtime_error)?;
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
            EvmCollectorCallReadEvidence::from_responses(
                &response,
                &verification,
                session.evidence(),
            )
            .map(ExternalReadExecution::primary)
            .map_err(evm_state_runtime_error)
        })
    }
}

struct ValidateContractExecutor {
    capabilities: EvmRunnerCapabilities,
}

impl ExternalReadPlanExecutor<ValidateEvmContractState> for ValidateContractExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &ValidateEvmContractState,
    ) -> mfm_runtime::Result<()> {
        let binding = state
            .config()
            .network_binding()
            .map_err(evm_state_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmContractValidationPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmContractValidationEvidence> {
        Box::pin(async move {
            let binding = network_binding(plan.network_id(), plan.chain_id())
                .map_err(evm_adapter_runtime_error)?;
            let session = self
                .capabilities
                .bind_evm_read_session(binding)
                .await
                .map_err(evm_capability_runtime_error)?;

            let (code_address, code_selector) =
                plan.code_request().map_err(evm_state_runtime_error)?;
            let code = session
                .read_code(code_address, &code_selector)
                .await
                .map_err(evm_capability_runtime_error)?;

            let mut calls = Vec::with_capacity(plan.calls().len());
            for request in plan.call_requests().map_err(evm_state_runtime_error)? {
                let response = session
                    .call(&request)
                    .await
                    .map_err(evm_capability_runtime_error)?;
                calls.push((request, response));
            }

            let canonicality_selector = plan
                .canonicality_selector()
                .map_err(evm_state_runtime_error)?;
            let canonical_block = session
                .read_block(&canonicality_selector)
                .await
                .map_err(evm_capability_runtime_error)?;

            EvmContractValidationEvidence::from_observations(
                code_address,
                &code_selector,
                &code,
                &calls,
                &canonical_block,
                session.evidence(),
            )
            .map(ExternalReadExecution::primary)
            .map_err(evm_state_runtime_error)
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
    replay::verify_external_read_state::<ResolveEvmJointTipState>(broker)?;
    replay::verify_external_read_state::<ObserveEvmNativeBalanceState>(broker)?;
    replay::verify_external_read_state::<ObserveErc20TokenMetadataState>(broker)?;
    replay::verify_external_read_state::<ObserveErc20BalanceState>(broker)?;
    replay::verify_external_read_state::<ValidateEvmContractState>(broker)?;
    let native_frames = replay_frames_for_state::<ObserveEvmNativeBalanceState>(broker)?;
    verify_evm_native_balance_fact_replay(broker)?;

    let metadata_frames = replay_frames_for_state::<ObserveErc20TokenMetadataState>(broker)?;
    let balance_frames = replay_frames_for_state::<ObserveErc20BalanceState>(broker)?;
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
