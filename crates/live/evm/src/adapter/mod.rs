//! Runtime and replay bindings for the four reusable EVM balance, transaction, and validation
//! states. This adapter owns no operation topology or application entry point.

use std::sync::Arc;

use mfm_capabilities::ProviderDiagnosticCode;
use mfm_events::v1 as events;
use mfm_evm::{
    evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version, CollectEvmBalancesState,
    EvmCapabilityError, EvmCapabilityFailureDisposition, EvmCapabilityPhase,
    EvmContractValidationEvidence, EvmContractValidationEvidenceBuilder, EvmContractValidationPlan,
    EvmInvalidRequest, EvmNetworkBinding, EvmReadCapability, EvmReadSession, EvmReadSessionSet,
    ValidateEvmContractState,
};
use mfm_runtime::{
    CapabilityImplementationId, ErasedRunCtx, ErasedRunnerRegistry, ExternalReadExecution,
    ExternalReadExecutionFuture, ExternalReadPlanExecutor, ExternalReadRunner,
    RunnerFactoryBinding, RunnerIngressContext, RunnerRegistrationBuilder,
};
use mfm_store::v1 as store;

mod balance_collection;
mod transaction;

pub use balance_collection::verify_evm_balance_collection_replay;
pub use transaction::{
    is_evm_transaction_replay_intent, register_evm_transaction_runner,
    verify_evm_transaction_replay, EvmMutationValidationFuture, EvmTransactionRunnerCapabilities,
};

pub(crate) const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";
const READ_FACTORY: &str = "read_external";

/// Shared process resources required by reusable EVM read states.
#[derive(Clone)]
pub struct EvmReadRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    sessions: Arc<dyn EvmReadSessionSet>,
}

impl EvmReadRunnerCapabilities {
    /// Creates one source-bound capability assembly for all EVM reads.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        sessions: Arc<dyn EvmReadSessionSet>,
    ) -> Self {
        Self {
            artifacts,
            sessions,
        }
    }

    pub(crate) async fn validate_read_route(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<()> {
        self.sessions.validate_binding(&binding).await
    }

    pub(crate) async fn bind(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<Arc<dyn EvmReadSession>> {
        let session = self.sessions.session(&binding).await?;
        if !session.evidence().matches_binding(&binding)
            || session.evidence().implementation_id() != self.sessions.implementation_id()
        {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::SessionAuthorityMismatch,
            });
        }
        Ok(session)
    }
}

/// Registers the reusable fact-producing EVM balance collection binding.
pub fn register_evm_balance_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmReadRunnerCapabilities,
    read_factory: &RunnerFactoryBinding,
    adapter_factory: &RunnerFactoryBinding,
) -> mfm_runtime::Result<()> {
    register_evm_read_foundation(registry, capabilities.sessions.as_ref(), adapter_factory)?;
    require_factory(read_factory, READ_FACTORY)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_state_runner_with_factory::<CollectEvmBalancesState>(
        read_factory,
        Arc::new(ExternalReadRunner::<CollectEvmBalancesState, _>::new(
            Arc::clone(&capabilities.artifacts),
            balance_collection::CollectEvmBalancesExecutor {
                capabilities: capabilities.clone(),
            },
        )),
    )?;
    Ok(())
}

/// Registers the reusable exact-anchor EVM contract-validation binding.
pub fn register_evm_validation_runner(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmReadRunnerCapabilities,
    read_factory: &RunnerFactoryBinding,
    adapter_factory: &RunnerFactoryBinding,
) -> mfm_runtime::Result<()> {
    register_evm_read_foundation(registry, capabilities.sessions.as_ref(), adapter_factory)?;
    require_factory(read_factory, READ_FACTORY)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_state_runner_with_factory::<ValidateEvmContractState>(
        read_factory,
        Arc::new(ExternalReadRunner::<ValidateEvmContractState, _>::new(
            Arc::clone(&capabilities.artifacts),
            ValidateContractExecutor { capabilities },
        )),
    )?;
    Ok(())
}

fn register_evm_read_foundation(
    registry: &mut ErasedRunnerRegistry,
    sessions: &dyn EvmReadSessionSet,
    adapter_factory: &RunnerFactoryBinding,
) -> mfm_runtime::Result<()> {
    registry.register_capability_spec::<EvmReadCapability>(CapabilityImplementationId::new(
        sessions.implementation_id(),
    )?)?;
    require_factory(adapter_factory, ADAPTER_FACTORY)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_adapter_executable_with_factory(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        adapter_factory,
    )?;
    Ok(())
}

fn require_factory(
    factory: &RunnerFactoryBinding,
    expected: &'static str,
) -> mfm_runtime::Result<()> {
    if factory.factory_id().as_str() != expected {
        return Err(mfm_runtime::RuntimeError::RunnerBinding(format!(
            "EVM registration requires factory id {expected}"
        )));
    }
    Ok(())
}

struct ValidateContractExecutor {
    capabilities: EvmReadRunnerCapabilities,
}

impl ValidateContractExecutor {
    async fn validate_read_route(
        &self,
        state: &ValidateEvmContractState,
    ) -> mfm_runtime::Result<()> {
        let binding = state
            .config()
            .network_binding()
            .map_err(evm_state_runtime_error)?;
        self.capabilities
            .validate_read_route(binding)
            .await
            .map_err(evm_ingress_runtime_error)
    }
}

