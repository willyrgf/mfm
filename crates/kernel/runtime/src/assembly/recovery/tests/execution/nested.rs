use super::*;
use crate::{RunViewState, RunnableReason, Runtime};
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{Occurrence, ProgramLimits, ProposedStateOutcome, RecoveryLimit, StopReason};

struct IncrementOffset;
impl mfm_program::State for IncrementOffset {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.nested-restart/increment@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_program::PureState for IncrementOffset {
    fn evaluate(
        input: Offset,
    ) -> std::result::Result<
        ProposedStateOutcome<Offset, EvmFailure>,
        mfm_values::InvocationDiagnostic,
    > {
        Ok(ProposedStateOutcome::Success {
            output: Offset {
                value: input.value.checked_add(1).ok_or_else(|| {
                    mfm_values::InvocationDiagnostic::from_fields(
                        "runtime_invariant",
                        "apply",
                        &"arithmetic_overflow",
                        None,
                    )
                })?,
            },
        })
    }
}

struct SelectRegion;
impl Handler for SelectRegion {
    type Params = Offset;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.nested-restart/select-region@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        initial: &Offset,
        _: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, mfm_values::InvocationDiagnostic> {
        let selected = (initial.value - u64::from(context.remaining().restarts()) + 1) % 2;
        Ok(context
            .eligible_restart_targets()
            .iter()
            .find(|target| target.position().index() as u64 == selected)
            .copied()
            .map(RecoveryRequest::Restart)
            .unwrap_or(RecoveryRequest::Stop))
    }
}

struct NestedRegions {
    restarts: u32,
}
impl mfm_program::Operation for NestedRegions {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn validate_input(&self, _: &Offset) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, EvmFailure>,
    ) -> mfm_program::Result<()> {
        let outer = scope.checkpoint::<Offset>()?;
        scope.pure::<IncrementOffset, Identity<EvmFailure>>(NoParams, Occurrence::new())?;
        let inner = scope.checkpoint::<Offset>()?;
        scope.pure::<IncrementOffset, Identity<EvmFailure>>(NoParams, Occurrence::new())?;
        scope.handler(
            HandlerBinding::new::<SelectRegion>(Offset {
                value: u64::from(self.restarts),
            })?
            .checkpoint(&outer)?
            .checkpoint(&inner)?,
        )?;
        scope.allowances(RecoveryAllowances::new(0, self.restarts))?;
        scope.read::<EvmRead, Observation, Identity<EvmFailure>>(
            &Offset { value: 1 },
            NoParams,
            Occurrence::new(),
        )
    }
}

// Alternating between nested checkpoints must restore saved inputs while sharing the same finite
// State and run recovery budgets.
#[tokio::test]
async fn nested_restarts_restore_inputs_without_resetting_local_or_global_allowances() {
    for (case, local, global, expected_limit, committed) in [
        (1, 5, 3, RecoveryLimit::Run, 3),
        (2, 2, 5, RecoveryLimit::StateRestart, 2),
    ] {
        let inputs = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = inputs.clone();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_pure::<IncrementOffset>().unwrap();
        builder.register_read::<EvmRead, Observation>().unwrap();
        builder.register_handler::<SelectRegion>().unwrap();
        builder
            .register_adapter::<Observation, _, _>(Offset { value: 1 }, move |_, intent| {
                let mut inputs = observed.lock().unwrap();
                inputs.push(intent.value);
                // Alternate between the inner and outer region; both must restore their saved input.
                let target = (inputs.len() % 2) as u64;
                Box::pin(async move { Ok(Offset { value: target }) })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([100 + case; 32]));
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test.nested-restart/run@1").unwrap(),
            &NestedRegions { restarts: local },
            &Offset { value: 7 },
            ProgramLimits::new(global),
        )
        .unwrap();
        let mut view = runtime
            .start(run.clone(), program, Offset { value: 7 })
            .await
            .unwrap();
        for (decision, (checkpoint, visit, head)) in [(1, 3, 5), (0, 5, 8), (1, 8, 12)]
            .into_iter()
            .take(committed)
            .enumerate()
        {
            let RunViewState::Runnable {
                position,
                reason:
                    RunnableReason::Restart {
                        checkpoint: selected,
                    },
            } = view.state()
            else {
                panic!("committed nested restart");
            };
            assert_eq!(selected.index(), checkpoint);
            assert_eq!(position.state, *selected);
            assert_eq!(position.visit.value(), visit);
            assert_eq!(view.head_sequence(), head);
            let cold = runtime.read(&run).await.unwrap();
            assert_eq!(cold.head_digest(), view.head_digest());
            assert!(
                matches!(cold.state(), RunViewState::Runnable { position: restored, reason: RunnableReason::Restart { checkpoint: target } } if restored == position && target == selected)
            );
            assert_eq!(inputs.lock().unwrap().len(), decision + 1);
            view = runtime.resume(&run).await.unwrap();
        }
        let RunViewState::Failed(report) = view.state() else {
            panic!("nested restart allowance exhaustion");
        };
        assert_eq!(report.reason(), &StopReason::Exhausted(expected_limit));
        assert_eq!(report.usage().state_restarts, committed as u32);
        assert_eq!(report.usage().run_decisions, committed as u32);
        assert_eq!(*inputs.lock().unwrap(), vec![9; committed + 1]);
        let cold = runtime.read(&run).await.unwrap();
        let RunViewState::Failed(retained) = cold.state() else {
            panic!("cold exhaustion");
        };
        assert_eq!(retained.value_ref(), report.value_ref());
        assert_eq!(retained.canonical_bytes(), report.canonical_bytes());
        assert_eq!(inputs.lock().unwrap().len(), committed + 1);
    }
}
