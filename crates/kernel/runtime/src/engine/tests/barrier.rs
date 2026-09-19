use super::*;
use mfm_capabilities::{ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_program::{
    BindRead, Checkpoint, CheckpointMarker, Handler, InjectRead, Read, ReadSelection, ReadState,
    RecoveryContext, RecoveryLimit, RecoveryRequest, ResolveReadBinding, ResolvedEffect,
    StopReason,
};

struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = NoParams;
    type Evidence = NoParams;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.barrier/read@1")?)
    }
    fn bind_evidence(
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        Ok(())
    }
}
struct Observe;
impl State for Observe {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Outage;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.barrier/observe@1")?)
    }
}
impl ReadState<Observation> for Observe {
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, InvocationDiagnostic> {
        Ok(NoParams)
    }
    fn interpret(
        _: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Outage>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure { failure: Outage {} })
    }
}
impl ReadSelection<Observation> for Observe {
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
}
#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Selection {
    injected: bool,
    before_checkpoint: bool,
}
struct NativeRead;
impl ReadImplementation<Observation> for NativeRead {
    type Binding = Selection;
    type NativeIntent = NoParams;
    type NativeEvidence = NoParams;
    type OperationalError = Outage;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.barrier/native@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &Selection,
        _: &NoParams,
    ) -> std::result::Result<NoParams, mfm_capabilities::CallbackFailure> {
        Ok(NoParams)
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &Selection,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
        _: &NoParams,
        _: &Object,
    ) -> std::result::Result<NoParams, mfm_capabilities::CallbackFailure> {
        Ok(NoParams)
    }
}
type ExternalAction = ResolvedEffect<Execute, Mutation, Native>;
impl InjectRead<Observe, Observation> for NativeRead {
    type Prefix = Vec<ExternalAction>;
    type Suffix = Identity<NoParams>;
    fn surround(binding: &Selection) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            if binding.injected {
                vec![ExternalAction::new(NoParams)]
            } else {
                vec![]
            },
            Identity::default(),
        ))
    }
}
impl ResolveReadBinding<Selection, Observation> for NativeRead {
    fn binding(config: &Selection) -> mfm_program::Result<Selection> {
        Ok(config.clone())
    }
}
struct Marker;
impl CheckpointMarker for Marker {
    type Context = NoParams;
}
struct RestartFirst;
impl Handler for RestartFirst {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.barrier/restart@1")?)
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        Ok(context
            .eligible_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .unwrap_or(RecoveryRequest::Stop))
    }
}
struct RestartPolicy;
impl OperationDefaults for RestartPolicy {
    type Handler = RestartFirst;
    type Targets = (Marker,);
}
impl ResolveDefaults<Selection> for RestartPolicy {
    fn resolve(_: &Selection) -> mfm_program::Result<PolicyValues<RestartFirst>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(1),
        })
    }
}
struct Clear;
impl OperationDefaults for Clear {
    type Handler = mfm_program::Stop;
    type Targets = ();
}
impl ResolveDefaults<Selection> for Clear {
    fn resolve(_: &Selection) -> mfm_program::Result<PolicyValues<mfm_program::Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(0),
        })
    }
}
struct Region {
    selection: Selection,
}
impl OperationDefinition for Region {
    type Body = mfm_program::Operation<RegionBody, RestartPolicy>;
}
impl Plan<NoParams> for Region {
    type Config = Selection;
    fn plan<'a>(&'a self, _: &'a NoParams) -> mfm_program::Result<(&'a Selection, Self::Body)> {
        Ok((&self.selection, Default::default()))
    }
}
#[derive(Default)]
struct RegionBody;
impl OperationDefinition for RegionBody {
    type Body = (
        mfm_program::Operation<Before, Clear>,
        Checkpoint<Marker>,
        Read<Observe, Observation>,
    );
}
impl Plan<Selection> for RegionBody {
    type Config = Selection;
    fn plan<'a>(
        &'a self,
        config: &'a Selection,
    ) -> mfm_program::Result<(&'a Selection, Self::Body)> {
        Ok((config, Default::default()))
    }
}
#[derive(Default)]
struct Before;
impl OperationDefinition for Before {
    type Body = Vec<ExternalAction>;
}
impl Plan<Selection> for Before {
    type Config = Selection;
    fn plan<'a>(
        &'a self,
        config: &'a Selection,
    ) -> mfm_program::Result<(&'a Selection, Self::Body)> {
        Ok((
            config,
            if config.before_checkpoint {
                vec![ExternalAction::new(NoParams)]
            } else {
                vec![]
            },
        ))
    }
}
struct Environment(Resources);
impl ProgramEnvironment for Environment {
    type Sources = mfm_program::Operation<Region>;
}
impl CapabilityFamily<Observation> for Environment {
    type Implementations = (NativeRead,);
}
impl CapabilityFamily<Mutation> for Environment {
    type Implementations = (Native,);
}
impl Resolve<Selection, Observation> for Environment {
    fn implementation(_: &Selection) -> mfm_program::Result<StableId> {
        Ok(NativeRead::implementation_id()?)
    }
}
impl BindRead<Observation, NativeRead> for Environment {
    type Adapter = Self;
    fn bind_read(&self, _: &Selection) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self(self.0.clone()))
    }
}
impl BindEffect<Mutation, Native> for Environment {
    type Adapter = Resources;
    fn bind_effect(&self, _: &NoParams) -> std::result::Result<Resources, InvocationDiagnostic> {
        Ok(self.0.clone())
    }
}
impl ReadAdapter<NoParams, NoParams, Outage> for Environment {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a NoParams,
    ) -> Pin<
        Box<dyn Future<Output = std::result::Result<NoParams, AdapterError<Outage>>> + Send + 'a>,
    > {
        Box::pin(async { Ok(NoParams) })
    }
}

