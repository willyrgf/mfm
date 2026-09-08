use super::*;
use mfm_evm::{
    AnchoredContractCallFailureReason, AnchoredObservationFacts, CallAt, CallCreatedAt, Called,
    CheckedCallPlan, CheckedCreatePlan, CheckedObservationPlan, CheckedTargetCallPlan,
    CompletedTransactionFacts, CreateAt, Created, Eip1559TransactionCommand, EvmTransaction,
    EvmTransactionFailure, ExecutedTransactionFacts, ObserveAt,
};
use mfm_program::StateExecutionError;
use mfm_program_derive::MfmContext;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FixtureRequest {
    pub label: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.contract-workflow")]
#[serde(deny_unknown_fields)]
pub struct ContractWorkflow<D, C, O> {
    pub request: FixtureRequest,
    pub deployment: D,
    pub configuration: C,
    pub observation: O,
}

pub type Initial = ContractWorkflow<CheckedCreatePlan, CheckedCallPlan, CheckedObservationPlan>;
pub type DeployRecipe = CreateAt<ContractWorkflowDeploymentSlot>;
pub type Deploy = EvmTransaction<Initial, DeployRecipe>;
pub type Deployment = <Deploy as Operation>::Output;
pub type DeployFailure = <Deploy as Operation>::Failure;
pub type ConfigureRecipe =
    CallCreatedAt<ContractWorkflowConfigurationSlot, ContractWorkflowDeploymentSlot>;
pub type Configure = EvmTransaction<Deployment, ConfigureRecipe>;
pub type Configuration = <Configure as Operation>::Output;
pub type ConfigureFailure = <Configure as Operation>::Failure;
pub type Observe = ReadAnchoredContractCall<
    Configuration,
    ObserveAt<ContractWorkflowObservationSlot, ContractWorkflowConfigurationSlot>,
>;
pub type Observed = <Observe as State>::Output;
pub type ObserveFailure = <Observe as State>::Failure;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.wallet-follow-up")]
#[serde(deny_unknown_fields)]
pub struct WalletContext<T> {
    pub transaction: T,
    pub label: u64,
}
pub type WalletInitial = WalletContext<CheckedTargetCallPlan>;
pub type WalletRecipe = CallAt<WalletContextTransactionSlot>;
pub type WalletTransaction = EvmTransaction<WalletInitial, WalletRecipe>;
pub type WalletReport = <WalletTransaction as Operation>::Output;
pub type WalletFailure = <WalletTransaction as Operation>::Failure;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FixtureReport {
    context: Observed,
    decoded: EvmU256,
}
impl FixtureReport {
    pub fn context(&self) -> &Observed {
        &self.context
    }
    pub fn decoded(&self) -> &EvmU256 {
        &self.decoded
    }
}
impl<'de> Deserialize<'de> for FixtureReport {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            context: Observed,
            decoded: EvmU256,
        }
        let wire = Wire::deserialize(deserializer)?;
        let result = wire
            .context
            .observation
            .result()
            .ok_or_else(|| serde::de::Error::custom("missing returned observation"))?;
        if decode_fixture_value(result.return_bytes()).as_ref() != Some(&wire.decoded) {
            return Err(serde::de::Error::custom(
                "report value disagrees with observation",
            ));
        }
        Ok(Self {
            context: wire.context,
            decoded: wire.decoded,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureFailureReason {
    PriorDeploymentReverted,
    DeploymentReverted,
    ConfigurationReverted,
    ObservationFailed(AnchoredContractCallFailureReason),
    InvalidReturnData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct FailurePlans<C> {
    pub creations: C,
    pub configuration: CheckedCallPlan,
    pub observation: CheckedObservationPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionEvidence {
    pub reservation: mfm_evm::Reservation,
    pub preparation: mfm_evm::PreparedEvmTransactionEvidence,
    pub settlement: mfm_evm::EvmTransactionSettlement,
}
impl TransactionEvidence {
    pub fn from_executed(facts: &ExecutedTransactionFacts) -> Self {
        Self {
            reservation: facts.reservation().clone(),
            preparation: facts.preparation().clone(),
            settlement: facts.settlement().clone(),
        }
    }
    pub fn executed(
        &self,
        command: Eip1559TransactionCommand,
    ) -> Result<ExecutedTransactionFacts, StateExecutionError> {
        let reserved = mfm_evm::ReservedEvmTransaction::new(command, self.reservation.clone())
            .map_err(|_| StateExecutionError)?;
        ExecutedTransactionFacts::new(
            mfm_evm::PreparedTransactionFacts::new(reserved, self.preparation.clone()),
            self.settlement.clone(),
        )
        .map_err(|_| StateExecutionError)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateProgress<N> {
    pub evidence: TransactionEvidence,
    pub next: Option<N>,
}
impl<N> CreateProgress<N> {
    pub fn validate(
        &self,
        plan: &CheckedCreatePlan,
    ) -> Result<Option<EvmAddress>, StateExecutionError> {
        let facts = self.evidence.executed(plan.command())?;
        match (facts.settlement().outcome(), self.next.is_some()) {
            (mfm_evm::EvmTransactionOutcome::Reverted, false) => Ok(None),
            (mfm_evm::EvmTransactionOutcome::Created { created_address }, true) => {
                Ok(Some(created_address.clone()))
            }
            _ => Err(StateExecutionError),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct CallProgress {
    pub target: EvmAddress,
    pub evidence: TransactionEvidence,
    pub observation: Option<AnchoredObservationFacts>,
}
impl CallProgress {
    pub fn validate<C>(
        &self,
        plans: &FailurePlans<C>,
        target: &EvmAddress,
    ) -> Result<(), StateExecutionError> {
        if &self.target != target {
            return Err(StateExecutionError);
        }
        let facts = self
            .evidence
            .executed(plans.configuration.command_for(self.target.clone()))?;
        match (facts.settlement().outcome(), &self.observation) {
            (mfm_evm::EvmTransactionOutcome::Reverted, None) => Ok(()),
            (mfm_evm::EvmTransactionOutcome::Called, Some(observation)) => {
                let expected = plans.observation.intent_for(
                    self.target.clone(),
                    facts.settlement().block_anchor().clone(),
                );
                if observation.intent() != &expected
                    || observation.intent().route_ref()
                        != &facts
                            .command()
                            .binding()
                            .route
                            .binding_ref()
                            .map_err(|_| StateExecutionError)?
                    || observation.intent().chain_id()
                        != facts.command().binding().route.chain_instance.chain_id
                    || observation
                        .result()
                        .is_some_and(|value| decode_fixture_value(value.return_bytes()).is_some())
                {
                    return Err(StateExecutionError);
                }
                Ok(())
            }
            _ => Err(StateExecutionError),
        }
    }
    pub fn reason(&self) -> FixtureFailureReason {
        match &self.observation {
            None => FixtureFailureReason::ConfigurationReverted,
            Some(observation) => observation.failure_reason().map_or(
                FixtureFailureReason::InvalidReturnData,
                FixtureFailureReason::ObservationFailed,
            ),
        }
    }
}

pub(super) fn create_plan(
    command: &Eip1559TransactionCommand,
) -> Result<CheckedCreatePlan, StateExecutionError> {
    if command.to().is_some() {
        return Err(StateExecutionError);
    }
    CheckedCreatePlan::new(
        command.binding().clone(),
        command.input().to_vec(),
        command.value().clone(),
        command.gas_limit(),
        command.max_priority_fee_per_gas(),
        command.max_fee_per_gas(),
    )
    .map_err(|_| StateExecutionError)
}
pub(super) fn call_plan(
    command: &Eip1559TransactionCommand,
) -> Result<CheckedCallPlan, StateExecutionError> {
    if command.to().is_none() {
        return Err(StateExecutionError);
    }
    CheckedCallPlan::new(
        command.binding().clone(),
        command.input().to_vec(),
        command.value().clone(),
        command.gas_limit(),
        command.max_priority_fee_per_gas(),
        command.max_fee_per_gas(),
    )
    .map_err(|_| StateExecutionError)
}
pub(super) fn observation_plan(
    facts: &AnchoredObservationFacts,
    binding: &EvmTransactionBinding,
) -> Result<CheckedObservationPlan, StateExecutionError> {
    CheckedObservationPlan::new(binding.route.clone(), facts.intent().calldata().to_vec())
        .map_err(|_| StateExecutionError)
}

#[derive(Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FixtureFailure {
    request: FixtureRequest,
    plans: FailurePlans<CheckedCreatePlan>,
    progress: CreateProgress<CallProgress>,
}
impl std::fmt::Debug for FixtureFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixtureFailure")
            .field("reason", &self.reason())
            .finish_non_exhaustive()
    }
}
impl FixtureFailure {
    pub(super) fn new(
        request: FixtureRequest,
        plans: FailurePlans<CheckedCreatePlan>,
        progress: CreateProgress<CallProgress>,
    ) -> Result<Self, StateExecutionError> {
        let target = progress.validate(&plans.creations)?;
        if let (Some(call), Some(target)) = (&progress.next, target) {
            call.validate(&plans, &target)?;
        }
        Ok(Self {
            request,
            plans,
            progress,
        })
    }
    pub fn request(&self) -> &FixtureRequest {
        &self.request
    }
    pub(super) fn plans(&self) -> &FailurePlans<CheckedCreatePlan> {
        &self.plans
    }
    pub(super) fn progress(&self) -> &CreateProgress<CallProgress> {
        &self.progress
    }
    pub fn reason(&self) -> FixtureFailureReason {
        self.progress.next.as_ref().map_or(
            FixtureFailureReason::DeploymentReverted,
            CallProgress::reason,
        )
    }
    fn from_observed(context: Observed) -> Result<Self, StateExecutionError> {
        let plans = FailurePlans {
            creations: create_plan(context.deployment.command())?,
            configuration: call_plan(context.configuration.command())?,
            observation: observation_plan(
                &context.observation,
                context.configuration.command().binding(),
            )?,
        };
        let progress = CreateProgress {
            evidence: TransactionEvidence::from_executed(context.deployment.executed()),
            next: Some(CallProgress {
                target: context.configuration.outcome().target().clone(),
                evidence: TransactionEvidence::from_executed(context.configuration.executed()),
                observation: Some(context.observation),
            }),
        };
        Self::new(context.request, plans, progress)
    }
}
impl<'de> Deserialize<'de> for FixtureFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: FixtureRequest,
            plans: FailurePlans<CheckedCreatePlan>,
            progress: CreateProgress<CallProgress>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.request, wire.plans, wire.progress).map_err(serde::de::Error::custom)
    }
}
impl
    TryFrom<
        EvmTransactionFailure<
            ContractWorkflow<ExecutedTransactionFacts, CheckedCallPlan, CheckedObservationPlan>,
        >,
    > for FixtureFailure
{
    type Error = StateExecutionError;
    fn try_from(failure: DeployFailure) -> Result<Self, Self::Error> {
        let context = failure.into_context();
        let plans = FailurePlans {
            creations: create_plan(context.deployment.command())?,
            configuration: context.configuration,
            observation: context.observation,
        };
        Self::new(
            context.request,
            plans,
            CreateProgress {
                evidence: TransactionEvidence::from_executed(&context.deployment),
                next: None,
            },
        )
    }
}
impl
    TryFrom<
        EvmTransactionFailure<
            ContractWorkflow<
                CompletedTransactionFacts<Created>,
                ExecutedTransactionFacts,
                CheckedObservationPlan,
            >,
        >,
    > for FixtureFailure
{
    type Error = StateExecutionError;
    fn try_from(failure: ConfigureFailure) -> Result<Self, Self::Error> {
        let context = failure.into_context();
        let plans = FailurePlans {
            creations: create_plan(context.deployment.command())?,
            configuration: call_plan(context.configuration.command())?,
            observation: context.observation,
        };
        let progress = CreateProgress {
            evidence: TransactionEvidence::from_executed(context.deployment.executed()),
            next: Some(CallProgress {
                target: context
                    .configuration
                    .command()
                    .to()
                    .ok_or(StateExecutionError)?
                    .clone(),
                evidence: TransactionEvidence::from_executed(&context.configuration),
                observation: None,
            }),
        };
        Self::new(context.request, plans, progress)
    }
}
impl
    TryFrom<
        mfm_evm::AnchoredContractCallFailure<
            ContractWorkflow<
                CompletedTransactionFacts<Created>,
                CompletedTransactionFacts<Called>,
                AnchoredObservationFacts,
            >,
        >,
    > for FixtureFailure
{
    type Error = StateExecutionError;
    fn try_from(failure: ObserveFailure) -> Result<Self, Self::Error> {
        Self::from_observed(failure.into_context())
    }
}

pub struct DecodeValue;
impl State for DecodeValue {
    type Input = Observed;
    type Output = FixtureReport;
    type Failure = FixtureFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/decode-value@2")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for DecodeValue {
    fn evaluate(
        input: Observed,
    ) -> Result<ProposedStateOutcome<FixtureReport, FixtureFailure>, StateExecutionError> {
        let result = input.observation.result().ok_or(StateExecutionError)?;
        match decode_fixture_value(result.return_bytes()) {
            Some(decoded) => Ok(ProposedStateOutcome::Success {
                output: FixtureReport {
                    context: input,
                    decoded,
                },
            }),
            None => Ok(ProposedStateOutcome::Failure {
                failure: FixtureFailure::from_observed(input)?,
            }),
        }
    }
}

pub struct Abort<I, O, F = FixtureFailure>(PhantomData<fn(I) -> (O, F)>);
impl<I: MfmValue, O: MfmValue, F: MfmValue> State for Abort<I, O, F>
where
    F: TryFrom<I>,
{
    type Input = I;
    type Output = O;
    type Failure = F;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/abort@2").map_err(|_| ProgramError::InvalidContract)
    }
}
impl<I: MfmValue, O: MfmValue, F: MfmValue> PureState for Abort<I, O, F>
where
    F: TryFrom<I>,
{
    fn evaluate(input: I) -> Result<ProposedStateOutcome<O, F>, StateExecutionError> {
        Ok(ProposedStateOutcome::Failure {
            failure: F::try_from(input).map_err(|_| StateExecutionError)?,
        })
    }
}

pub struct EffectFixtureOperation {
    pub binding: EvmTransactionBinding,
}
impl Operation for EffectFixtureOperation {
    type Input = Initial;
    type Output = FixtureReport;
    type Failure = FixtureFailure;
    fn expand(
        &self,
        body: &mut OperationExpansion<Initial, FixtureReport, FixtureFailure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<DeployFailure, Deployment>(
            |protected| protected.operation(&Deploy::new(self.binding.clone())),
            |handler| handler.pure::<Abort<DeployFailure, Deployment>>(),
        )?;
        body.with_failure_handler::<ConfigureFailure, Configuration>(
            |protected| protected.operation(&Configure::new(self.binding.clone())),
            |handler| handler.pure::<Abort<ConfigureFailure, Configuration>>(),
        )?;
        body.with_failure_handler::<ObserveFailure, Observed>(
            |protected| protected.read::<Observe, EvmAnchoredContractCallRead>(&self.binding.route),
            |handler| handler.pure::<Abort<ObserveFailure, Observed>>(),
        )?;
        body.pure::<DecodeValue>()
    }
}

pub fn register_fixture_states(builder: &mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()> {
    register_evm_transaction_states::<Initial, DeployRecipe>(builder)?;
    register_evm_transaction_states::<Deployment, ConfigureRecipe>(builder)?;
    builder.register_read::<Observe, EvmAnchoredContractCallRead>()?;
    builder.register_pure::<DecodeValue>()?;
    builder.register_pure::<Abort<DeployFailure, Deployment>>()?;
    builder.register_pure::<Abort<ConfigureFailure, Configuration>>()?;
    builder.register_pure::<Abort<ObserveFailure, Observed>>()
}
