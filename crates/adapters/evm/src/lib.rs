#![warn(missing_docs)]
//! Runtime bindings for one EVM transaction and exact-anchor contract validation.
//!
//! Portfolio collection is intentionally absent; it is owned by
//! `mfm-adapters-portfolio`. This crate binds only the two reusable state kinds
//! retained by `mfm-states-evm`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmCapabilityError, EvmCapabilityFailureDisposition, EvmCapabilityPhase, EvmInvalidRequest,
    EvmNetworkBinding, EvmReadCapability, EvmReadSession, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_runtime::{
    CapabilityImplementationId, ErasedRunCtx, ErasedRunnerRegistry, ExternalReadExecution,
    ExternalReadExecutionFuture, ExternalReadPlanExecutor, ExternalReadRunner,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerRegistrationBuilder,
};
use mfm_states_evm::{
    evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version, EvmContractValidationEvidence,
    EvmContractValidationPlan, ValidateEvmContractState,
};
use mfm_store::v1 as store;

mod transaction;

pub use transaction::{
    is_evm_transaction_replay_intent, register_evm_transaction_runner,
    verify_evm_transaction_replay, EvmMutationValidationFuture, EvmTransactionRunnerCapabilities,
    EvmTransactionSessionBindFuture,
};

pub(crate) const ADAPTER_FACTORY: &str = "evm_jsonrpc_adapter";
const READ_FACTORY: &str = "read_external";

/// Future returned by the application-owned validation-session binder.
pub type EvmValidationSessionBindFuture = Pin<
    Box<
        dyn Future<Output = mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>>> + Send + 'static,
    >,
>;

type ValidateEvmBinding =
    dyn Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync;
type BindEvmReadSession = dyn Fn(EvmNetworkBinding) -> EvmValidationSessionBindFuture + Send + Sync;

/// Process resources required by exact-anchor contract validation.
#[derive(Clone)]
pub struct EvmValidationRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    validate_evm_binding: Arc<ValidateEvmBinding>,
    bind_evm_read_session: Arc<BindEvmReadSession>,
}

impl EvmValidationRunnerCapabilities {
    /// Creates lazy, exact-bound validation capability assembly.
    pub fn new<V, B>(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        validate_evm_binding: V,
        bind_evm_read_session: B,
    ) -> Self
    where
        V: Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync + 'static,
        B: Fn(EvmNetworkBinding) -> EvmValidationSessionBindFuture + Send + Sync + 'static,
    {
        Self {
            artifacts,
            validate_evm_binding: Arc::new(validate_evm_binding),
            bind_evm_read_session: Arc::new(bind_evm_read_session),
        }
    }

    fn validate(&self, binding: &EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> {
        (self.validate_evm_binding)(binding)
    }

    async fn bind(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>> {
        let session = (self.bind_evm_read_session)(binding.clone()).await?;
        if !session.evidence().matches_binding(&binding)
            || session.evidence().implementation_id() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::SessionAuthorityMismatch,
            });
        }
        Ok(session)
    }
}

/// Registers the one exact-anchor validation read binding.
pub fn register_evm_validation_runner(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmValidationRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    registry.register_capability_spec::<EvmReadCapability>(CapabilityImplementationId::new(
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
    )?)?;
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm",
        "typed-evm-jsonrpc",
        env!("CARGO_PKG_VERSION"),
    )?;
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_adapter_executable_with_factory(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<ValidateEvmContractState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<ValidateEvmContractState, _>::new(
            Arc::clone(&capabilities.artifacts),
            ValidateContractExecutor { capabilities },
        )),
    )?;
    Ok(())
}

struct ValidateContractExecutor {
    capabilities: EvmValidationRunnerCapabilities,
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
            .validate(&binding)
            .map_err(evm_read_runtime_error)
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
            let (code_address, code_selector) =
                plan.code_request().map_err(evm_state_runtime_error)?;
            let code = session
                .read_code(code_address, &code_selector)
                .await
                .map_err(evm_read_runtime_error)?;
            let mut calls = Vec::with_capacity(plan.calls().len());
            for request in plan.call_requests().map_err(evm_state_runtime_error)? {
                let response = session
                    .call(&request)
                    .await
                    .map_err(evm_read_runtime_error)?;
                calls.push((request, response));
            }
            let canonicality_selector = plan
                .canonicality_selector()
                .map_err(evm_state_runtime_error)?;
            let canonical_block = session
                .read_block(&canonicality_selector)
                .await
                .map_err(evm_read_runtime_error)?;
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

/// Verifies exact-anchor validation from retained evidence only.
pub fn verify_evm_validation_replay(
    broker: &mfm_replay::v1::ReplayBroker,
) -> mfm_replay::v1::Result<()> {
    mfm_replay::v1::verify_external_read_state::<ValidateEvmContractState>(broker)
}

fn evm_read_runtime_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    evm_capability_runtime_error(error, EvmCapabilityPhase::ReadOnly)
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

fn evm_state_runtime_error(error: mfm_states_evm::EvmStateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

pub(crate) fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;
