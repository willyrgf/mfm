#![warn(missing_docs)]
//! Runtime binding for the aggregate Bitcoin balance read.

use std::sync::Arc;

use mfm_bitcoin::{
    bitcoin_jsonrpc_adapter_kind, bitcoin_jsonrpc_adapter_version, BitcoinBalanceCollectionError,
    BitcoinBalanceCollectionEvidence, BitcoinBalanceCollectionPlan,
    BitcoinBalanceCollectionReadCapability, BitcoinBalanceSession, BitcoinCapabilityError,
    CollectBitcoinBalancesState,
};
use mfm_events::v1 as events;
use mfm_runtime::{
    CapabilityImplementationId, ErasedRunCtx, ErasedRunnerRegistry, ExternalReadExecution,
    ExternalReadExecutionFuture, ExternalReadPlanExecutor, ExternalReadRunner,
    RunnerFactoryBinding, RunnerIngressContext, RunnerRegistrationBuilder,
};
use mfm_store::v1 as store;

const READ_FACTORY: &str = "read_external";
const ADAPTER_FACTORY: &str = "bitcoin_jsonrpc_adapter";

/// Registers the one aggregate Bitcoin read runner against one supplied session object.
pub fn register_bitcoin_jsonrpc_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    session: Arc<dyn BitcoinBalanceSession>,
    read_factory: &RunnerFactoryBinding,
    adapter_factory: &RunnerFactoryBinding,
) -> mfm_runtime::Result<()> {
    registry.register_capability_spec::<BitcoinBalanceCollectionReadCapability>(
        CapabilityImplementationId::new(session.implementation_id())?,
    )?;

    require_factory(read_factory, READ_FACTORY)?;
    require_factory(adapter_factory, ADAPTER_FACTORY)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_adapter_executable_with_factory(
        bitcoin_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        bitcoin_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<CollectBitcoinBalancesState>(
        read_factory,
        Arc::new(ExternalReadRunner::<CollectBitcoinBalancesState, _>::new(
            artifacts,
            CollectBitcoinBalancesExecutor { session },
        )),
    )?;
    Ok(())
}

fn require_factory(
    factory: &RunnerFactoryBinding,
    expected: &'static str,
) -> mfm_runtime::Result<()> {
    if factory.factory_id().as_str() != expected {
        return Err(mfm_runtime::RuntimeError::RunnerBinding(format!(
            "Bitcoin registration requires factory id {expected}"
        )));
    }
    Ok(())
}

struct CollectBitcoinBalancesExecutor {
    session: Arc<dyn BitcoinBalanceSession>,
}

impl ExternalReadPlanExecutor<CollectBitcoinBalancesState> for CollectBitcoinBalancesExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a CollectBitcoinBalancesState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move {
            let request = state.config().request().map_err(bitcoin_state_error)?;
            self.session
                .validate_binding(request.binding())
                .await
                .map_err(bitcoin_ingress_error)
        })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a BitcoinBalanceCollectionPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, BitcoinBalanceCollectionEvidence> {
        Box::pin(async move {
            let request = plan.request().map_err(bitcoin_state_error)?;
            let response = self
                .session
                .collect_balances(&request)
                .await
                .map_err(bitcoin_capability_error)?;
            Ok(ExternalReadExecution::primary(
                BitcoinBalanceCollectionEvidence::from_response(&response),
            ))
        })
    }
}

/// Verifies aggregate Bitcoin replay from retained evidence only.
pub fn verify_bitcoin_jsonrpc_replay(
    broker: &mfm_replay::v1::ReplayBroker,
) -> mfm_replay::v1::Result<()> {
    mfm_replay::v1::verify_external_read_state::<CollectBitcoinBalancesState>(broker)
}

fn bitcoin_capability_error(error: BitcoinCapabilityError) -> mfm_runtime::RuntimeError {
    match error {
        BitcoinCapabilityError::Provider {
            retryable: true, ..
        } => mfm_runtime::RuntimeError::Blocked(
            "Bitcoin balance source is temporarily unavailable".to_owned(),
        ),
        BitcoinCapabilityError::Provider {
            diagnostic,
            retryable: false,
        } => {
            let failure = mfm_runtime::RuntimeFailure::new(
                events::ErrorCode::new("BitcoinProviderContractInvalid")
                    .expect("static Bitcoin error code is valid"),
                events::ErrorCategory::Validation,
                "Bitcoin provider response violated the aggregate read contract",
                vec![diagnostic],
            )
            .expect("static Bitcoin failure contract is valid");
            mfm_runtime::RuntimeError::Failure(failure)
        }
        BitcoinCapabilityError::InvalidRequest { .. } | BitcoinCapabilityError::SourceMismatch => {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "Bitcoin aggregate read binding was invalid".to_owned(),
            )
        }
    }
}

fn bitcoin_ingress_error(error: BitcoinCapabilityError) -> mfm_runtime::RuntimeError {
    let BitcoinCapabilityError::Provider { diagnostic, .. } = &error else {
        return bitcoin_capability_error(error);
    };
    let (code, message) = match diagnostic.code() {
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationMissing => {
            ("RuntimeConfigRequired", "runtime configuration is required")
        }
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid => {
            ("RuntimeConfigInvalid", "runtime configuration is invalid")
        }
        _ => return bitcoin_capability_error(error),
    };
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("runtime configuration error code is checked text"),
        events::ErrorCategory::Capability,
        message,
        vec![diagnostic.clone()],
    )
    .expect("runtime configuration failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

fn bitcoin_state_error(_error: BitcoinBalanceCollectionError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "certified Bitcoin collection material was invalid".to_owned(),
    )
}

fn adapter_identity_error(error: mfm_ids::IdentityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
mod tests;
