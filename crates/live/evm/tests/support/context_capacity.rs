use super::accumulating_contract::{Fault, RecordingStore, ScriptedEvidence};
use super::*;
use mfm_evm::{
    AnchoredContractCallFailure, AnchoredContractCallFailureReason, AnchoredObservationFacts,
    CallCreatedAt, Called, CompletedTransactionFacts, CreateAt, Created, EvmChainInstance,
    EvmTransaction, EvmTransactionFailure, ExecutedTransactionFacts, ObserveAt,
    TransactionReportFacts,
};
use mfm_program::StateExecutionError;
use mfm_program_derive::MfmContext;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.two-creations")]
#[serde(deny_unknown_fields)]
struct CapacityWorkflow<A, D, C, O> {
    request: FixtureRequest,
    prior: A,
    deployment: D,
    configuration: C,
    observation: O,
}
type CapacityInitial =
    CapacityWorkflow<CheckedCreatePlan, CheckedCreatePlan, CheckedCallPlan, CheckedObservationPlan>;
type FirstRecipe = CreateAt<CapacityWorkflowPriorSlot>;
type FirstTransaction = EvmTransaction<CapacityInitial, FirstRecipe>;
type AfterFirst = <FirstTransaction as Operation>::Output;
type FirstFailure = <FirstTransaction as Operation>::Failure;
type SecondRecipe = CreateAt<CapacityWorkflowDeploymentSlot>;
type SecondTransaction = EvmTransaction<AfterFirst, SecondRecipe>;
type AfterSecond = <SecondTransaction as Operation>::Output;
type SecondFailure = <SecondTransaction as Operation>::Failure;
type CallRecipe = CallCreatedAt<CapacityWorkflowConfigurationSlot, CapacityWorkflowDeploymentSlot>;
type CallTransaction = EvmTransaction<AfterSecond, CallRecipe>;
type AfterCall = <CallTransaction as Operation>::Output;
type CallFailure = <CallTransaction as Operation>::Failure;
type Observation = ReadAnchoredContractCall<
    AfterCall,
    ObserveAt<CapacityWorkflowObservationSlot, CapacityWorkflowConfigurationSlot>,
>;
type AfterObservation = <Observation as State>::Output;
type ObservationFailure = <Observation as State>::Failure;

trait ReportEntry {
    fn entry(self) -> Result<FixtureEntryData, StateExecutionError>;
}
macro_rules! entry {
    ($ty:ty, $variant:ident) => {
        impl ReportEntry for $ty {
            fn entry(self) -> Result<FixtureEntryData, StateExecutionError> {
                Ok(FixtureEntryData::$variant(self.into()))
            }
        }
    };
}
entry!(CheckedCreatePlan, CreatePlan);
entry!(CheckedCallPlan, CallPlan);
entry!(CheckedObservationPlan, ObservationPlan);
entry!(AnchoredObservationFacts, Observation);
impl ReportEntry for CompletedTransactionFacts<Created> {
    fn entry(self) -> Result<FixtureEntryData, StateExecutionError> {
        Ok(FixtureEntryData::Transaction(Box::new(self.into())))
    }
}
impl ReportEntry for CompletedTransactionFacts<Called> {
    fn entry(self) -> Result<FixtureEntryData, StateExecutionError> {
        Ok(FixtureEntryData::Transaction(Box::new(self.into())))
    }
}
impl ReportEntry for ExecutedTransactionFacts {
    fn entry(self) -> Result<FixtureEntryData, StateExecutionError> {
        Ok(FixtureEntryData::Transaction(Box::new(
            TransactionReportFacts::from_executed(self)?,
        )))
    }
}
fn failure<A: ReportEntry, D: ReportEntry, C: ReportEntry, O: ReportEntry>(
    context: CapacityWorkflow<A, D, C, O>,
    reason: FixtureFailureReason,
) -> Result<FixtureFailure, StateExecutionError> {
    FixtureFailure::new(
        context.request,
        vec![
            FixtureEntry {
                step: FixtureStep::PriorDeployment,
                data: context.prior.entry()?,
            },
            FixtureEntry {
                step: FixtureStep::Deployment,
                data: context.deployment.entry()?,
            },
            FixtureEntry {
                step: FixtureStep::Configuration,
                data: context.configuration.entry()?,
            },
            FixtureEntry {
                step: FixtureStep::Observation,
                data: context.observation.entry()?,
            },
        ],
        reason,
    )
}
macro_rules! revert_report {
    ($a:ty, $d:ty, $c:ty, $reason:ident) => {
        impl TryFrom<EvmTransactionFailure<CapacityWorkflow<$a, $d, $c, CheckedObservationPlan>>>
            for FixtureFailure
        {
            type Error = StateExecutionError;
            fn try_from(
                value: EvmTransactionFailure<CapacityWorkflow<$a, $d, $c, CheckedObservationPlan>>,
            ) -> Result<Self, Self::Error> {
                failure(value.into_context(), FixtureFailureReason::$reason)
            }
        }
    };
}
revert_report!(
    ExecutedTransactionFacts,
    CheckedCreatePlan,
    CheckedCallPlan,
    PriorDeploymentReverted
);
revert_report!(
    CompletedTransactionFacts<Created>,
    ExecutedTransactionFacts,
    CheckedCallPlan,
    DeploymentReverted
);
revert_report!(
    CompletedTransactionFacts<Created>,
    CompletedTransactionFacts<Created>,
    ExecutedTransactionFacts,
    ConfigurationReverted
);
impl
    TryFrom<
        AnchoredContractCallFailure<
            CapacityWorkflow<
                CompletedTransactionFacts<Created>,
                CompletedTransactionFacts<Created>,
                CompletedTransactionFacts<Called>,
                AnchoredObservationFacts,
            >,
        >,
    > for FixtureFailure
{
    type Error = StateExecutionError;
    fn try_from(value: ObservationFailure) -> Result<Self, Self::Error> {
        let reason = value.reason();
        failure(
            value.into_context(),
            FixtureFailureReason::ObservationFailed(reason),
        )
    }
}