impl ExternalReadPlanExecutor<ValidateEvmContractState> for ValidateContractExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a ValidateEvmContractState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move { self.validate_read_route(state).await })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmContractValidationPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmContractValidationEvidence> {
        Box::pin(async move {
            let binding = EvmNetworkBinding::new(
                mfm_ids::LocalPublicId::new(plan.network_id())
                    .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?,
                plan.chain_id(),
            )
            .map_err(evm_read_runtime_error)?;
            let session = self
                .capabilities
                .bind(binding)
                .await
                .map_err(evm_read_runtime_error)?;
            collect_contract_validation_evidence(plan, session)
                .await
                .map(ExternalReadExecution::primary)
        })
    }
}

async fn collect_contract_validation_evidence(
    plan: &EvmContractValidationPlan,
    session: Arc<dyn EvmReadSession>,
) -> mfm_runtime::Result<EvmContractValidationEvidence> {
    let (code_address, code_selector) = plan.code_request().map_err(evm_state_runtime_error)?;
    let code = session
        .read_code(code_address, &code_selector)
        .await
        .map_err(evm_read_runtime_error)?;
    let mut evidence = EvmContractValidationEvidenceBuilder::new(plan, code, session.evidence())
        .map_err(evm_state_runtime_error)?;
    while let Some(request) = evidence
        .next_call_request()
        .map_err(evm_state_runtime_error)?
    {
        let response = session
            .call(&request)
            .await
            .map_err(evm_read_runtime_error)?;
        evidence
            .push_call_response(response)
            .map_err(evm_state_runtime_error)?;
    }
    let canonicality_selector = plan
        .canonicality_selector()
        .map_err(evm_state_runtime_error)?;
    let canonical_block = session
        .read_block(&canonicality_selector)
        .await
        .map_err(evm_read_runtime_error)?;
    evidence
        .finish(canonical_block)
        .map_err(evm_state_runtime_error)
}

/// Verifies exact-anchor validation from retained evidence only.
pub fn verify_evm_validation_replay(
    broker: &mfm_replay::v1::ReplayBroker,
) -> mfm_replay::v1::Result<()> {
    mfm_replay::v1::verify_external_read_state::<ValidateEvmContractState>(broker)
}

fn evm_read_runtime_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    evm_capability_runtime_error(error, EvmCapabilityPhase::ReadOnly)
}

/// Preserves closed runtime-configuration failures during pre-admission EVM validation.
///
/// Live execution uses phase-aware operational blocking instead; this conversion is only for
/// runner ingress, before `RunAdmitted` can be persisted.
pub fn evm_ingress_runtime_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    let Some(diagnostic) = error.redacted_diagnostic() else {
        return evm_read_runtime_error(error);
    };
    let (code, safe_message) = match diagnostic.code() {
        ProviderDiagnosticCode::ProviderConfigurationMissing => {
            ("RuntimeConfigRequired", "runtime configuration is required")
        }
        ProviderDiagnosticCode::ProviderConfigurationInvalid => {
            ("RuntimeConfigInvalid", "runtime configuration is invalid")
        }
        _ => return evm_read_runtime_error(error),
    };
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("runtime configuration error code is checked text"),
        events::ErrorCategory::Capability,
        safe_message,
        vec![diagnostic.clone()],
    )
    .expect("runtime configuration failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

fn evm_capability_runtime_error(
    error: EvmCapabilityError,
    phase: EvmCapabilityPhase,
) -> mfm_runtime::RuntimeError {
    match error.failure_disposition(phase) {
        EvmCapabilityFailureDisposition::OperationalBlock => mfm_runtime::RuntimeError::Blocked(
            "EVM runtime provider capability is unavailable".to_owned(),
        ),
        EvmCapabilityFailureDisposition::TerminalValidation => {
            let Some(diagnostic) = error.redacted_diagnostic().cloned() else {
                return mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string());
            };
            let failure = mfm_runtime::RuntimeFailure::new(
                events::ErrorCode::new("EvmProviderContractInvalid")
                    .expect("EVM runtime failure code is checked public text"),
                events::ErrorCategory::Validation,
                "EVM provider response violated the request contract",
                vec![diagnostic],
            )
            .expect("EVM runtime failure metadata is a checked public contract");
            mfm_runtime::RuntimeError::Failure(failure)
        }
    }
}

fn evm_state_runtime_error(error: mfm_evm::EvmStateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

pub(crate) fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
struct TestReadSessionSet {
    session: Arc<dyn EvmReadSession>,
    validation_error: Option<EvmCapabilityError>,
    validations: Arc<std::sync::atomic::AtomicUsize>,
    binds: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl EvmReadSessionSet for TestReadSessionSet {
    fn implementation_id(&self) -> &str {
        mfm_evm::EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    }

    fn validate_binding<'a>(
        &'a self,
        _binding: &'a EvmNetworkBinding,
    ) -> mfm_evm::EvmSessionFuture<'a, ()> {
        use std::sync::atomic::Ordering;

        self.validations.fetch_add(1, Ordering::SeqCst);
        let result = self.validation_error.clone().map_or(Ok(()), Err);
        Box::pin(std::future::ready(result))
    }

    fn session<'a>(
        &'a self,
        _binding: &'a EvmNetworkBinding,
    ) -> mfm_evm::EvmSessionFuture<'a, Arc<dyn EvmReadSession>> {
        use std::sync::atomic::Ordering;

        self.binds.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(Arc::clone(&self.session))))
    }
}

#[cfg(test)]
fn test_read_session_set(
    session: Arc<dyn EvmReadSession>,
    validation_error: Option<EvmCapabilityError>,
    validations: Arc<std::sync::atomic::AtomicUsize>,
    binds: Arc<std::sync::atomic::AtomicUsize>,
) -> Arc<dyn EvmReadSessionSet> {
    Arc::new(TestReadSessionSet {
        session,
        validation_error,
        validations,
        binds,
    })
}

#[cfg(test)]
mod balance_collection_tests;
#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;
