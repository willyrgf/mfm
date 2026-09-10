use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static POLICY_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum NonAcceptance {
    ServerTimeoutBeforeAcceptance { request_id: u64 },
}
impl ClassifyError for NonAcceptance {
    fn classify(&self) -> Classification {
        POLICY_CALLS.fetch_add(1, Ordering::SeqCst);
        match self {
            Self::ServerTimeoutBeforeAcceptance { .. } => Classification::Retryable,
        }
    }
}
struct QualifiedSubmit;
impl EffectCapabilityContract for QualifiedSubmit {
    type Command = Number;
    type Evidence = Number;
    type OperationalError = NonAcceptance;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.qualified-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        id: &EffectId,
        command: &Number,
        evidence: &Number,
    ) -> mfm_capabilities::Result<()> {
        Submit::bind_evidence(id, command, evidence)
    }
}
impl EffectState<QualifiedSubmit> for Execute {
    type AdapterContext = Number;
    fn adapter_context(
        input: &Number,
        _: &Number,
        _: &NonAcceptance,
    ) -> std::result::Result<Number, StateExecutionError> {
        Ok(Number { value: input.value })
    }
    fn prepare(input: &Number) -> std::result::Result<Number, PreparationError> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Execute> for QualifiedSubmit {
    type Setup = Number;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &Number) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &Number) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct CountedStandard;
impl Handler for CountedStandard {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.counted-standard@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        params: &NoParams,
        summary: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        POLICY_CALLS.fetch_add(1, Ordering::SeqCst);
        StandardRecovery::handle(params, summary, context)
    }
}
struct ProtocolFlow;
impl Operation for ProtocolFlow {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn validate_input(&self, _: &Number) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Number, Number, Never>,
    ) -> mfm_program::Result<()> {
        scope.handler(HandlerBinding::new::<CountedStandard>(NoParams)?)?;
        scope.allowances(RecoveryAllowances::new(1, 0))?;
        scope.effect::<Execute, QualifiedSubmit, Identity<Never>>(
            &Number { value: 1 },
            NoParams,
            Occurrence::new(),
            EffectBounds::new(65536, 65536, 2, 65536)?,
        )
    }
}

#[tokio::test]
async fn qualified_nonacceptance_retries_and_cold_history_never_reclassifies() {
    assert_eq!(
        Cause::Timeout { deadline_ms: 5000 }.classify(),
        Classification::OutcomeUnknown
    );
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_effect::<Execute, QualifiedSubmit>()
            .unwrap();
        builder.register_handler::<CountedStandard>().unwrap();
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<QualifiedSubmit, _, _>(
                Number { value: 1 },
                move |id, command_ref, command| {
                    let attempt = {
                        let mut seen = seen.lock().unwrap();
                        seen.push((id.clone(), command_ref.clone(), command.value));
                        seen.len()
                    };
                    Box::pin(async move {
                        if attempt == 1 {
                            Err(AdapterError::Operational(
                                NonAcceptance::ServerTimeoutBeforeAcceptance { request_id: 17 },
                            ))
                        } else {
                            Ok(EffectAdapterOutcome::Settled(Number {
                                value: command.value,
                            }))
                        }
                    })
                },
            )
            .unwrap();
        builder.finish()
    };
    let run = RunId::from_digest(DigestBytes::from_array([96; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/protocol-classification@1").unwrap(),
        &ProtocolFlow,
        &Number { value: 9 },
        ProgramLimits::new(1),
    )
    .unwrap();
    let runtime = Runtime::new(build(), store.clone());
    let pending = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .unwrap();
    assert_eq!(POLICY_CALLS.load(Ordering::SeqCst), 2);
    let RunViewState::EffectPending {
        latest_failure: Some(failure),
        ..
    } = pending.state()
    else {
        panic!("audited retry")
    };
    assert_eq!(failure.decision, PendingDecision::Retry);
    assert!(matches!(
        failure.incident.error.decode::<NonAcceptance>().unwrap(),
        NonAcceptance::ServerTimeoutBeforeAcceptance { request_id: 17 }
    ));
    let cold = Runtime::new(build(), store);
    let read = cold.read(&run).await.unwrap();
    assert_eq!(read.head_digest(), pending.head_digest());
    assert_eq!(POLICY_CALLS.load(Ordering::SeqCst), 2);
    let settled = cold.resume(&run).await.unwrap();
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    let reread = cold.resume(&run).await.unwrap();
    assert_eq!(reread.head_digest(), settled.head_digest());
    assert_eq!(POLICY_CALLS.load(Ordering::SeqCst), 2);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
}
