use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::EffectAdapterOutcome;
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    compile, load, EffectSelection, EffectState, Operation, OperationDefinition, Plan,
    ProgramError, ProgramLimits, ProposedStateOutcome, Pure, PureState, ReadSelection, ReadState,
    ResolvedEffect, ResolvedRead, State,
};
use mfm_runtime::{InvocationFailure, RunViewState, Runtime, RuntimeError};
use mfm_store::MemoryStore;
use mfm_values::{canonicalize_mfm_value, InvocationDiagnostic};
use serde::Serialize;

use super::program::*;
use super::resources::{Native, Resources};

#[derive(Debug, Serialize, thiserror::Error)]
#[error("test callback execution failed")]
struct ExecutionFault {
    input: u64,
}

static FAIL_EXECUTION: AtomicBool = AtomicBool::new(true);

struct FailingPure;
struct FailingRead;
struct FailingEffect;

macro_rules! error_state {
    ($state:ident, $id:literal) => {
        impl State for $state {
            type Input = Number;
            type Output = Number;
            type Failure = Number;
            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| ProgramError::InvalidContract)
            }
        }
    };
}
error_state!(FailingPure, "mfm.test.runtime/failing-pure@1");
error_state!(FailingRead, "mfm.test.runtime/failing-read@1");
error_state!(FailingEffect, "mfm.test.runtime/failing-effect@1");

