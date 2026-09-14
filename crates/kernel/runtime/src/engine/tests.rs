use super::*;
use crate::{Runtime, RuntimeAssemblyBuilder};
use mfm_capabilities::{AdapterError, CapabilityError};
use mfm_ids::{DigestBytes, EntryPointId, StableId};
use mfm_program::{
    CapabilityInjection, Classification, HandlerBinding, Identity, Never, NoParams, Occurrence,
    OperationExpansion, ProgramError, ProgramLimits, RecoveryAllowances, StandardRecovery, State,
};
use mfm_program_derive::MfmValue;
use mfm_store::MemoryStore;
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

// Scope the fault to this test's run; it is consumed once before any projection is constructed.
static PROJECTION_FAULT: Mutex<Option<(RunId, u64, usize)>> = Mutex::new(None);

pub(super) fn projection_fault(driver: &Driver) -> Result<()> {
    let mut fault = PROJECTION_FAULT.lock().unwrap();
    if let Some((run, sequence, skip)) = fault.as_mut() {
        if run == driver.head.run_id() && *sequence == driver.head.head_sequence() {
            if *skip > 0 {
                *skip -= 1;
            } else {
                *fault = None;
                return Err(RuntimeError::at(
                    Operation::Project,
                    Stage::Project,
                    InvocationDiagnostic::from_fields("projection_test", "view", &"injected", None),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Outage {}
impl ClassifyError for Outage {
    fn classify(&self) -> Classification {
        Classification::Retryable
    }
}
struct Mutation;
impl EffectCapabilityContract for Mutation {
    type Command = NoParams;
    type Evidence = NoParams;
    type OperationalError = Outage;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.projection/mutation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &EffectId,
        _: &NoParams,
        _: &NoParams,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        panic!("these pending attempts never supply settlement evidence")
    }
}
struct Execute;
impl State for Execute {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.projection/execute@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Mutation> for Execute {
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, InvocationDiagnostic> {
        Ok(NoParams)
    }
    fn interpret(
        _: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Never>, InvocationDiagnostic> {
        panic!("these pending attempts never reach interpretation")
    }
}
impl CapabilityInjection<Execute> for Mutation {
    type Setup = NoParams;
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct Flow {
    retry: bool,
}
impl mfm_program::Operation for Flow {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn validate_input(&self, _: &NoParams) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<NoParams, NoParams, Never>,
    ) -> mfm_program::Result<()> {
        body.handler(HandlerBinding::new::<StandardRecovery>(NoParams)?)?;
        body.allowances(RecoveryAllowances::new(u32::from(self.retry), 0))?;
        body.effect::<Execute, Mutation, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}

#[tokio::test]
async fn projection_acknowledgement_requires_insertion_and_retains_previous_observation() {
    // The last two cases enter both recovered-record projection branches: Retry yields and Stop
    // returns RecoveryStopped. All invocations use Runtime entry points and a real MemoryStore.
    for (case, fail_sequence, skip, resume, operational, retry, previous_sequence) in [
        (0, 1, 0, false, false, false, None), // initial admission inserted
        (1, 2, 0, false, false, false, Some(1)), // prepared continuation inserted
        (2, 2, 0, true, false, false, None),  // resume's initial projection, no insertion
        (3, 2, 1, true, false, false, Some(2)), // Pending projection, no insertion
        (4, 4, 0, false, true, true, Some(3)), // recovery Retry inserted
        (5, 4, 0, false, true, false, Some(3)), // recovery Stop inserted
    ] {
        let run = RunId::from_digest(DigestBytes::from_array([220 + case; 32]));
        let store = Arc::new(MemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Execute, Mutation>().unwrap();
        builder.register_handler::<StandardRecovery>().unwrap();
        builder
            .register_effect_adapter::<Mutation, _, _>(NoParams, move |_, _, _| {
                counted.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if operational {
                        Err(AdapterError::Operational(Outage {}))
                    } else {
                        Ok(EffectAdapterOutcome::Pending)
                    }
                })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), store.clone());
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/projection-acknowledgement@1").unwrap(),
            &Flow { retry },
            &NoParams,
            ProgramLimits::new(1),
        )
        .unwrap();
        let pending = if resume {
            Some(
                runtime
                    .start(run.clone(), program.clone(), NoParams)
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        let before_calls = calls.load(Ordering::SeqCst);
        *PROJECTION_FAULT.lock().unwrap() = Some((run.clone(), fail_sequence, skip));
        let result = if resume {
            runtime.resume(&run).await
        } else {
            runtime.start(run.clone(), program, NoParams).await
        };
        let InvocationFailure::Execution {
            run_id,
            error,
            last_observed,
        } = result.err().unwrap()
        else {
            panic!("projection failure must end the invocation")
        };
        assert!(
            PROJECTION_FAULT.lock().unwrap().is_none(),
            "projection fault must be reached"
        );
        assert_eq!(run_id, run);
        let loaded = store
            .load_run(&run, previous_sequence)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.head().head_sequence(), fail_sequence);
        let cause = if resume {
            assert!(
                !matches!(error, RuntimeError::Projection { .. }),
                "no insertion in this invocation"
            );
            let pending = pending.as_ref().unwrap();
            assert_eq!(loaded.head().head_digest(), pending.head_digest());
            &error
        } else {
            let RuntimeError::Projection {
                acknowledged,
                cause,
            } = &error
            else {
                panic!("known insertion must retain its mechanical acknowledgement")
            };
            assert_eq!(acknowledged.as_ref(), loaded.head());
            cause.as_ref()
        };
        let RuntimeError::Native {
            operation: Operation::Project,
            stage: Stage::Project,
            cause,
        } = cause
        else {
            panic!("the producing projection diagnostic must survive")
        };
        assert_eq!(cause.code(), "projection_test");
        assert_eq!(cause.details().as_value(), &serde_json::json!("injected"));
        assert_eq!(
            last_observed.as_ref().map(|view| view.head_sequence()),
            previous_sequence
        );
        if let Some(previous) = last_observed {
            let frame = decode_frame(loaded.probe().unwrap()).unwrap();
            assert_eq!(previous.head_digest(), frame.head_digest());
            if resume {
                assert!(matches!(
                    previous.state(),
                    RunViewState::EffectPending { .. }
                ));
            } else if operational {
                assert!(matches!(
                    previous.state(),
                    RunViewState::AwaitingRecovery { .. }
                ));
            } else {
                assert!(matches!(previous.state(), RunViewState::Runnable { .. }));
            }
        }
        assert_eq!(
            calls.load(Ordering::SeqCst) - before_calls,
            usize::from(operational || (resume && skip == 1))
        );
        // Inspection succeeds after the one-shot failure and cannot advance the acknowledged head.
        assert_eq!(
            runtime.read(&run).await.unwrap().head_digest(),
            loaded.head().head_digest()
        );
    }
}
