use super::*;
use mfm_evm::{
    AnchoredContractCallFailureReason, AnchoredObservationFacts, CallAt, CallCreatedAt, Called,
    CheckedCallPlan, CheckedCreatePlan, CheckedObservationPlan, CheckedTargetCallPlan,
    CompletedTransactionFacts, CreateAt, Created, EvmTransaction, EvmTransactionFailure,
    ExecutedTransactionFacts, ObserveAt, TransactionReportFacts, TransactionReportOutcome,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FixtureStep {
    PriorDeployment,
    Deployment,
    Configuration,
    Observation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FixtureEntryData {
    CreatePlan(CheckedCreatePlan),
    CallPlan(CheckedCallPlan),
    ObservationPlan(CheckedObservationPlan),
    Transaction(Box<TransactionReportFacts>),
    Observation(Box<AnchoredObservationFacts>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FixtureEntry {
    pub step: FixtureStep,
    pub data: FixtureEntryData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FixtureFailureReason {
    PriorDeploymentReverted,
    DeploymentReverted,
    ConfigurationReverted,
    ObservationFailed(AnchoredContractCallFailureReason),
    InvalidReturnData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FixtureFailure {
    request: FixtureRequest,
    entries: Vec<FixtureEntry>,
    reason: FixtureFailureReason,
}
impl FixtureFailure {
    pub fn new(
        request: FixtureRequest,
        entries: Vec<FixtureEntry>,
        reason: FixtureFailureReason,
    ) -> Result<Self, StateExecutionError> {
        use FixtureEntryData as E;
        use TransactionReportOutcome as T;
        let (prior, tail) = match entries.as_slice() {
            [prior, rest @ ..]
                if entries.len() == 4 && prior.step == FixtureStep::PriorDeployment =>
            {
                (Some(prior), rest)
            }
            tail if entries.len() == 3 => (None, tail),
            _ => return Err(StateExecutionError),
        };
        let [deployment, configuration, observation] = tail else {
            return Err(StateExecutionError);
        };
        if [deployment.step, configuration.step, observation.step]
            != [
                FixtureStep::Deployment,
                FixtureStep::Configuration,
                FixtureStep::Observation,
            ]
        {
            return Err(StateExecutionError);
        }
        if let Some(prior) = prior {
            let E::Transaction(facts) = &prior.data else {
                return Err(StateExecutionError);
            };
            if facts.executed().command().to().is_some() {
                return Err(StateExecutionError);
            }
            if reason == FixtureFailureReason::PriorDeploymentReverted {
                if matches!(facts.outcome(), T::Reverted)
                    && matches!(deployment.data, E::CreatePlan(_))
                    && matches!(configuration.data, E::CallPlan(_))
                    && matches!(observation.data, E::ObservationPlan(_))
                {
                    return Ok(Self {
                        request,
                        entries,
                        reason,
                    });
                }
                return Err(StateExecutionError);
            }
            if !matches!(facts.outcome(), T::Created(_)) {
                return Err(StateExecutionError);
            }
        }
        if let E::Transaction(deployment) = &deployment.data {
            if deployment.executed().command().to().is_some() {
                return Err(StateExecutionError);
            }
            if let E::Transaction(configuration) = &configuration.data {
                let T::Created(created) = deployment.outcome() else {
                    return Err(StateExecutionError);
                };
                if configuration.executed().command().to() != Some(created.created_address()) {
                    return Err(StateExecutionError);
                }
                if let E::Observation(observation) = &observation.data {
                    let command = configuration.executed().command();
                    let intent = observation.intent();
                    if Some(intent.target()) != command.to()
                        || intent.anchor() != configuration.executed().settlement().block_anchor()
                        || intent.route_ref()
                            != &command
                                .binding()
                                .route
                                .binding_ref()
                                .map_err(|_| StateExecutionError)?
                        || intent.chain_id() != command.binding().route.chain_instance.chain_id
                    {
                        return Err(StateExecutionError);
                    }
                }
            }
        }
        let valid = match (
            &deployment.data,
            &configuration.data,
            &observation.data,
            reason,
        ) {
            (
                E::Transaction(d),
                E::CallPlan(_),
                E::ObservationPlan(_),
                FixtureFailureReason::DeploymentReverted,
            ) => matches!(d.outcome(), T::Reverted),
            (
                E::Transaction(d),
                E::Transaction(c),
                E::ObservationPlan(_),
                FixtureFailureReason::ConfigurationReverted,
            ) => matches!(d.outcome(), T::Created(_)) && matches!(c.outcome(), T::Reverted),
            (
                E::Transaction(d),
                E::Transaction(c),
                E::Observation(o),
                FixtureFailureReason::ObservationFailed(reason),
            ) => {
                matches!(d.outcome(), T::Created(_))
                    && matches!(c.outcome(), T::Called(_))
                    && o.failure_reason() == Some(reason)
            }
            (
                E::Transaction(d),
                E::Transaction(c),
                E::Observation(o),
                FixtureFailureReason::InvalidReturnData,
            ) => {
                matches!(d.outcome(), T::Created(_))
                    && matches!(c.outcome(), T::Called(_))
                    && o.result()
                        .is_some_and(|r| decode_fixture_value(r.return_bytes()).is_none())
            }
            _ => false,
        };
        if !valid {
            return Err(StateExecutionError);
        }
        Ok(Self {
            request,
            entries,
            reason,
        })
    }
    pub fn request(&self) -> &FixtureRequest {
        &self.request
    }
    pub fn reason(&self) -> FixtureFailureReason {
        self.reason
    }
    pub fn entries(&self) -> &[FixtureEntry] {
        &self.entries
    }
}
impl<'de> Deserialize<'de> for FixtureFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: FixtureRequest,
            entries: Vec<FixtureEntry>,
            reason: FixtureFailureReason,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.request, wire.entries, wire.reason).map_err(serde::de::Error::custom)
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
        Self::new(
            context.request,
            vec![
                FixtureEntry {
                    step: FixtureStep::Deployment,
                    data: FixtureEntryData::Transaction(Box::new(
                        TransactionReportFacts::from_executed(context.deployment)?,
                    )),
                },
                FixtureEntry {
                    step: FixtureStep::Configuration,
                    data: FixtureEntryData::CallPlan(context.configuration),
                },
                FixtureEntry {
                    step: FixtureStep::Observation,
                    data: FixtureEntryData::ObservationPlan(context.observation),
                },
            ],
            FixtureFailureReason::DeploymentReverted,
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
        Self::new(
            context.request,
            vec![
                FixtureEntry {
                    step: FixtureStep::Deployment,
                    data: FixtureEntryData::Transaction(Box::new(context.deployment.into())),
                },
                FixtureEntry {
                    step: FixtureStep::Configuration,
                    data: FixtureEntryData::Transaction(Box::new(
                        TransactionReportFacts::from_executed(context.configuration)?,
                    )),
                },
                FixtureEntry {
                    step: FixtureStep::Observation,
                    data: FixtureEntryData::ObservationPlan(context.observation),
                },
            ],
            FixtureFailureReason::ConfigurationReverted,
        )
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
        let reason = failure.reason();
        Self::from_observed(
            failure.into_context(),
            FixtureFailureReason::ObservationFailed(reason),
        )
    }
}
impl FixtureFailure {
    fn from_observed(
        context: Observed,
        reason: FixtureFailureReason,
    ) -> Result<Self, StateExecutionError> {
        Self::new(
            context.request,
            vec![
                FixtureEntry {
                    step: FixtureStep::Deployment,
                    data: FixtureEntryData::Transaction(Box::new(context.deployment.into())),
                },
                FixtureEntry {
                    step: FixtureStep::Configuration,
                    data: FixtureEntryData::Transaction(Box::new(context.configuration.into())),
                },
                FixtureEntry {
                    step: FixtureStep::Observation,
                    data: FixtureEntryData::Observation(Box::new(context.observation)),
                },
            ],
            reason,
        )
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
                failure: FixtureFailure::from_observed(
                    input,
                    FixtureFailureReason::InvalidReturnData,
                )?,
            }),
        }
    }
}

pub struct Abort<I, O>(PhantomData<fn(I) -> O>);
impl<I: MfmValue, O: MfmValue> State for Abort<I, O>
where
    FixtureFailure: TryFrom<I>,
{
    type Input = I;
    type Output = O;
    type Failure = FixtureFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/abort@2").map_err(|_| ProgramError::InvalidContract)
    }
}
impl<I: MfmValue, O: MfmValue> PureState for Abort<I, O>
where
    FixtureFailure: TryFrom<I>,
{
    fn evaluate(input: I) -> Result<ProposedStateOutcome<O, FixtureFailure>, StateExecutionError> {
        Ok(ProposedStateOutcome::Failure {
            failure: FixtureFailure::try_from(input).map_err(|_| StateExecutionError)?,
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