fn execution(input: Number) -> Result<ProposedStateOutcome<Number, Number>, InvocationDiagnostic> {
    if FAIL_EXECUTION.load(Ordering::SeqCst) {
        Err(InvocationDiagnostic::from_fields(
            "state_internal",
            "execution",
            &(ExecutionFault { input: input.value }),
            None,
        ))
    } else {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

impl PureState for FailingPure {
    fn evaluate(
        input: Number,
    ) -> Result<ProposedStateOutcome<Number, Number>, InvocationDiagnostic> {
        execution(input)
    }
}
impl ReadState<Observation> for FailingRead {
    fn prepare(input: &Number) -> Result<Intent, InvocationDiagnostic> {
        Ok(Intent { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Evidence,
    ) -> Result<ProposedStateOutcome<Number, Number>, InvocationDiagnostic> {
        execution(input)
    }
}
impl EffectState<Mutation> for FailingEffect {
    fn prepare(input: &Number) -> Result<Command, InvocationDiagnostic> {
        Ok(Command { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &EffectEvidence,
    ) -> Result<ProposedStateOutcome<Number, Number>, InvocationDiagnostic> {
        execution(input)
    }
}
impl ReadSelection<Observation> for FailingRead {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}

impl EffectSelection<Mutation> for FailingEffect {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}

enum ErrorProgram {
    Pure,
    Read,
    Effect,
}
type Source = Operation<ErrorProgram>;
impl OperationDefinition for ErrorProgram {
    type Body = (
        Vec<Pure<FailingPure>>,
        Vec<ResolvedRead<FailingRead, Observation, Native>>,
        Vec<ResolvedEffect<FailingEffect, Mutation, Native>>,
    );
}
impl Plan<Number> for ErrorProgram {
    type Config = Number;
    fn plan<'a>(&'a self, input: &'a Number) -> mfm_program::Result<(&'a Number, Self::Body)> {
        let body = match self {
            Self::Pure => (vec![Pure::default()], vec![], vec![]),
            Self::Read => (
                vec![],
                vec![ResolvedRead::new(Binding { route: 7 })],
                vec![],
            ),
            Self::Effect => (
                vec![],
                vec![],
                vec![ResolvedEffect::new(Binding { route: 8 })],
            ),
        };
        Ok((input, body))
    }
}

// Callback failures must leave recoverable history; retrying interpretation of an accepted
// Effect must not repeat the external action.
#[tokio::test]
async fn internal_callback_errors_preserve_heads_and_do_not_repeat_settled_effects() {
    let store = Arc::new(MemoryStore::new());
    let identities = Arc::new(std::sync::Mutex::new(Vec::new()));
    let read_calls = Arc::new(AtomicUsize::new(0));
    let build_runtime = || {
        let mut resources = Resources::<Source>::read({
            let calls = read_calls.clone();
            move |reference, intent| {
                calls.fetch_add(1, Ordering::SeqCst);
                let evidence = Evidence {
                    intent_value_ref: reference.clone(),
                    value: intent.value,
                    accepted: true,
                };
                Box::pin(async move { Ok(evidence) })
            }
        });
        resources.effect = Some(Arc::new({
            let identities = identities.clone();
            move |effect_id, reference, command| {
                identities.lock().unwrap().push((
                    effect_id.clone(),
                    reference.clone(),
                    canonicalize_mfm_value(command).unwrap().0,
                ));
                let evidence = EffectEvidence {
                    effect_id: effect_id.clone(),
                    value: command.value,
                    accepted: true,
                };
                Box::pin(async move { Ok(EffectAdapterOutcome::Settled(evidence)) })
            }
        }));
        (Runtime::new(store.clone()), resources)
    };
    let (runtime, resources) = build_runtime();
    for (index, operation, expected_head, expected_operation) in [
        (70, ErrorProgram::Pure, 1, "pure_evaluate"),
        (71, ErrorProgram::Read, 1, "read_interpret"),
        (72, ErrorProgram::Effect, 3, "effect_interpret"),
    ] {
        FAIL_EXECUTION.store(true, Ordering::SeqCst);
        let id = RunId::from_digest(DigestBytes::from_array([index; 32]));
        let program = compile(
            EntryPointId::new("mfm.test.runtime/callback-error@1").unwrap(),
            &Source::from(operation),
            &Number { value: 12 },
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        assert_execution_failure(
            runtime
                .start(id.clone(), &program, &Number { value: 12 })
                .await
                .err()
                .unwrap(),
            expected_operation,
        );
        let pending = runtime.read(&id, &program).await.unwrap();
        assert_eq!(pending.head_sequence(), expected_head);
        if expected_head == 3 {
            assert!(matches!(
                pending.state(),
                RunViewState::AwaitingInterpretation { .. }
            ));
        } else {
            assert!(matches!(pending.state(), RunViewState::Runnable { .. }));
        }
        let (cold, cold_resources) = build_runtime();
        let program = load(program.canonical_bytes(), &cold_resources).unwrap();
        assert_execution_failure(
            cold.resume(&id, &program).await.err().unwrap(),
            expected_operation,
        );
        assert_eq!(
            cold.read(&id, &program).await.unwrap().head_digest(),
            pending.head_digest()
        );
        FAIL_EXECUTION.store(false, Ordering::SeqCst);
        let completed = cold.resume(&id, &program).await.unwrap();
        assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        assert_eq!(completed.head_sequence(), expected_head + 1);
        FAIL_EXECUTION.store(true, Ordering::SeqCst);
        let (terminal, terminal_resources) = build_runtime();
        let program = load(program.canonical_bytes(), &terminal_resources).unwrap();
        let reloaded = terminal.read(&id, &program).await.unwrap();
        assert_eq!(reloaded.head_digest(), completed.head_digest());
        let resumed = terminal.resume(&id, &program).await.unwrap();
        assert_eq!(resumed.head_digest(), completed.head_digest());
        let (RunViewState::Succeeded(actual), RunViewState::Succeeded(expected)) =
            (resumed.state(), completed.state())
        else {
            panic!("terminal success")
        };
        assert_eq!(actual.canonical_bytes(), expected.canonical_bytes());
        assert_eq!(actual.value_ref(), expected.value_ref());
    }
    assert_eq!(read_calls.load(Ordering::SeqCst), 3);
    let retained = identities.lock().unwrap();
    assert_eq!(
        retained.len(),
        1,
        "accepted settlement is never reconciled again"
    );
}

fn assert_execution_failure(failure: InvocationFailure, expected_operation: &str) {
    let InvocationFailure::Execution {
        error:
            RuntimeError::Native {
                operation,
                stage: mfm_runtime::Stage::Execute,
                cause,
            },
        ..
    } = failure
    else {
        panic!("typed callback failure")
    };
    assert_eq!(serde_json::to_value(operation).unwrap(), expected_operation);
    assert_eq!(cause.details().as_value()["input"], 12);
    let projected = cause.details().as_value();
    assert_eq!(projected["input"], 12);
}
