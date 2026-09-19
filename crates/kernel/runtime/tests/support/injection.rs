use super::program::*;
use super::resources::{EffectSource, Native, ReadSource, Resources};
use mfm_capabilities::EffectAdapterOutcome;
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    compile, load, EffectSelection, EffectState, InjectEffect, Never, ProgramError, ProgramLimits,
    ProposedStateOutcome, Pure, PureState, ResolvedEffect, State,
};
use mfm_runtime::{RunViewState, Runtime};
use mfm_store::MemoryStore;
use mfm_values::InvocationDiagnostic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
struct Injected;
impl State for Injected {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.injection/main@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Mutation> for Injected {
    fn prepare(input: &Number) -> Result<Command, InvocationDiagnostic> {
        Ok(Command { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &EffectEvidence,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
struct Project;
impl State for Project {
    type Input = Number;
    type Output = Number;
    type Failure = Number;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.injection/project@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Project {
    fn evaluate(
        input: Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Number>, InvocationDiagnostic> {
        if input.value == 2 {
            Ok(ProposedStateOutcome::Failure { failure: input })
        } else {
            Ok(ProposedStateOutcome::Success { output: input })
        }
    }
}

struct Surround;
impl super::resources::NativeIdentity for Surround {
    const ID: &'static str = "mfm.test.runtime/native-surround@1";
}
type Source = ResolvedEffect<Injected, Mutation, Native<Surround>>;
impl EffectSelection<Mutation> for Injected {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}
impl InjectEffect<Injected, Mutation> for Native<Surround> {
    type Prefix = (Pure<Increment>, ReadSource, EffectSource);
    type Suffix = Pure<Project>;
    fn surround(_: &Binding) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            (
                Pure::default(),
                ReadSource::new(Binding { route: 7 }),
                EffectSource::new(Binding { route: 8 }),
            ),
            Pure::default(),
        ))
    }
}

// Native injection produces one ordinary immutable sequence. A cold resume retains the pending
// command and the suffix State's exact original, without a Runtime failure mapper.
#[tokio::test]
async fn injected_effects_resume_and_suffix_failure_retains_its_exact_original() {
    let store = Arc::new(MemoryStore::new());
    let pending = Arc::new(AtomicBool::new(true));
    let mut resources = Resources::<Source>::read(|reference, intent| {
        let evidence = Evidence {
            intent_value_ref: reference.clone(),
            value: intent.value,
            accepted: true,
        };
        Box::pin(async move { Ok(evidence) })
    });
    resources.effect = Some(Arc::new(move |id, _, command| {
        let id = id.clone();
        let value = command.value;
        let pending = pending.swap(false, Ordering::SeqCst);
        Box::pin(async move {
            Ok(if pending {
                EffectAdapterOutcome::Pending
            } else {
                EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id: id,
                    value,
                    accepted: true,
                })
            })
        })
    }));
    let runtime = Runtime::new(store.clone());
    let run_id = RunId::from_digest(DigestBytes::from_array([197; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.injection/program@1").unwrap(),
        &Source::new(Binding { route: 8 }),
        &Number { value: 1 },
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_eq!(program.declarations().len(), 5);
    let first = runtime
        .start(run_id.clone(), &program, &Number { value: 1 })
        .await
        .unwrap();
    assert!(matches!(first.state(), RunViewState::EffectPending { .. }));
    let bytes = program.canonical_bytes().to_vec();
    drop(program);
    drop(runtime);
    let cold = load(&bytes, &resources).unwrap();
    let runtime = Runtime::new(store);
    let finished = runtime.resume(&run_id, &cold).await.unwrap();
    let report = finished.failure().expect("suffix domain failure");
    assert_eq!(
        report
            .failure()
            .original()
            .decode::<Number>()
            .unwrap()
            .value,
        2
    );
    assert_eq!(
        report.declaration().state_implementation_ref(),
        &mfm_program::state_implementation_ref::<Project>().unwrap()
    );
    assert_eq!(finished.head_sequence(), 11);
    assert_eq!(
        runtime.read(&run_id, &cold).await.unwrap().head_digest(),
        finished.head_digest()
    );
}
