use super::accumulating_contract::{Fault, RecordingStore, ScriptedEvidence};
use super::*;
use mfm_evm::{
    AnchoredContractCallFailure, AnchoredContractCallFailureReason, AnchoredObservationFacts,
    CallCreatedAt, Called, CompletedTransactionFacts, CreateAt, Created, EvmChainInstance,
    EvmTransaction, EvmTransactionFailure, ExecutedTransactionFacts, ObserveAt,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct CreationPlans {
    prior: CheckedCreatePlan,
    deployment: CheckedCreatePlan,
}

#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct CapacityFailure {
    request: FixtureRequest,
    plans: workflow::FailurePlans<CreationPlans>,
    progress: workflow::CreateProgress<workflow::CreateProgress<workflow::CallProgress>>,
}
impl CapacityFailure {
    fn new(
        request: FixtureRequest,
        plans: workflow::FailurePlans<CreationPlans>,
        progress: workflow::CreateProgress<workflow::CreateProgress<workflow::CallProgress>>,
    ) -> Result<Self, StateExecutionError> {
        progress.validate(&plans.creations.prior)?;
        if let Some(deployment) = &progress.next {
            let target = deployment.validate(&plans.creations.deployment)?;
            if let (Some(call), Some(target)) = (&deployment.next, target) {
                call.validate(&plans, &target)?;
            }
        }
        Ok(Self {
            request,
            plans,
            progress,
        })
    }
    fn reason(&self) -> FixtureFailureReason {
        match &self.progress.next {
            None => FixtureFailureReason::PriorDeploymentReverted,
            Some(deployment) => deployment.next.as_ref().map_or(
                FixtureFailureReason::DeploymentReverted,
                workflow::CallProgress::reason,
            ),
        }
    }
    fn from_observed(context: AfterObservation) -> Result<Self, StateExecutionError> {
        let plans = workflow::FailurePlans {
            creations: CreationPlans {
                prior: workflow::create_plan(context.prior.command())?,
                deployment: workflow::create_plan(context.deployment.command())?,
            },
            configuration: workflow::call_plan(context.configuration.command())?,
            observation: workflow::observation_plan(
                &context.observation,
                context.configuration.command().binding(),
            )?,
        };
        let call = workflow::CallProgress {
            target: context.configuration.outcome().target().clone(),
            evidence: workflow::TransactionEvidence::from_executed(
                context.configuration.executed(),
            ),
            observation: Some(context.observation),
        };
        let deployment = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(context.deployment.executed()),
            next: Some(call),
        };
        let progress = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(context.prior.executed()),
            next: Some(deployment),
        };
        Self::new(context.request, plans, progress)
    }
}
impl<'de> Deserialize<'de> for CapacityFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: FixtureRequest,
            plans: workflow::FailurePlans<CreationPlans>,
            progress: workflow::CreateProgress<workflow::CreateProgress<workflow::CallProgress>>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.request, wire.plans, wire.progress).map_err(serde::de::Error::custom)
    }
}
impl
    TryFrom<
        EvmTransactionFailure<
            CapacityWorkflow<
                ExecutedTransactionFacts,
                CheckedCreatePlan,
                CheckedCallPlan,
                CheckedObservationPlan,
            >,
        >,
    > for CapacityFailure
{
    type Error = StateExecutionError;
    fn try_from(value: FirstFailure) -> Result<Self, Self::Error> {
        let context = value.into_context();
        let plans = workflow::FailurePlans {
            creations: CreationPlans {
                prior: workflow::create_plan(context.prior.command())?,
                deployment: context.deployment,
            },
            configuration: context.configuration,
            observation: context.observation,
        };
        let progress = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(&context.prior),
            next: None,
        };
        Self::new(context.request, plans, progress)
    }
}
impl
    TryFrom<
        EvmTransactionFailure<
            CapacityWorkflow<
                CompletedTransactionFacts<Created>,
                ExecutedTransactionFacts,
                CheckedCallPlan,
                CheckedObservationPlan,
            >,
        >,
    > for CapacityFailure
{
    type Error = StateExecutionError;
    fn try_from(value: SecondFailure) -> Result<Self, Self::Error> {
        let context = value.into_context();
        let plans = workflow::FailurePlans {
            creations: CreationPlans {
                prior: workflow::create_plan(context.prior.command())?,
                deployment: workflow::create_plan(context.deployment.command())?,
            },
            configuration: context.configuration,
            observation: context.observation,
        };
        let progress = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(context.prior.executed()),
            next: Some(workflow::CreateProgress {
                evidence: workflow::TransactionEvidence::from_executed(&context.deployment),
                next: None,
            }),
        };
        Self::new(context.request, plans, progress)
    }
}
impl
    TryFrom<
        EvmTransactionFailure<
            CapacityWorkflow<
                CompletedTransactionFacts<Created>,
                CompletedTransactionFacts<Created>,
                ExecutedTransactionFacts,
                CheckedObservationPlan,
            >,
        >,
    > for CapacityFailure
{
    type Error = StateExecutionError;
    fn try_from(value: CallFailure) -> Result<Self, Self::Error> {
        let context = value.into_context();
        let plans = workflow::FailurePlans {
            creations: CreationPlans {
                prior: workflow::create_plan(context.prior.command())?,
                deployment: workflow::create_plan(context.deployment.command())?,
            },
            configuration: workflow::call_plan(context.configuration.command())?,
            observation: context.observation,
        };
        let call = workflow::CallProgress {
            target: context
                .configuration
                .command()
                .to()
                .ok_or(StateExecutionError)?
                .clone(),
            evidence: workflow::TransactionEvidence::from_executed(&context.configuration),
            observation: None,
        };
        let deployment = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(context.deployment.executed()),
            next: Some(call),
        };
        let progress = workflow::CreateProgress {
            evidence: workflow::TransactionEvidence::from_executed(context.prior.executed()),
            next: Some(deployment),
        };
        Self::new(context.request, plans, progress)
    }
}
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
    > for CapacityFailure
{
    type Error = StateExecutionError;
    fn try_from(value: ObservationFailure) -> Result<Self, Self::Error> {
        Self::from_observed(value.into_context())
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
    type Failure = CapacityFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.two-creations/decode@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for CapacityDecode {
    fn evaluate(
        input: AfterObservation,
    ) -> Result<ProposedStateOutcome<CapacityReport, CapacityFailure>, StateExecutionError> {
        let result = input.observation.result().ok_or(StateExecutionError)?;
        Ok(match decode_fixture_value(result.return_bytes()) {
            Some(decoded) => ProposedStateOutcome::Success {
                output: CapacityReport {
                    context: input,
                    decoded,
                },
            },
            None => ProposedStateOutcome::Failure {
                failure: CapacityFailure::from_observed(input)?,
            },
        })
    }
}
struct CapacityOperation {
    binding: EvmTransactionBinding,
    bound: mfm_program::ConclusionBound,
}
impl Operation for CapacityOperation {
    type Input = CapacityInitial;
    type Output = CapacityReport;
    type Failure = CapacityFailure;
    fn validate_input(&self, input: &CapacityInitial) -> mfm_program::Result<()> {
        if input.prior.binding() != &self.binding
            || input.deployment.binding() != &self.binding
            || input.configuration.binding() != &self.binding
            || input.observation.route_ref()
                != &self
                    .binding
                    .route
                    .binding_ref()
                    .map_err(|_| ProgramError::InvalidContract)?
            || input.observation.chain_id() != self.binding.route.chain_instance.chain_id
        {
            return Err(ProgramError::InvalidContract);
        }
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<CapacityInitial, CapacityReport, CapacityFailure>,
    ) -> mfm_program::Result<()> {
        use mfm_program::{Identity, NoParams, Occurrence};
        let bounds = transaction_bounds(self.bound);
        body.operation::<FirstTransaction, MapFailure<FirstFailure, CapacityFailure>>(
            &FirstTransaction::new(self.binding.clone(), bounds),
            NoParams,
        )?;
        body.operation::<SecondTransaction, MapFailure<SecondFailure, CapacityFailure>>(
            &SecondTransaction::new(self.binding.clone(), bounds),
            NoParams,
        )?;
        body.operation::<CallTransaction, MapFailure<CallFailure, CapacityFailure>>(
            &CallTransaction::new(self.binding.clone(), bounds),
            NoParams,
        )?;
        body.read::<Observation, EvmAnchoredContractCallRead, MapFailure<ObservationFailure, CapacityFailure>>(&self.binding.route, NoParams, Occurrence::new(), self.bound)?;
        body.pure::<CapacityDecode, Identity<CapacityFailure>>(
            NoParams,
            Occurrence::new(),
            self.bound,
        )
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
            CapacityFailure::schema_descriptor()
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
            .register_map::<MapFailure<FirstFailure, CapacityFailure>>()
            .unwrap();
        builder
            .register_map::<MapFailure<SecondFailure, CapacityFailure>>()
            .unwrap();
        builder
            .register_map::<MapFailure<CallFailure, CapacityFailure>>()
            .unwrap();
        builder
            .register_map::<MapFailure<ObservationFailure, CapacityFailure>>()
            .unwrap();
        source.register(&mut builder, &binding);
        let runtime = Runtime::new(builder.finish(), store.clone());
        let id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(case).unwrap() + 40; 32],
        ));
        let program = expand_program(
            EntryPointId::new("mfm.test/capacity-contract@1").unwrap(),
            &CapacityOperation {
                binding,
                bound: fixture_bound(&input),
            },
            &input,
            mfm_program::ProgramLimits::new(1),
        )
        .unwrap();
        let terminal = runtime
            .start(id.clone(), program.clone(), input.clone())
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
                value.canonical_bytes()
            }
            (RunViewState::Failed(value), Some(reason)) => {
                let report: CapacityFailure = root_failure(value);
                assert_eq!(report.reason(), reason);
                assert_eq!(report.request.label, 99);
                assert_eq!(report.plans.creations.prior, input.prior);
                assert_eq!(report.plans.creations.deployment, input.deployment);
                assert_eq!(report.plans.configuration, input.configuration);
                assert_eq!(report.plans.observation, input.observation);
                let mut wire = serde_json::to_value(&report).unwrap();
                let prior = wire["plans"]["creations"]["prior"].clone();
                wire["plans"]["creations"]["prior"] =
                    wire["plans"]["creations"]["deployment"].clone();
                wire["plans"]["creations"]["deployment"] = prior;
                assert!(serde_json::from_value::<CapacityFailure>(wire).is_err());
                value.canonical_bytes()
            }
            _ => panic!("capacity branch mismatch"),
        };
        assert!(value.len() < 8_388_608);
        {
            let frames = store.frames.lock().unwrap();
            let total: usize = frames.iter().map(Vec::len).sum();
            let largest = frames.iter().map(Vec::len).max().unwrap();
            assert!(total < 536_870_912);
            assert!(largest < 25_231_360);
            let admitted = program
                .history_bound(mfm_program::ConclusionBound::new(frames[0].len() as u64).unwrap())
                .unwrap();
            assert_eq!(admitted.frames(), 47);
            assert!(admitted.bytes() <= mfm_journal::MAX_RUN_BYTES);
            assert!(total as u64 <= admitted.bytes());
            assert!(frames
                .iter()
                .skip(1)
                .all(|frame| frame.len() as u64 <= fixture_bound(&input).max_frame_bytes()));
            println!(
                "admitted {} frames / {} bytes",
                admitted.frames(),
                admitted.bytes()
            );
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
            println!("context capacity {case} {create_bytes}/{call_bytes} {fault:?}: value={} total={total} largest={largest}", value.len());
        }
        let cold = runtime.read(&id).await.unwrap();
        assert_eq!(cold.head_digest(), terminal.head_digest());
        let cold_value = match cold.state() {
            RunViewState::Succeeded(v) => v.canonical_bytes(),
            RunViewState::Failed(v) => v.canonical_bytes(),
            _ => panic!("cold terminal"),
        };
        assert_eq!(cold_value, value);
    }
}
