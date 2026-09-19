use super::*;

struct FailureState;
impl State for FailureState {
    type Input = Number;
    type Output = Number;
    type Failure = Cause;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.phase-failure@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for FailureState {
    fn evaluate(
        _: Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Cause>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure {
            failure: Cause::Timeout { deadline_ms: 5000 },
        })
    }
}
impl EffectState<Submit> for FailureState {
    fn prepare(input: &Number) -> std::result::Result<Number, InvocationDiagnostic> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Cause>, InvocationDiagnostic> {
        Self::evaluate(input)
    }
}
impl EffectSelection<Submit> for FailureState {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}
struct RestartDeclared;
impl Handler for RestartDeclared {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.restart-declared@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        context
            .declared_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .ok_or_else(|| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "handle",
                    &(ProgramError::InvalidContract),
                    None,
                )
            })
    }
}
struct BeforeFailure;
impl CheckpointMarker for BeforeFailure {
    type Context = Number;
}
struct PhaseDefinition {
    effect: bool,
}
impl OperationDefinition for PhaseDefinition {
    type Body = (
        Checkpoint<BeforeFailure>,
        Vec<Pure<FailureState>>,
        Vec<ResolvedEffect<FailureState, Submit, Native<Cause>>>,
    );
}
impl Plan<Number> for PhaseDefinition {
    type Config = Number;
    fn plan<'a>(&'a self, input: &'a Number) -> mfm_program::Result<(&'a Number, Self::Body)> {
        let (pure, effect) = if self.effect {
            (vec![], vec![ResolvedEffect::new(Number { value: 1 })])
        } else {
            (vec![Pure::default()], vec![])
        };
        Ok((input, (Checkpoint::default(), pure, effect)))
    }
}
struct RestartPolicy;
impl OperationDefaults for RestartPolicy {
    type Handler = RestartDeclared;
    type Targets = (BeforeFailure,);
}
impl ResolveDefaults<Number> for RestartPolicy {
    fn resolve(_: &Number) -> mfm_program::Result<PolicyValues<RestartDeclared>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(3),
            restarts: Some(3),
        })
    }
}
type RetryPhase = Operation<PhaseDefinition, Policy<RetryUnknown, 3>>;
type RestartPhase = Operation<PhaseDefinition, RestartPolicy>;

// A handler request cannot override Runtime safety rules or spend recovery allowance on a denied
// action.
#[tokio::test]
async fn runtime_denies_pure_retry_ineligible_restart_and_settled_effect_retry() {
    for (case, (effect, restart, denial)) in [
        (false, false, RecoveryDenial::PureRetry),
        (true, false, RecoveryDenial::EffectSettled),
        (false, true, RecoveryDenial::CheckpointUnavailable),
    ]
    .into_iter()
    .enumerate()
    {
        let store = Arc::new(MemoryStore::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let resources = Resources::<(RetryPhase, RestartPhase)>::new(move |_, _, command| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move { Ok(EffectAdapterOutcome::Settled(command)) })
        });
        let runtime = Runtime::new(store);
        let run = RunId::from_digest(DigestBytes::from_array([94 + case as u8; 32]));
        let entry = EntryPointId::new("mfm.test/phase-denial@1").unwrap();
        let program = if restart {
            compile(
                entry,
                &RestartPhase::from(PhaseDefinition { effect }),
                &Number { value: 9 },
                &resources,
                ProgramLimits::new(6),
            )
        } else {
            compile(
                entry,
                &RetryPhase::from(PhaseDefinition { effect }),
                &Number { value: 9 },
                &resources,
                ProgramLimits::new(6),
            )
        }
        .unwrap();
        let terminal = runtime
            .start(run.clone(), &program, &Number { value: 9 })
            .await
            .unwrap();
        let RunViewState::Failed(report) = terminal.state() else {
            panic!("runtime denied retry")
        };
        assert_eq!(report.reason(), &StopReason::Disallowed(denial));
        assert_eq!(report.usage().run_decisions, 0);
        assert_eq!(report.usage().state_retries, 0);
        let cold = runtime.resume(&run, &program).await.unwrap();
        assert_eq!(cold.head_digest(), terminal.head_digest());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(effect)
        );
    }
}