// Native injection must not permit recovery to replay an external action. A checkpoint after
// an acknowledged Effect remains usable and must not repeat the preceding action.
#[tokio::test]
async fn injected_effect_blocks_prior_checkpoint_but_preserves_post_effect_restart() {
    for (case, before_checkpoint, injected) in
        [(0, false, false), (1, false, true), (2, true, false)]
    {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resources = Environment(Resources {
            calls: calls.clone(),
            operational: false,
            settled: true,
        });
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/barrier@1").unwrap(),
            &mfm_program::Operation::new(Region {
                selection: Selection {
                    injected,
                    before_checkpoint,
                },
            }),
            &NoParams,
            &resources,
            ProgramLimits::new(1),
        )
        .unwrap();
        let runtime = Runtime::new(Arc::new(MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([50 + case; 32]));
        let first = runtime
            .start(run.clone(), &program, &NoParams)
            .await
            .unwrap();
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let program = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
        let cold = runtime.read(&run, &program).await.unwrap();
        assert_eq!(cold.head_digest(), first.head_digest());
        if injected {
            let report = first
                .failure()
                .expect("external action invalidates earlier restart target");
            assert_eq!(report.reason(), &StopReason::Requested);
            assert_eq!(report.usage().run_decisions, 0);
            assert_eq!(calls.lock().unwrap().len(), 1);
        } else {
            let RunViewState::Runnable {
                position,
                reason: crate::RunnableReason::Restart { checkpoint },
            } = first.state()
            else {
                panic!("eligible checkpoint")
            };
            assert_eq!(checkpoint.index(), usize::from(before_checkpoint));
            assert_eq!(position.state, *checkpoint);
            assert_eq!(
                position.visit.value(),
                if before_checkpoint { 2 } else { 1 }
            );
            let exhausted = runtime.resume(&run, &program).await.unwrap();
            let report = exhausted
                .failure()
                .expect("restart bound survives cold restoration");
            assert_eq!(
                report.reason(),
                &StopReason::Exhausted(RecoveryLimit::StateRestart)
            );
            assert_eq!(report.usage().state_restarts, 1);
            assert_eq!(calls.lock().unwrap().len(), usize::from(before_checkpoint));
        }
    }
}
