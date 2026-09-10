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
    ) -> std::result::Result<ProposedStateOutcome<Number, Cause>, StateExecutionError> {
        Ok(ProposedStateOutcome::Failure {
            failure: Cause::Timeout { deadline_ms: 5000 },
        })
    }
}
impl EffectState<Submit> for FailureState {
    type AdapterContext = Number;
    fn adapter_context(
        input: &Number,
        _: &Number,
        _: &Cause,
    ) -> std::result::Result<Number, StateExecutionError> {
        Ok(Number { value: input.value })
    }
    fn prepare(input: &Number) -> std::result::Result<Number, PreparationError> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Cause>, StateExecutionError> {
        Self::evaluate(input)
    }
}
impl CapabilityInjection<FailureState> for Submit {
    type Setup = Number;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Cause;
    type FailureMap = Identity<Cause>;
    fn failure_map_params(_: &Number) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &Number) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct RestartDeclared;
impl Handler for RestartDeclared {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.restart-declared@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        context
            .declared_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .ok_or(StateExecutionError)
    }
}
struct PhaseOperation {
    effect: bool,
    restart: bool,
}
impl Operation for PhaseOperation {
    type Input = Number;
    type Output = Number;
    type Failure = Cause;
    fn validate_input(&self, _: &Number) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Number, Number, Cause>,
    ) -> mfm_program::Result<()> {
        let checkpoint = scope.checkpoint::<Number>()?;
        scope.handler(if self.restart {
            HandlerBinding::new::<RestartDeclared>(NoParams)?.checkpoint(&checkpoint)?
        } else {
            HandlerBinding::new::<RetryUnknown>(NoParams)?
        })?;
        scope.allowances(RecoveryAllowances::new(3, 3))?;
        if self.effect {
            scope.effect::<FailureState, Submit, Identity<Cause>>(
                &Number { value: 1 },
                NoParams,
                Occurrence::new(),
                EffectBounds::new(65536, 65536, 2, 65536)?,
            )
        } else {
            scope.pure::<FailureState, Identity<Cause>>(
                NoParams,
                Occurrence::new(),
                ConclusionBound::new(65536)?,
            )
        }
    }
}

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
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        if effect {
            builder.register_effect::<FailureState, Submit>().unwrap();
        } else {
            builder.register_pure::<FailureState>().unwrap();
        }
        builder.register_handler::<RetryUnknown>().unwrap();
        builder.register_handler::<RestartDeclared>().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(Number { value: 1 }, move |_, _, command| {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(Number {
                        value: command.value,
                    }))
                })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), store);
        let run = RunId::from_digest(DigestBytes::from_array([94 + case as u8; 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test/phase-denial@1").unwrap(),
            &PhaseOperation { effect, restart },
            &Number { value: 9 },
            ProgramLimits::new(6),
        )
        .unwrap();
        let terminal = runtime
            .start(run.clone(), program, Number { value: 9 })
            .await
            .unwrap();
        let RunViewState::Failed(report) = terminal.state() else {
            panic!("runtime denied retry")
        };
        assert_eq!(report.reason(), &StopReason::Disallowed(denial));
        assert_eq!(report.usage().run_decisions, 0);
        assert_eq!(report.usage().state_retries, 0);
        let cold = runtime.resume(&run).await.unwrap();
        assert_eq!(cold.head_digest(), terminal.head_digest());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(effect)
        );
    }
}