#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct CapacityReport {
    context: AfterObservation,
    decoded: EvmU256,
}
impl<'de> Deserialize<'de> for CapacityReport {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            context: AfterObservation,
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
struct CapacityDecode;
impl State for CapacityDecode {
    type Input = AfterObservation;
    type Output = CapacityReport;
    type Failure = FixtureFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.two-creations/decode@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for CapacityDecode {
    fn evaluate(
        input: AfterObservation,
    ) -> Result<ProposedStateOutcome<CapacityReport, FixtureFailure>, StateExecutionError> {
        let result = input.observation.result().ok_or(StateExecutionError)?;
        Ok(match decode_fixture_value(result.return_bytes()) {
            Some(decoded) => ProposedStateOutcome::Success {
                output: CapacityReport {
                    context: input,
                    decoded,
                },
            },
            None => ProposedStateOutcome::Failure {
                failure: failure(input, FixtureFailureReason::InvalidReturnData)?,
            },
        })
    }
}
struct CapacityOperation {
    binding: EvmTransactionBinding,
}
impl Operation for CapacityOperation {
    type Input = CapacityInitial;
    type Output = CapacityReport;
    type Failure = FixtureFailure;
    fn expand(
        &self,
        body: &mut OperationExpansion<CapacityInitial, CapacityReport, FixtureFailure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<FirstFailure, AfterFirst>(
            |body| body.operation(&FirstTransaction::new(self.binding.clone())),
            |body| body.pure::<Abort<FirstFailure, AfterFirst>>(),
        )?;
        body.with_failure_handler::<SecondFailure, AfterSecond>(
            |body| body.operation(&SecondTransaction::new(self.binding.clone())),
            |body| body.pure::<Abort<SecondFailure, AfterSecond>>(),
        )?;
        body.with_failure_handler::<CallFailure, AfterCall>(
            |body| body.operation(&CallTransaction::new(self.binding.clone())),
            |body| body.pure::<Abort<CallFailure, AfterCall>>(),
        )?;
        body.with_failure_handler::<ObservationFailure, AfterObservation>(
            |body| body.read::<Observation, EvmAnchoredContractCallRead>(&self.binding.route),
            |body| body.pure::<Abort<ObservationFailure, AfterObservation>>(),
        )?;
        body.pure::<CapacityDecode>()
    }
}

#[tokio::test]
async fn two_creations_call_observation_and_reports_fit_the_unchanged_capacity_envelope() {
    let sizes = [
        (3, 3),
        (1024, 1024),
        (16384, 16384),
        (49152, 49152),
        (49152, 131072),
    ];
    let cases = sizes
        .into_iter()
        .map(|(create, call)| (create, call, Fault::None, abi_word(42).to_vec(), None))
        .chain([
            (
                49152,
                131072,
                Fault::Revert(0),
                vec![],
                Some(FixtureFailureReason::PriorDeploymentReverted),
            ),
            (
                49152,
                131072,
                Fault::Revert(1),
                vec![],
                Some(FixtureFailureReason::DeploymentReverted),
            ),
            (
                49152,
                131072,
                Fault::Revert(2),
                vec![],
                Some(FixtureFailureReason::ConfigurationReverted),
            ),
            (
                49152,
                131072,
                Fault::Observation(AnchoredContractCallFailureReason::Rejected),
                vec![],
                Some(FixtureFailureReason::ObservationFailed(
                    AnchoredContractCallFailureReason::Rejected,
                )),
            ),
            (
                49152,
                131072,
                Fault::Observation(AnchoredContractCallFailureReason::SafeFailure),
                vec![],
                Some(FixtureFailureReason::ObservationFailed(
                    AnchoredContractCallFailureReason::SafeFailure,
                )),
            ),
            (
                49152,
                131072,
                Fault::Observation(AnchoredContractCallFailureReason::IntegrityBlocked),
                vec![],
                Some(FixtureFailureReason::ObservationFailed(
                    AnchoredContractCallFailureReason::IntegrityBlocked,
                )),
            ),
            (
                49152,
                131072,
                Fault::None,
                vec![1; 131072],
                Some(FixtureFailureReason::InvalidReturnData),
            ),
        ]);
    for (label, size) in [
        (
            "initial",
            CapacityInitial::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "after first",
            AfterFirst::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "after second",
            AfterSecond::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "after call",
            AfterCall::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "after observation",
            AfterObservation::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "success",
            CapacityReport::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
        (
            "failure",
            FixtureFailure::schema_descriptor()
                .unwrap()
                .identity_canonical_json()
                .unwrap()
                .as_bytes()
                .len(),
        ),
    ] {
        assert!(size <= 65536, "{label} schema bytes: {size}");
        println!("context schema {label}: {size}");
    }
    for (case, (create_bytes, call_bytes, fault, returned, expected_reason)) in cases.enumerate() {
        let binding = EvmTransactionBinding {
            route: EvmTransactionRoute {
                chain_instance: EvmChainInstance {
                    chain_id: nonzero(1),
                    expected_genesis_hash: EvmHash::from_bytes([1; 32]),
                },
                endpoint_ref: EvmEndpoint::new("capacity-fixture")
                    .unwrap()
                    .endpoint_ref()
                    .unwrap(),
            },
            authority_epoch: EvmAuthorityEpoch::new([2; 32]),
            sender: EvmAddress::from_bytes([3; 20]),
        };
        let input = CapacityWorkflow {
            request: FixtureRequest { label: 99 },
            prior: CheckedCreatePlan::new(
                binding.clone(),
                vec![1; create_bytes],
                EvmU256::from_u64(0),
                nonzero(DEPLOYMENT_GAS),
                (PRIORITY_FEE) as u128,
                (MAX_FEE) as u128,
            )
            .unwrap(),
            deployment: CheckedCreatePlan::new(
                binding.clone(),
                vec![2; create_bytes],
                EvmU256::from_u64(0),
                nonzero(DEPLOYMENT_GAS),
                (PRIORITY_FEE) as u128,
                (MAX_FEE) as u128,
            )
            .unwrap(),
            configuration: CheckedCallPlan::new(
                binding.clone(),
                vec![3; call_bytes],
                EvmU256::from_u64(0),
                nonzero(CONFIGURATION_GAS),
                (PRIORITY_FEE) as u128,
                (MAX_FEE) as u128,
            )
            .unwrap(),
            observation: CheckedObservationPlan::new(binding.route.clone(), vec![4; call_bytes])
                .unwrap(),
        };
        let source = ScriptedEvidence::new(fault, returned);
        let store = Arc::new(RecordingStore::default());
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        register_evm_transaction_states::<CapacityInitial, FirstRecipe>(&mut builder).unwrap();
        register_evm_transaction_states::<AfterFirst, SecondRecipe>(&mut builder).unwrap();
        register_evm_transaction_states::<AfterSecond, CallRecipe>(&mut builder).unwrap();
        builder
            .register_read::<Observation, EvmAnchoredContractCallRead>()
            .unwrap();
        builder.register_pure::<CapacityDecode>().unwrap();
        builder
            .register_pure::<Abort<FirstFailure, AfterFirst>>()
            .unwrap();
        builder
            .register_pure::<Abort<SecondFailure, AfterSecond>>()
            .unwrap();
        builder
            .register_pure::<Abort<CallFailure, AfterCall>>()
            .unwrap();
        builder
            .register_pure::<Abort<ObservationFailure, AfterObservation>>()
            .unwrap();
        source.register(&mut builder, &binding);
        let runtime = Runtime::new(builder.finish(), store.clone());
        let id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(case).unwrap() + 40; 32],
        ));
        let program = expand_program(
            EntryPointId::new("mfm.test/capacity-contract@1").unwrap(),
            &CapacityOperation { binding },
        )
        .unwrap();
        let terminal = runtime
            .start(id.clone(), program, input.clone())
            .await
            .unwrap();
        let value = match (terminal.state(), expected_reason) {
            (RunViewState::Succeeded(value), None) => {
                let report: CapacityReport =
                    serde_json::from_slice(value.canonical_bytes()).unwrap();
                assert_eq!(report.context.request.label, 99);
                assert_eq!(report.decoded, EvmU256::from_u64(42));
                assert_eq!(report.context.prior.command(), &input.prior.command());
                assert_eq!(
                    report.context.deployment.command(),
                    &input.deployment.command()
                );
                assert_eq!(
                    report.context.configuration.command(),
                    &input.configuration.command_for(
                        report
                            .context
                            .deployment
                            .outcome()
                            .created_address()
                            .clone()
                    )
                );
                assert_ne!(
                    report.context.prior.outcome().created_address(),
                    report.context.configuration.outcome().target()
                );
                assert_eq!(
                    report.context.observation.intent().target(),
                    report.context.configuration.outcome().target()
                );
                assert_eq!(terminal.head_sequence(), 24);
                value
            }
            (RunViewState::Failed(value), Some(reason)) => {
                let report: FixtureFailure =
                    serde_json::from_slice(value.canonical_bytes()).unwrap();
                assert_eq!(report.reason(), reason);
                assert_eq!(report.request().label, 99);
                assert_eq!(report.entries().len(), 4);
                let mut wire = serde_json::to_value(&report).unwrap();
                wire["entries"].as_array_mut().unwrap().swap(0, 1);
                assert!(serde_json::from_value::<FixtureFailure>(wire).is_err());
                value
            }
            _ => panic!("capacity branch mismatch"),
        };
        assert!(value.canonical_bytes().len() < 8_388_608);
        {
            let frames = store.frames.lock().unwrap();
            let total: usize = frames.iter().map(Vec::len).sum();
            let largest = frames.iter().map(Vec::len).max().unwrap();
            assert!(total < 536_870_912);
            assert!(largest < 25_231_360);
            // Inspect every repeated object closure, rather than only the terminal payload.
            for frame in frames.iter() {
                #[derive(Deserialize)]
                struct Closure {
                    objects: Vec<Object>,
                }
                #[derive(Deserialize)]
                struct Object {
                    canonical: Box<serde_json::value::RawValue>,
                }
                let wire: Closure = serde_json::from_slice(frame).unwrap();
                assert!(!wire.objects.is_empty());
                for object in wire.objects {
                    assert!(object.canonical.get().len() < 8_388_608);
                }
            }
            println!("context capacity {case} {create_bytes}/{call_bytes} {fault:?}: value={} total={total} largest={largest}", value.canonical_bytes().len());
        }
        let cold = runtime.read(&id).await.unwrap();
        assert_eq!(cold.head_digest(), terminal.head_digest());
        let cold_value = match cold.state() {
            RunViewState::Succeeded(v) | RunViewState::Failed(v) => v,
            _ => panic!("cold terminal"),
        };
        assert_eq!(cold_value.canonical_bytes(), value.canonical_bytes());
    }
}
