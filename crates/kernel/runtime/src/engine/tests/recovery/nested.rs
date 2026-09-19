use super::*;
use crate::{RunViewState, RunnableReason, Runtime};
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{
    Checkpoint, CheckpointMarker, Handler, ProgramLimits, ProposedStateOutcome, Pure,
    RecoveryContext, RecoveryLimit, RecoveryRequest, StopReason,
};

pub(super) struct IncrementOffset;
impl mfm_program::State for IncrementOffset {
    type Input = Offset;
    type Output = Offset;
    type Failure = ReadFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.nested-restart/increment@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_program::PureState for IncrementOffset {
    fn evaluate(
        input: Offset,
    ) -> std::result::Result<
        ProposedStateOutcome<Offset, ReadFailure>,
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

pub(super) struct SelectRegion;
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

pub(super) struct NestedRegions {
    pub(super) restarts: u32,
}
pub(super) struct Outer;
pub(super) struct Inner;
impl CheckpointMarker for Outer {
    type Context = Offset;
}
impl CheckpointMarker for Inner {
    type Context = Offset;
}
impl OperationDefinition for NestedRegions {
    type Body = mfm_program::Operation<
        (
            Checkpoint<Outer>,
            mfm_program::Operation<(Pure<IncrementOffset>,), StopDefaults>,
            Checkpoint<Inner>,
            mfm_program::Operation<(Pure<IncrementOffset>,), StopDefaults>,
            Read<Observe, Observation>,
        ),
        RegionPolicy,
    >;
}
impl Plan<Offset> for NestedRegions {
    type Config = u32;
    fn plan<'a>(&'a self, _: &'a Offset) -> mfm_program::Result<(&'a u32, Self::Body)> {
        Ok((&self.restarts, Default::default()))
    }
}
// Earlier Pure work cannot inherit a target that still lies ahead. Installing Stop replaces
// the complete target unit; leaving each child restores the parent's two owned targets.
pub(super) struct StopDefaults;
impl OperationDefaults for StopDefaults {
    type Handler = mfm_program::Stop;
    type Targets = ();
}
impl ResolveDefaults<u32> for StopDefaults {
    fn resolve(_: &u32) -> mfm_program::Result<PolicyValues<mfm_program::Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(0),
        })
    }
}
pub(super) struct RegionPolicy;
impl OperationDefaults for RegionPolicy {
    type Handler = SelectRegion;
    type Targets = (Outer, Inner);
}
impl ResolveDefaults<u32> for RegionPolicy {
    fn resolve(restarts: &u32) -> mfm_program::Result<PolicyValues<SelectRegion>> {
        Ok(PolicyValues {
            handler: Some(Offset {
                value: u64::from(*restarts),
            }),
            retries: Some(0),
            restarts: Some(*restarts),
        })
    }
}
impl Resolve<u32, Observation> for ReadResources<false> {
    fn implementation(_: &u32) -> mfm_program::Result<StableId> {
        Ok(NativeRead::implementation_id()?)
    }
}
impl ResolveReadBinding<u32, Observation> for NativeRead {
    fn binding(_: &u32) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
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
        let resources = ReadResources::<false>(Arc::new(move |value| {
            observed.lock().unwrap().push(value);
            Box::pin(async move { Ok(Observed { value }) })
        }));
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([100 + case; 32]));
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test.nested-restart/run@1").unwrap(),
            &mfm_program::Operation::new(NestedRegions { restarts: local }),
            &Offset { value: 7 },
            &resources,
            ProgramLimits::new(global),
        )
        .unwrap();
        let mut view = admit(
            store.clone(),
            run.clone(),
            program,
            Object::from_value(&Offset { value: 7 }).unwrap(),
            Advancement::Manual,
        )
        .await
        .unwrap();
        let document = runtime.program_document(&run).await.unwrap();
        let cold_resources = ReadResources::<true>(Arc::clone(&resources.0));
        drop(resources);
        let program = mfm_program::load(document.canonical_bytes(), &cold_resources).unwrap();
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
            let cold = runtime.read(&run, &program).await.unwrap();
            assert_eq!(cold.head_digest(), view.head_digest());
            assert!(
                matches!(cold.state(), RunViewState::Runnable { position: restored, reason: RunnableReason::Restart { checkpoint: target } } if restored == position && target == selected)
            );
            assert_eq!(inputs.lock().unwrap().len(), decision + 1);
            view = runtime.resume(&run, &program).await.unwrap();
        }
        let RunViewState::Failed(report) = view.state() else {
            panic!("nested restart allowance exhaustion");
        };
        assert_eq!(report.reason(), &StopReason::Exhausted(expected_limit));
        assert_eq!(report.usage().state_restarts, committed as u32);
        assert_eq!(report.usage().run_decisions, committed as u32);
        assert_eq!(*inputs.lock().unwrap(), vec![9; committed + 1]);
        let cold = runtime.read(&run, &program).await.unwrap();
        let RunViewState::Failed(retained) = cold.state() else {
            panic!("cold exhaustion");
        };
        assert_eq!(retained.value_ref(), report.value_ref());
        assert_eq!(retained.canonical_bytes(), report.canonical_bytes());
        assert_eq!(inputs.lock().unwrap().len(), committed + 1);
    }
}
