use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_ids::{DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_program::{
    expand_program, ConclusionBound, EffectBounds, EffectState, Identity, NoContext, NoParams,
    Occurrence, Operation, OperationExpansion, PreparationError, ProgramLimits,
    ProposedStateOutcome, PureState, State, StateExecutionError,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
    SizeResource,
};
use mfm_store::MemoryStore;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    bytes: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Failure {
    detail: String,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Evidence {
    effect_id: EffectId,
}

struct Mutation;
impl EffectCapabilityContract for Mutation {
    type OperationalError = NoContext;
    type Command = Input;
    type Evidence = Evidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.capacity/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        id: &EffectId,
        _: &Input,
        evidence: &Evidence,
    ) -> mfm_capabilities::Result<()> {
        if id == &evidence.effect_id {
            Ok(())
        } else {
            Err(CapabilityError::EvidenceBinding)
        }
    }
}
struct Failing;
impl State for Failing {
    type Input = Input;
    type Output = Input;
    type Failure = Failure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.capacity/failing@1")
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
}
impl PureState for Failing {
    fn evaluate(input: Input) -> Result<ProposedStateOutcome<Input, Failure>, StateExecutionError> {
        Ok(ProposedStateOutcome::Failure {
            failure: Failure {
                detail: "x".repeat(input.bytes as usize),
            },
        })
    }
}
impl EffectState<Mutation> for Failing {
    type AdapterContext = Failure;
    fn adapter_context(
        input: &Input,
        _: &Input,
        _: &NoContext,
    ) -> Result<Failure, StateExecutionError> {
        Ok(Failure {
            detail: "x".repeat(input.bytes as usize),
        })
    }
    fn prepare(input: &Input) -> Result<Input, PreparationError> {
        Ok(Input { bytes: input.bytes })
    }
    fn interpret(
        input: Input,
        _: &Evidence,
    ) -> Result<ProposedStateOutcome<Input, Failure>, StateExecutionError> {
        Self::evaluate(input)
    }
}
impl mfm_program::CapabilityInjection<Failing> for Mutation {
    type FailureMap = Identity<Failure>;
    type Setup = NoParams;
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
    type ExpandedFailure = Failure;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<mfm_ids::ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
}
struct Plan {
    effect: bool,
}
impl Operation for Plan {
    type Input = Input;
    type Output = Input;
    type Failure = Failure;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Input, Input, Failure>,
    ) -> mfm_program::Result<()> {
        if self.effect {
            scope.effect::<Failing, Mutation, Identity<Failure>>(
                &NoParams,
                NoParams,
                Occurrence::new(),
                EffectBounds::new(65536, 40 * 1024 * 1024)?,
            )
        } else {
            scope.pure::<Failing, Identity<Failure>>(
                NoParams,
                Occurrence::new(),
                ConclusionBound::new(40 * 1024 * 1024)?,
            )
        }
    }
}

#[tokio::test]
async fn inline_report_size_preserves_original_values_and_pending_authority() {
    for effect in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut assembly = RuntimeAssemblyBuilder::new().unwrap();
        if effect {
            assembly.register_effect::<Failing, Mutation>().unwrap();
            assembly
                .register_effect_adapter::<Mutation, _, _>(NoParams, {
                    let calls = Arc::clone(&calls);
                    move |id, _, _| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let effect_id = id.clone();
                        Box::pin(async move {
                            Ok(EffectAdapterOutcome::Settled(Evidence { effect_id }))
                        })
                    }
                })
                .unwrap();
        } else {
            assembly.register_pure::<Failing>().unwrap();
        }
        let runtime = Runtime::new(assembly.finish(), Arc::new(MemoryStore::new()));
        for (index, bytes) in [5 * 1024 * 1024, 17 * 1024 * 1024, 33 * 1024 * 1024]
            .into_iter()
            .enumerate()
        {
            let id = RunId::from_digest(DigestBytes::from_array(
                [index as u8 + if effect { 170 } else { 160 }; 32],
            ));
            let program = expand_program(
                EntryPointId::new("mfm.test.capacity/report@1").unwrap(),
                &Plan { effect },
                &Input { bytes },
                ProgramLimits::new(0),
            )
            .unwrap();
            let result = runtime.start(id.clone(), program, Input { bytes }).await;
            if index == 0 {
                let hot = result.unwrap();
                let RunViewState::Failed(report) = hot.state() else {
                    panic!("expected durable failure")
                };
                let wire: serde_json::Value =
                    serde_json::from_slice(report.canonical_bytes()).unwrap();
                assert_eq!(wire["cause"]["original"], wire["cause"]["root"]);
                assert!(report.canonical_bytes().len() > 10 * 1024 * 1024);
                let before = calls.load(Ordering::SeqCst);
                let cold = runtime.read(&id).await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), before);
                let RunViewState::Failed(cold_report) = cold.state() else {
                    panic!("cold failure")
                };
                assert_eq!(report.canonical_bytes(), cold_report.canonical_bytes());
                assert_eq!(report.value_ref(), cold_report.value_ref());
            } else {
                let Err(InvocationFailure::Execution {
                    error,
                    last_observed: Some(observed),
                    ..
                }) = result
                else {
                    panic!("expected stopped invocation")
                };
                let (resource, size) = error.size_limit().expect("precise size failure");
                assert_eq!(
                    resource,
                    if index == 1 {
                        SizeResource::FailureReport
                    } else {
                        SizeResource::CanonicalObject
                    }
                );
                assert_eq!(size.limit(), 33_554_432);
                assert!(size.actual() > size.limit());
                let before = calls.load(Ordering::SeqCst);
                let cold = runtime.read(&id).await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), before);
                assert_eq!(cold.head_sequence(), if effect { 2 } else { 1 });
                assert_eq!(cold.head_digest(), observed.head_digest());
                if effect {
                    let RunViewState::EffectPending { effect_id, .. } = observed.state() else {
                        panic!("prepare lost")
                    };
                    let RunViewState::EffectPending {
                        effect_id: retained,
                        ..
                    } = cold.state()
                    else {
                        panic!("cold prepare lost")
                    };
                    assert_eq!(effect_id, retained);
                } else {
                    assert!(matches!(cold.state(), RunViewState::Runnable { .. }));
                }
            }
        }
    }
}

#[path = "support/qualification_capacity.rs"]
mod qualification_capacity;
