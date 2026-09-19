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
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.qualified-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        id: &EffectId,
        command_ref: &ContentRef,
        command: &Number,
        evidence_ref: &ContentRef,
        evidence: &Number,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        Submit::bind_evidence(id, command_ref, command, evidence_ref, evidence)
    }
}
impl EffectState<QualifiedSubmit> for Execute {
    fn prepare(input: &Number) -> std::result::Result<Number, InvocationDiagnostic> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl EffectSelection<QualifiedSubmit> for Execute {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}
struct CountedStandard;
impl Handler for CountedStandard {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.counted-standard@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        params: &NoParams,
        classification: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        POLICY_CALLS.fetch_add(1, Ordering::SeqCst);
        StandardRecovery::handle(params, classification, context)
    }
}
#[derive(Default)]
struct ProtocolDefinition;
impl OperationDefinition for ProtocolDefinition {
    type Body = ResolvedEffect<Execute, QualifiedSubmit, Native<NonAcceptance>>;
}
impl Plan<Number> for ProtocolDefinition {
    type Config = Number;
    fn plan<'a>(&'a self, input: &'a Number) -> mfm_program::Result<(&'a Number, Self::Body)> {
        Ok((input, ResolvedEffect::new(Number { value: 1 })))
    }
}
type ProtocolFlow = Operation<ProtocolDefinition, Policy<CountedStandard, 1>>;

// Explicit evidence of nonacceptance permits retrying the same command; restoring the recorded
// decision must not run classification or policy again.
#[tokio::test]
async fn qualified_nonacceptance_retries_and_cold_history_never_reclassifies() {
    assert_eq!(
        Cause::Timeout { deadline_ms: 5000 }.classify(),
        Classification::OutcomeUnknown
    );
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resources = Resources::<ProtocolFlow, NonAcceptance>::new({
        let seen = Arc::clone(&calls);
        move |id, command_ref, command| {
            let attempt = {
                let mut seen = seen.lock().unwrap();
                seen.push((id, command_ref, command.value));
                seen.len()
            };
            Box::pin(async move {
                if attempt == 1 {
                    Err(AdapterError::Operational(
                        NonAcceptance::ServerTimeoutBeforeAcceptance { request_id: 17 },
                    ))
                } else {
                    Ok(EffectAdapterOutcome::Settled(command))
                }
            })
        }
    });
    let run = RunId::from_digest(DigestBytes::from_array([96; 32]));
    let program = compile(
        EntryPointId::new("mfm.test/protocol-classification@1").unwrap(),
        &ProtocolFlow::default(),
        &Number { value: 9 },
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let runtime = Runtime::new(store.clone());
    let pending = runtime
        .start(run.clone(), &program, &Number { value: 9 })
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
    assert_eq!(failure.1, RecoveryOutcome::Retry);
    assert!(matches!(
        failure.0.decode::<NonAcceptance>().unwrap(),
        NonAcceptance::ServerTimeoutBeforeAcceptance { request_id: 17 }
    ));
    let cold = Runtime::new(store);
    let program = load(program.canonical_bytes(), &resources).unwrap();
    let read = cold.read(&run, &program).await.unwrap();
    assert_eq!(read.head_digest(), pending.head_digest());
    assert_eq!(POLICY_CALLS.load(Ordering::SeqCst), 2);
    let settled = cold.resume(&run, &program).await.unwrap();
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    let reread = cold.resume(&run, &program).await.unwrap();
    assert_eq!(reread.head_digest(), settled.head_digest());
    assert_eq!(POLICY_CALLS.load(Ordering::SeqCst), 2);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
}
