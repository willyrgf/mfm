use super::{
    artifact, stages, EvmAddress, EvmChainInstance, EvmContractExecutionConfig,
    EvmScalarContractArtifact, EvmTransactionBinding, EvmTransactionImplementation,
    EvmTransactionOutcome, EvmU256,
};
use super::{Eip1559TransactionCommand, EvmTransactionSettlement};
use mfm_capabilities::{codec, CallbackFailure, EffectImplementation};
use mfm_chain::transaction::TransactionRequest;
use mfm_chain::transaction::{
    ConfigurationApplied, DeployedContract, DeploymentRequest, LifecyclePlanning, TransactionEffect,
};
use mfm_chain::ContractLocator;
use mfm_values::InvocationDiagnostic;
use mfm_values::Object;

/// Native deterministic recipe for one supported semantic transaction request.
/// Callers select maintained shared States; this trait belongs to EVM implementation support.
pub trait EvmTransactionRecipe: TransactionRequest {
    /// Constructs and qualifies the complete nonce-free native command without IO.
    fn command(&self) -> Result<Eip1559TransactionCommand, CallbackFailure>;
    /// Projects request-specific facts from a checked successful native settlement.
    fn applied(
        &self,
        settlement: &EvmTransactionSettlement,
    ) -> Result<Self::Applied, CallbackFailure>;
}

pub(super) fn execution_config(
    request: &DeploymentRequest,
) -> Result<EvmContractExecutionConfig, CallbackFailure> {
    let selected = <EvmTransactionImplementation as EffectImplementation<
        TransactionEffect<DeploymentRequest>,
    >>::implementation_id()
    .map_err(|error| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "transaction_selection_identity",
            &error,
            None,
        )
    })?;
    if request.execution().transaction_implementation() != &selected {
        return Err(stages::invariant("transaction_selection").into());
    }
    codec::decode(|| {
        request
            .artifact()
            .native()
            .decode::<EvmScalarContractArtifact>()
    })?;
    let ledger = codec::decode(|| {
        request
            .artifact()
            .ledger()
            .native()
            .decode::<EvmChainInstance>()
    })?;
    let config = codec::decode(|| {
        request
            .execution()
            .native()
            .decode::<EvmContractExecutionConfig>()
    })?;
    if ledger != config.binding().route.chain_instance {
        return Err(stages::invariant("transaction_ledger").into());
    }
    super::read::qualify_route(
        &config.binding().route,
        request.execution().observation_route_ref(),
    )?;
    Ok(config)
}

impl EvmTransactionRecipe for DeploymentRequest {
    fn command(&self) -> Result<Eip1559TransactionCommand, CallbackFailure> {
        let config = execution_config(self)?;
        let artifact = codec::decode(|| {
            self.artifact()
                .native()
                .decode::<EvmScalarContractArtifact>()
        })?;
        let options = config.deployment();
        Ok(Eip1559TransactionCommand::create(
            config.binding().clone(),
            artifact.initcode().to_vec(),
            EvmU256::from_u64(0),
            options.gas_limit(),
            options.max_priority_fee_per_gas(),
            options.max_fee_per_gas(),
        )
        .map_err(|error| {
            InvocationDiagnostic::from_fields("state_internal", "deployment_command", &error, None)
        })?)
    }
    fn applied(
        &self,
        settlement: &EvmTransactionSettlement,
    ) -> Result<ContractLocator, CallbackFailure> {
        execution_config(self)?;
        let EvmTransactionOutcome::Created { created_address } = settlement.outcome() else {
            return Err(stages::invariant("deployment_outcome").into());
        };
        Ok(ContractLocator::new(
            self.artifact().ledger().clone(),
            codec::encode(|| {
                Object::from_value(created_address)
                    .map_err(|error| error.into_diagnostic("deployment_locator"))
            })?,
        ))
    }
}

impl EvmTransactionRecipe for DeployedContract {
    fn command(&self) -> Result<Eip1559TransactionCommand, CallbackFailure> {
        let config = execution_config(self.request())?;
        let contract = self.contract()?;
        if contract.ledger() != self.request().artifact().ledger() {
            return Err(stages::invariant("configuration_ledger").into());
        }
        let target = codec::decode(|| contract.native().decode::<EvmAddress>())?;
        let options = config.configuration();
        Ok(Eip1559TransactionCommand::call(
            config.binding().clone(),
            target,
            artifact::configure_calldata(self.effective()),
            EvmU256::from_u64(0),
            options.gas_limit(),
            options.max_priority_fee_per_gas(),
            options.max_fee_per_gas(),
        )
        .map_err(|error| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "configuration_command",
                &error,
                None,
            )
        })?)
    }
    fn applied(
        &self,
        settlement: &EvmTransactionSettlement,
    ) -> Result<ConfigurationApplied, CallbackFailure> {
        if !matches!(settlement.outcome(), EvmTransactionOutcome::Called) {
            return Err(stages::invariant("configuration_outcome").into());
        }
        Ok(ConfigurationApplied)
    }
}

impl<Config: LifecyclePlanning + ?Sized, R: EvmTransactionRecipe>
    mfm_program::ResolveEffectBinding<Config, TransactionEffect<R>>
    for EvmTransactionImplementation
{
    fn binding(config: &Config) -> mfm_program::Result<EvmTransactionBinding> {
        Ok(execution_config(config.deployment_request())
            .map_err(|cause| mfm_program::ProgramError::Diagnostic(cause.into_diagnostic()))?
            .binding()
            .clone())
    }
}
