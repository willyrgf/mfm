use mfm_capabilities::{AdapterError, EffectCapabilityContract};
use mfm_evm::{EvmOperationalError, EvmTransactionOperationalError};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_program::*;
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
};
use mfm_store::MemoryStore;
use std::sync::Arc;

struct Submission;
impl EffectCapabilityContract for Submission {
    type Command = NoParams;
    type Evidence = NoParams;
    type OperationalError = EvmTransactionOperationalError;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.transport-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &EffectId,
        _: &NoParams,
        _: &NoParams,
    ) -> std::result::Result<(), mfm_values::InvocationDiagnostic> {
        Ok(())
    }
}
struct Execute;
impl State for Execute {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.transport-execute@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Submission> for Execute {
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, mfm_values::InvocationDiagnostic> {
        Ok(NoParams)
    }
    fn interpret(
        input: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Never>, mfm_values::InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Execute> for Submission {
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
struct Plan;
impl Operation for Plan {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn validate_input(&self, _: &NoParams) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<NoParams, NoParams, Never>,
    ) -> mfm_program::Result<()> {
        scope.effect::<Execute, Submission, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}

#[tokio::test]
async fn pending_json_retains_the_committed_original_cause_and_decision() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_effect::<Execute, Submission>().unwrap();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = Arc::clone(&attempts);
    builder
        .register_effect_adapter::<Submission, _, _>(NoParams, move |_, _, _| {
            let attempt = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                if attempt == 0 {
                    Ok(EffectAdapterOutcome::Pending)
                } else {
                    Err(AdapterError::Operational(
                        EvmTransactionOperationalError::Provider {
 operation: mfm_evm::TransactionProviderOperation::Submit,
                            cause: EvmOperationalError::new(mfm_evm::EvmOperationalKind::Timeout, mfm_evm::ProviderFailure {
        method: mfm_evm::EvmRpcMethod::SendRawTransaction, stage: mfm_evm::RpcStage::Send,
        failure: mfm_evm::ProviderFailureKind::Client,
        diagnostics: serde_json::from_str(r#"{"response":null,"sources":{"layers":[],"end":"unavailable"},"omissions":[],"omissions_truncated":false}"#).unwrap(),
    }),
                        },
                    ))
                }
            })
        })
        .unwrap();
    let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
    let run = RunId::from_digest(DigestBytes::from_array([99; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/pending-json@1").unwrap(),
        &Plan,
        &NoParams,
        ProgramLimits::new(0),
    )
    .unwrap();
    let pending = runtime.start(run.clone(), program, NoParams).await.unwrap();
    let before =
        serde_json::to_value(mfm_app::SerializableRunView::new(&pending).unwrap()).unwrap();
    assert_eq!(before["state"]["kind"], "effect_pending");
    assert_eq!(before["state"]["latest_failure"], serde_json::Value::Null);
    let error = runtime.resume(&run).await.err().unwrap();
    let InvocationFailure::RecoveryStopped { observed, .. } = &error else {
        panic!("audited stop")
    };
    let model = serde_json::to_value(mfm_app::SerializableRunView::new(observed).unwrap()).unwrap();
    assert_eq!(model["head_sequence"], 4);
    assert_eq!(
        model["state"]["latest_failure"]["error"]["value"],
        serde_json::json!({"kind":"provider","operation":"submit","cause":{
            "kind":"timeout","source":{
                "method":"send_raw_transaction","stage":"send","failure":{"kind":"client"},
                "diagnostics":{"response":null,"sources":{"layers":[],"end":"unavailable"},
                    "omissions":[],"omissions_truncated":false}
            }
        }})
    );
    assert_eq!(
        model["state"]["latest_failure"]["decision"],
        serde_json::json!({"stop":{"reason":"requested"}})
    );
    let RunViewState::EffectPending {
        latest_failure: Some(failure),
        effect,
    } = observed.state()
    else {
        panic!("original cause")
    };
    assert_eq!(
        model["state"]["latest_failure"]["error"]["value_ref"],
        serde_json::to_value(failure.0.value_ref()).unwrap()
    );
    assert_eq!(
        model["state"]["latest_failure"]["input"]["value_ref"],
        serde_json::to_value(effect.call().input().value_ref()).unwrap()
    );
    assert_eq!(model["state"]["latest_failure"]["mode"], "effect");
    assert_eq!(
        model["state"]["latest_failure"]["input"]["value"],
        serde_json::Value::Null
    );
    assert_eq!(
        model["state"]["latest_failure"]["command"]["value"],
        serde_json::Value::Null
    );
    assert_eq!(
        model["state"]["latest_failure"]["effect_id"],
        model["state"]["effect_id"]
    );
    assert!(model["state"]["latest_failure"]
        .get("state_context")
        .is_none());
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(
        serde_json::to_value(mfm_app::SerializableRunView::new(&cold).unwrap()).unwrap(),
        model
    );
    let error = mfm_app::RunRequestError::Invocation(error);
    let envelope = serde_json::to_value(
        mfm_app::SerializableClientError::for_run(&error, &error.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(envelope["invocation"]["observed"], model);
    assert_eq!(
        envelope["invocation"]["error"],
        model["state"]["latest_failure"]["error"]
    );
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}
