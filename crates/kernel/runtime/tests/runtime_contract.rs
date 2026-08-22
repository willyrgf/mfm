use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilityError, EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_journal::{EncodedRunFrame, StoredRunBytes};
use mfm_program::{
    expand_program, CapabilityInjection, EffectState, Never, Operation, OperationExpansion,
    PreparationError, ProgramError, ProposedStateOutcome, PureState, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{AdapterError, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};
use mfm_values::canonicalize_mfm_value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Number {
    value: u64,
}

struct Increment;

impl State for Increment {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Increment {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: Number {
                value: input.value + 1,
            },
        }
    }
}

struct PureProgram;

impl Operation for PureProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment>()
    }
}

struct EmptyProgram;

impl Operation for EmptyProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Intent {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Evidence {
    value: u64,
    accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Binding {
    route: u64,
}

struct Observation;

impl ReadCapabilityContract for Observation {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct Observe;

impl State for Observe {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<Observation> for Observe {
    fn prepare(input: &Self::Input) -> Result<Intent, PreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        if evidence.accepted {
            ProposedStateOutcome::Success { output: input }
        } else {
            ProposedStateOutcome::Failure { failure: input }
        }
    }
}

impl CapabilityInjection<Observe> for Observation {
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct ReadProgram;

impl Operation for ReadProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<Observe, Observation>(&Binding { route: 7 })
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Command {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EffectEvidence {
    effect_id: EffectId,
    value: u64,
    accepted: bool,
}

struct Mutation;

impl EffectCapabilityContract for Mutation {
    type Command = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (effect_id == &evidence.effect_id && command.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct ConflictingReadCapability;

impl ReadCapabilityContract for ConflictingReadCapability {
    type Intent = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct Mutate;

impl State for Mutate {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/mutate@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl EffectState<Mutation> for Mutate {
    fn prepare(input: &Self::Input) -> Result<Command, PreparationError> {
        Ok(Command { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &EffectEvidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        if evidence.accepted {
            ProposedStateOutcome::Success { output: input }
        } else {
            ProposedStateOutcome::Failure { failure: input }
        }
    }
}

impl CapabilityInjection<Mutate> for Mutation {
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct EffectProgram;

impl Operation for EffectProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<Mutate, Mutation>(&Binding { route: 8 })
    }
}

struct RejectEffect;

impl State for RejectEffect {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/reject-effect@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl EffectState<Mutation> for RejectEffect {
    fn prepare(_input: &Self::Input) -> Result<Command, PreparationError> {
        Err(PreparationError)
    }

    fn interpret(
        input: Self::Input,
        _evidence: &EffectEvidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

impl CapabilityInjection<RejectEffect> for Mutation {
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct RejectEffectProgram;

impl Operation for RejectEffectProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<RejectEffect, Mutation>(&Binding { route: 8 })
    }
}

#[test]
fn effect_registration_rejects_duplicate_and_wrong_kind_capability_entries() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |effect_id, command| {
            let effect_id = effect_id.clone();
            let value = command.value;
            Box::pin(async move {
                Ok(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                })
            })
        })
        .expect("Effect adapter");
    assert_eq!(
        builder.register_effect_adapter::<Mutation, _, _>(
            Binding { route: 8 },
            |effect_id, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            },
        ),
        Err(RuntimeError::IncompatibleAssembly)
    );
    assert_eq!(
        builder
            .register_adapter::<ConflictingReadCapability, _, _>(Binding { route: 8 }, |_intent| {
                Box::pin(async { Err(AdapterError::Internal) })
            },),
        Err(RuntimeError::IncompatibleAssembly)
    );
}

#[tokio::test]
async fn missing_effect_adapter_is_rejected_before_store_io() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(MemoryStore::new()),
    );
    let run_id = RunId::from_digest(DigestBytes::from_array([29; 32]));
    assert!(matches!(
        runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/missing-effect-adapter@1")
                        .expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 1 },
            )
            .await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        runtime.read(&run_id).await,
        Err(RuntimeError::Absent)
    ));
}

#[tokio::test]
async fn pure_and_zero_state_programs_are_identical_hot_and_cold() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("Pure State");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([1; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/pure@1").expect("entry point"),
        &PureProgram,
    )
    .expect("Program");

    let hot = runtime
        .start(run_id.clone(), program, Number { value: 4 })
        .await
        .expect("start");
    let RunViewState::Succeeded(hot_value) = hot.state() else {
        panic!("Pure Program did not succeed");
    };
    assert_eq!(hot.head_sequence(), 2);
    assert_eq!(hot_value.canonical_bytes(), br#"{"value":5}"#);

    let cold = runtime.read(&run_id).await.expect("cold read");
    let RunViewState::Succeeded(cold_value) = cold.state() else {
        panic!("cold Program did not succeed");
    };
    assert_eq!(cold.head_sequence(), hot.head_sequence());
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(cold_value.canonical_bytes(), hot_value.canonical_bytes());

    let mut empty_builder = RuntimeAssemblyBuilder::new();
    empty_builder
        .register_value::<Number>()
        .expect("root value");
    let empty = Runtime::new(
        empty_builder.finish().expect("empty assembly"),
        Arc::new(MemoryStore::new()),
    );
    let empty_program = expand_program(
        EntryPointId::new("mfm.test.runtime/empty@1").expect("entry point"),
        &EmptyProgram,
    )
    .expect("empty Program");
    let empty_view = empty
        .start(
            RunId::from_digest(DigestBytes::from_array([2; 32])),
            empty_program,
            Number { value: 9 },
        )
        .await
        .expect("empty start");
    let RunViewState::Succeeded(value) = empty_view.state() else {
        panic!("zero-State Program did not succeed");
    };
    assert_eq!(empty_view.head_sequence(), 1);
    assert_eq!(value.canonical_bytes(), br#"{"value":9}"#);
}

#[tokio::test]
async fn a_fused_read_is_replayable_and_a_failed_observation_is_resumable() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, {
            let calls = Arc::clone(&calls);
            move |intent| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(Evidence {
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([3; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadProgram,
    )
    .expect("Program");
    let hot = runtime
        .start(run_id.clone(), program, Number { value: 12 })
        .await
        .expect("Read execution");
    assert!(matches!(hot.state(), RunViewState::Succeeded(_)));
    assert_eq!(hot.head_sequence(), 2);
    let cold = runtime.read(&run_id).await.expect("cold read");
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let unavailable_store = Arc::new(MemoryStore::new());
    let mut unavailable_builder = RuntimeAssemblyBuilder::new();
    unavailable_builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    unavailable_builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, |_| {
            Box::pin(async { Err(AdapterError::Unavailable) })
        })
        .expect("unavailable adapter");
    let unavailable = Runtime::new(
        unavailable_builder.finish().expect("assembly"),
        unavailable_store.clone(),
    );
    let interrupted_run_id = RunId::from_digest(DigestBytes::from_array([4; 32]));
    let interrupted_program = expand_program(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadProgram,
    )
    .expect("Program");
    assert!(matches!(
        unavailable
            .start(
                interrupted_run_id.clone(),
                interrupted_program,
                Number { value: 21 },
            )
            .await,
        Err(RuntimeError::Unavailable)
    ));
    let prefix = unavailable
        .read(&interrupted_run_id)
        .await
        .expect("durable prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable));

    let mut resumed_builder = RuntimeAssemblyBuilder::new();
    resumed_builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    resumed_builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, |intent| {
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: false,
                })
            })
        })
        .expect("replacement adapter");
    let resumed = Runtime::new(
        resumed_builder.finish().expect("assembly"),
        unavailable_store,
    )
    .resume(&interrupted_run_id)
    .await
    .expect("resume");
    let RunViewState::Failed(value) = resumed.state() else {
        panic!("rejected observation did not follow the Program failure path");
    };
    assert_eq!(resumed.head_sequence(), 2);
    assert_eq!(value.canonical_bytes(), br#"{"value":21}"#);
}

#[tokio::test]
async fn cancellation_during_observation_preserves_a_runnable_prefix() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            move |intent| {
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(Evidence {
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Arc::new(Runtime::new(builder.finish().expect("assembly"), store));
    let run_id = RunId::from_digest(DigestBytes::from_array([5; 32]));
    let task = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move {
            runtime
                .start(
                    run_id,
                    expand_program(
                        EntryPointId::new("mfm.test.runtime/cancel@1").expect("entry point"),
                        &ReadProgram,
                    )
                    .expect("Program"),
                    Number { value: 8 },
                )
                .await
        })
    };
    entered.notified().await;
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("observation task was not cancelled"),
    }
    release.notify_waiters();

    let prefix = runtime.read(&run_id).await.expect("durable prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable));
}

#[tokio::test]
async fn effect_prepare_is_durable_before_adapter_entry_and_cold_resume_reuses_identity() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let observed = Arc::clone(&observed);
            move |effect_id, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                observed
                    .lock()
                    .expect("observations")
                    .push((effect_id.clone(), value));
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("Effect adapter");
    let runtime = Arc::new(Runtime::new(
        builder.finish().expect("assembly"),
        store.clone(),
    ));
    let run_id = RunId::from_digest(DigestBytes::from_array([30; 32]));
    let task = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move {
            runtime
                .start(
                    run_id,
                    expand_program(
                        EntryPointId::new("mfm.test.runtime/effect@1").expect("entry point"),
                        &EffectProgram,
                    )
                    .expect("Program"),
                    Number { value: 34 },
                )
                .await
        })
    };
    entered.notified().await;

    let pending = runtime.read(&run_id).await.expect("pending view");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(pending.state(), RunViewState::Runnable));
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("Effect task was not cancelled"),
    }
    release.notify_waiters();
    let first_effect = observed.lock().expect("observations")[0].0.clone();

    let resumed_observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut resumed_builder = RuntimeAssemblyBuilder::new();
    resumed_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    resumed_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let resumed_observed = Arc::clone(&resumed_observed);
            move |effect_id, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                resumed_observed
                    .lock()
                    .expect("resumed observations")
                    .push((effect_id.clone(), value));
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("Effect adapter");
    let resumed_runtime = Runtime::new(resumed_builder.finish().expect("assembly"), store);
    let resumed = resumed_runtime.resume(&run_id).await.expect("cold resume");
    assert_eq!(resumed.head_sequence(), 3);
    assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
    {
        let resumed_calls = resumed_observed.lock().expect("resumed observations");
        assert_eq!(resumed_calls.as_slice(), &[(first_effect, 34)]);
    }

    let cold = resumed_runtime.read(&run_id).await.expect("cold view");
    assert_eq!(cold.head_digest(), resumed.head_digest());
    assert_eq!(
        resumed_observed.lock().expect("resumed observations").len(),
        1
    );
}

#[tokio::test]
async fn effect_preparation_and_evidence_failures_append_no_conclusion() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_effect::<RejectEffect, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let calls = Arc::clone(&calls);
            move |effect_id, command| {
                calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([31; 32]));
    assert!(matches!(
        runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/reject-effect@1").expect("entry point"),
                    &RejectEffectProgram,
                )
                .expect("Program"),
                Number { value: 1 },
            )
            .await,
        Err(RuntimeError::Internal)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runtime
            .read(&run_id)
            .await
            .expect("genesis")
            .head_sequence(),
        1
    );

    let invalid_store = Arc::new(MemoryStore::new());
    let mut invalid_builder = RuntimeAssemblyBuilder::new();
    invalid_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    invalid_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |_effect_id, command| {
            let value = command.value;
            Box::pin(async move {
                Ok(EffectEvidence {
                    effect_id: EffectId::from_digest(DigestBytes::from_array([99; 32])),
                    value,
                    accepted: true,
                })
            })
        })
        .expect("adapter");
    let invalid = Runtime::new(invalid_builder.finish().expect("assembly"), invalid_store);
    let invalid_run_id = RunId::from_digest(DigestBytes::from_array([32; 32]));
    assert!(matches!(
        invalid
            .start(
                invalid_run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/invalid-evidence@1").expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 2 },
            )
            .await,
        Err(RuntimeError::Internal)
    ));
    assert_eq!(
        invalid
            .read(&invalid_run_id)
            .await
            .expect("pending")
            .head_sequence(),
        2
    );
}

struct SequenceIndeterminateStore {
    inner: MemoryStore,
    sequence: u64,
    commit_before_error: bool,
    fired: std::sync::atomic::AtomicBool,
}

struct SequenceNotInsertedStore {
    inner: MemoryStore,
    sequence: u64,
    fired: std::sync::atomic::AtomicBool,
}

impl SequenceNotInsertedStore {
    fn new(sequence: u64) -> Self {
        Self {
            inner: MemoryStore::new(),
            sequence,
            fired: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Store for SequenceNotInsertedStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            if frame.run_sequence() == self.sequence && !self.fired.swap(true, Ordering::SeqCst) {
                let _ = self.inner.append_run(frame).await?;
                return Ok(AppendResult::NotInserted);
            }
            self.inner.append_run(frame).await
        })
    }
}

struct RecordingStore {
    inner: MemoryStore,
    frames: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl RecordingStore {
    fn new() -> Self {
        Self {
            inner: MemoryStore::new(),
            frames: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        self.frames.lock().expect("recorded frames").clone()
    }
}

impl Store for RecordingStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.inner.append_run(frame).await?;
            if result == AppendResult::Inserted {
                self.frames
                    .lock()
                    .expect("recorded frames")
                    .push(frame.canonical_bytes().to_vec());
            }
            Ok(result)
        })
    }
}

struct RetainedStore {
    frames: Vec<Vec<u8>>,
}

impl Store for RetainedStore {
    fn load_run<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            Ok(Some(
                StoredRunBytes::new(self.frames.clone())
                    .map_err(|_| StoreError::CorruptPhysicalState)?,
            ))
        })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async { Err(StoreError::CorruptPhysicalState) })
    }
}

impl SequenceIndeterminateStore {
    fn new(sequence: u64, commit_before_error: bool) -> Self {
        Self {
            inner: MemoryStore::new(),
            sequence,
            commit_before_error,
            fired: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Store for SequenceIndeterminateStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            if frame.run_sequence() == self.sequence && !self.fired.swap(true, Ordering::SeqCst) {
                if self.commit_before_error {
                    self.inner.append_run(frame).await?;
                }
                return Err(StoreError::Indeterminate);
            }
            self.inner.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn ambiguous_effect_appends_recover_from_exact_retained_facts() {
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let prepare_store = Arc::new(SequenceIndeterminateStore::new(2, false));
    let mut prepare_builder = RuntimeAssemblyBuilder::new();
    prepare_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    prepare_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let prepare_calls = Arc::clone(&prepare_calls);
            move |effect_id, command| {
                prepare_calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let prepare_runtime = Runtime::new(prepare_builder.finish().expect("assembly"), prepare_store);
    let prepare_run = RunId::from_digest(DigestBytes::from_array([33; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/prepare-indeterminate@1").expect("entry point"),
        &EffectProgram,
    )
    .expect("Program");
    assert!(matches!(
        prepare_runtime
            .start(prepare_run.clone(), program, Number { value: 3 })
            .await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        prepare_runtime
            .read(&prepare_run)
            .await
            .expect("genesis")
            .head_sequence(),
        1
    );
    assert!(matches!(
        prepare_runtime
            .resume(&prepare_run)
            .await
            .expect("resume after absent prepare")
            .state(),
        RunViewState::Succeeded(_)
    ));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);

    let conclusion_calls = Arc::new(AtomicUsize::new(0));
    let conclusion_store = Arc::new(SequenceIndeterminateStore::new(3, true));
    let mut conclusion_builder = RuntimeAssemblyBuilder::new();
    conclusion_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    conclusion_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let conclusion_calls = Arc::clone(&conclusion_calls);
            move |effect_id, command| {
                conclusion_calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let conclusion_runtime = Runtime::new(
        conclusion_builder.finish().expect("assembly"),
        conclusion_store,
    );
    let conclusion_run = RunId::from_digest(DigestBytes::from_array([34; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/conclusion-indeterminate@1").expect("entry point"),
        &EffectProgram,
    )
    .expect("Program");
    assert!(matches!(
        conclusion_runtime
            .start(conclusion_run.clone(), program, Number { value: 4 })
            .await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(conclusion_calls.load(Ordering::SeqCst), 1);
    let recovered = conclusion_runtime
        .resume(&conclusion_run)
        .await
        .expect("committed conclusion");
    assert_eq!(recovered.head_sequence(), 3);
    assert!(matches!(recovered.state(), RunViewState::Succeeded(_)));
    assert_eq!(conclusion_calls.load(Ordering::SeqCst), 1);

    let committed_prepare_calls = Arc::new(AtomicUsize::new(0));
    let committed_prepare_store = Arc::new(SequenceIndeterminateStore::new(2, true));
    let mut committed_prepare_builder = RuntimeAssemblyBuilder::new();
    committed_prepare_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    committed_prepare_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let committed_prepare_calls = Arc::clone(&committed_prepare_calls);
            move |effect_id, command| {
                committed_prepare_calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let committed_prepare_runtime = Runtime::new(
        committed_prepare_builder.finish().expect("assembly"),
        committed_prepare_store,
    );
    let committed_prepare_run = RunId::from_digest(DigestBytes::from_array([43; 32]));
    assert!(matches!(
        committed_prepare_runtime
            .start(
                committed_prepare_run.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/prepare-committed-indeterminate@1")
                        .expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 5 },
            )
            .await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(committed_prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        committed_prepare_runtime
            .read(&committed_prepare_run)
            .await
            .expect("committed prepare")
            .head_sequence(),
        2
    );
    let completed = committed_prepare_runtime
        .resume(&committed_prepare_run)
        .await
        .expect("resume committed prepare");
    assert_eq!(completed.head_sequence(), 3);
    assert_eq!(committed_prepare_calls.load(Ordering::SeqCst), 1);

    let absent_conclusion_calls = Arc::new(AtomicUsize::new(0));
    let absent_conclusion_store = Arc::new(SequenceIndeterminateStore::new(3, false));
    let mut absent_conclusion_builder = RuntimeAssemblyBuilder::new();
    absent_conclusion_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    absent_conclusion_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let absent_conclusion_calls = Arc::clone(&absent_conclusion_calls);
            move |effect_id, command| {
                absent_conclusion_calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let absent_conclusion_runtime = Runtime::new(
        absent_conclusion_builder.finish().expect("assembly"),
        absent_conclusion_store,
    );
    let absent_conclusion_run = RunId::from_digest(DigestBytes::from_array([44; 32]));
    assert!(matches!(
        absent_conclusion_runtime
            .start(
                absent_conclusion_run.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/conclusion-absent-indeterminate@1")
                        .expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 6 },
            )
            .await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(absent_conclusion_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        absent_conclusion_runtime
            .read(&absent_conclusion_run)
            .await
            .expect("absent conclusion")
            .head_sequence(),
        2
    );
    let completed = absent_conclusion_runtime
        .resume(&absent_conclusion_run)
        .await
        .expect("retry absent conclusion");
    assert_eq!(completed.head_sequence(), 3);
    assert_eq!(absent_conclusion_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn effect_not_inserted_reloads_the_committed_prepare_or_conclusion() {
    for (offset, sequence) in [2_u64, 3].into_iter().enumerate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(SequenceNotInsertedStore::new(sequence));
        let mut builder = RuntimeAssemblyBuilder::new();
        builder
            .register_effect::<Mutate, Mutation>()
            .expect("Effect State");
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let calls = Arc::clone(&calls);
                move |effect_id, command| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let effect_id = effect_id.clone();
                    let value = command.value;
                    Box::pin(async move {
                        Ok(EffectEvidence {
                            effect_id,
                            value,
                            accepted: true,
                        })
                    })
                }
            })
            .expect("adapter");
        let runtime = Runtime::new(builder.finish().expect("assembly"), store);
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(38 + offset).expect("RunId byte"); 32],
        ));
        let completed = runtime
            .start(
                run_id,
                expand_program(
                    EntryPointId::new(format!("mfm.test.runtime/not-inserted-{sequence}@1"))
                        .expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 8 },
            )
            .await
            .expect("converged Effect");
        assert_eq!(completed.head_sequence(), 3);
        assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn every_adapter_failure_leaves_one_pending_prepare() {
    #[derive(Clone, Copy)]
    enum FailureMode {
        Unavailable,
        Internal,
        Panic,
    }

    for (offset, mode) in [
        FailureMode::Unavailable,
        FailureMode::Internal,
        FailureMode::Panic,
    ]
    .into_iter()
    .enumerate()
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(MemoryStore::new());
        let mut builder = RuntimeAssemblyBuilder::new();
        builder
            .register_effect::<Mutate, Mutation>()
            .expect("Effect State");
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let calls = Arc::clone(&calls);
                move |_effect_id, _command| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    match mode {
                        FailureMode::Unavailable => {
                            Box::pin(async { Err(AdapterError::Unavailable) })
                        }
                        FailureMode::Internal => Box::pin(async { Err(AdapterError::Internal) }),
                        FailureMode::Panic => panic!("adapter panic"),
                    }
                }
            })
            .expect("adapter");
        let runtime = Runtime::new(builder.finish().expect("assembly"), store);
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(40 + offset).expect("RunId byte"); 32],
        ));
        let result = runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new(format!("mfm.test.runtime/adapter-failure-{offset}@1"))
                        .expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 9 },
            )
            .await;
        let expected = match mode {
            FailureMode::Unavailable => RuntimeError::Unavailable,
            FailureMode::Internal | FailureMode::Panic => RuntimeError::Internal,
        };
        assert!(matches!(result, Err(error) if error == expected));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pending = runtime.read(&run_id).await.expect("pending view");
        assert_eq!(pending.head_sequence(), 2);
        assert!(matches!(pending.state(), RunViewState::Runnable));
    }
}

#[tokio::test]
async fn retained_effect_facts_are_validated_without_adapter_io() {
    let pending_store = Arc::new(RecordingStore::new());
    let mut pending_builder = RuntimeAssemblyBuilder::new();
    pending_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    pending_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |_, _| {
            Box::pin(async { Err(AdapterError::Unavailable) })
        })
        .expect("adapter");
    let pending_runtime = Runtime::new(
        pending_builder.finish().expect("assembly"),
        pending_store.clone(),
    );
    let run_id = RunId::from_digest(DigestBytes::from_array([36; 32]));
    assert!(matches!(
        pending_runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/retained-effect@1").expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 7 },
            )
            .await,
        Err(RuntimeError::Unavailable)
    ));
    let pending_frames = pending_store.snapshot();
    assert_eq!(pending_frames.len(), 2);

    let read_calls = Arc::new(AtomicUsize::new(0));
    let mut read_builder = RuntimeAssemblyBuilder::new();
    read_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    read_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let read_calls = Arc::clone(&read_calls);
            move |effect_id, command| {
                read_calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let read_runtime = Runtime::new(
        read_builder.finish().expect("assembly"),
        Arc::new(RetainedStore {
            frames: pending_frames.clone(),
        }),
    );
    let pending = read_runtime.read(&run_id).await.expect("pending view");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(pending.state(), RunViewState::Runnable));
    assert_eq!(read_calls.load(Ordering::SeqCst), 0);

    let mut wrong_id_frames = pending_frames.clone();
    let mut wrong_id: serde_json::Value =
        serde_json::from_slice(&wrong_id_frames[1]).expect("prepare wire");
    wrong_id["record"]["effect_id"] =
        serde_json::json!(EffectId::from_digest(DigestBytes::from_array([88; 32])).as_str());
    wrong_id_frames[1] =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&wrong_id).expect("json"))
            .expect("canonical")
            .to_vec();

    let mut wrong_command_frames = pending_frames;
    let mut wrong_command: serde_json::Value =
        serde_json::from_slice(&wrong_command_frames[1]).expect("prepare wire");
    let replacement_command = br#"{"value":99}"#;
    let replacement_digest = raw_content_digest(replacement_command);
    let original_command_ref = wrong_command["record"]["command"].clone();
    wrong_command["record"]["command"]["content_digest"] =
        serde_json::json!(replacement_digest.as_str());
    for object in wrong_command["objects"].as_array_mut().expect("objects") {
        if object["content_ref"] == original_command_ref {
            object["content_ref"]["content_digest"] =
                serde_json::json!(replacement_digest.as_str());
            object["canonical"] = serde_json::from_slice(replacement_command).expect("command");
        }
    }
    wrong_command_frames[1] = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&wrong_command).expect("json"),
    )
    .expect("canonical")
    .to_vec();

    for frames in [wrong_id_frames, wrong_command_frames] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RuntimeAssemblyBuilder::new();
        builder
            .register_effect::<Mutate, Mutation>()
            .expect("Effect State");
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let calls = Arc::clone(&calls);
                move |effect_id, command| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let effect_id = effect_id.clone();
                    let value = command.value;
                    Box::pin(async move {
                        Ok(EffectEvidence {
                            effect_id,
                            value,
                            accepted: true,
                        })
                    })
                }
            })
            .expect("adapter");
        let runtime = Runtime::new(
            builder.finish().expect("assembly"),
            Arc::new(RetainedStore { frames }),
        );
        assert!(matches!(
            runtime.read(&run_id).await,
            Err(RuntimeError::InvalidHistory)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    let settled_store = Arc::new(RecordingStore::new());
    let mut settled_builder = RuntimeAssemblyBuilder::new();
    settled_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    settled_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |effect_id, command| {
            let effect_id = effect_id.clone();
            let value = command.value;
            Box::pin(async move {
                Ok(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                })
            })
        })
        .expect("adapter");
    let settled_runtime = Runtime::new(
        settled_builder.finish().expect("assembly"),
        settled_store.clone(),
    );
    let settled_run_id = RunId::from_digest(DigestBytes::from_array([37; 32]));
    settled_runtime
        .start(
            settled_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.runtime/retained-settlement@1").expect("entry point"),
                &EffectProgram,
            )
            .expect("Program"),
            Number { value: 7 },
        )
        .await
        .expect("settled Effect");
    let settled_frames = settled_store.snapshot();
    assert_eq!(settled_frames.len(), 3);

    let mut swapped_evidence_frames = settled_frames.clone();
    let replacement_evidence = serde_json::json!({
        "accepted": true,
        "effect_id": EffectId::from_digest(DigestBytes::from_array([89; 32])),
        "value": 7,
    });
    let replacement_evidence_bytes = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&replacement_evidence).expect("json"),
    )
    .expect("canonical");
    let replacement_evidence_digest = raw_content_digest(replacement_evidence_bytes.as_bytes());
    let mut swapped_evidence: serde_json::Value =
        serde_json::from_slice(&swapped_evidence_frames[2]).expect("conclusion wire");
    let original_evidence_ref = swapped_evidence["record"]["evidence"].clone();
    swapped_evidence["record"]["evidence"]["content_digest"] =
        serde_json::json!(replacement_evidence_digest.as_str());
    for object in swapped_evidence["objects"].as_array_mut().expect("objects") {
        if object["content_ref"] == original_evidence_ref {
            object["content_ref"]["content_digest"] =
                serde_json::json!(replacement_evidence_digest.as_str());
            object["canonical"] = replacement_evidence.clone();
        }
    }
    swapped_evidence["objects"]
        .as_array_mut()
        .expect("objects")
        .sort_by(|left, right| {
            left["content_ref"]["schema_id"]
                .as_str()
                .expect("left schema")
                .cmp(
                    right["content_ref"]["schema_id"]
                        .as_str()
                        .expect("right schema"),
                )
                .then_with(|| {
                    left["content_ref"]["content_digest"]
                        .as_str()
                        .expect("left digest")
                        .cmp(
                            right["content_ref"]["content_digest"]
                                .as_str()
                                .expect("right digest"),
                        )
                })
        });
    swapped_evidence_frames[2] = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&swapped_evidence).expect("json"),
    )
    .expect("canonical")
    .to_vec();

    let mut wrong_outcome_frames = settled_frames;
    let prepare: serde_json::Value =
        serde_json::from_slice(&wrong_outcome_frames[1]).expect("prepare wire");
    let command_schema = prepare["record"]["command"]["schema_id"].clone();
    let mut wrong_outcome: serde_json::Value =
        serde_json::from_slice(&wrong_outcome_frames[2]).expect("conclusion wire");
    let original_outcome_ref = wrong_outcome["record"]["outcome"]["value"].clone();
    wrong_outcome["record"]["outcome"]["value"]["schema_id"] = command_schema.clone();
    for object in wrong_outcome["objects"].as_array_mut().expect("objects") {
        if object["content_ref"] == original_outcome_ref {
            object["content_ref"]["schema_id"] = command_schema.clone();
        }
    }
    wrong_outcome["objects"]
        .as_array_mut()
        .expect("objects")
        .sort_by(|left, right| {
            left["content_ref"]["schema_id"]
                .as_str()
                .expect("left schema")
                .cmp(
                    right["content_ref"]["schema_id"]
                        .as_str()
                        .expect("right schema"),
                )
                .then_with(|| {
                    left["content_ref"]["content_digest"]
                        .as_str()
                        .expect("left digest")
                        .cmp(
                            right["content_ref"]["content_digest"]
                                .as_str()
                                .expect("right digest"),
                        )
                })
        });
    wrong_outcome_frames[2] = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&wrong_outcome).expect("json"),
    )
    .expect("canonical")
    .to_vec();

    for frames in [swapped_evidence_frames, wrong_outcome_frames] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RuntimeAssemblyBuilder::new();
        builder
            .register_effect::<Mutate, Mutation>()
            .expect("Effect State");
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let calls = Arc::clone(&calls);
                move |effect_id, command| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let effect_id = effect_id.clone();
                    let value = command.value;
                    Box::pin(async move {
                        Ok(EffectEvidence {
                            effect_id,
                            value,
                            accepted: true,
                        })
                    })
                }
            })
            .expect("adapter");
        let runtime = Runtime::new(
            builder.finish().expect("assembly"),
            Arc::new(RetainedStore { frames }),
        );
        assert!(matches!(
            runtime.read(&settled_run_id).await,
            Err(RuntimeError::InvalidHistory)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn concurrent_pending_effect_callers_converge_on_one_conclusion() {
    let store = Arc::new(MemoryStore::new());
    let mut unavailable_builder = RuntimeAssemblyBuilder::new();
    unavailable_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    unavailable_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |_, _| {
            Box::pin(async { Err(AdapterError::Unavailable) })
        })
        .expect("adapter");
    let unavailable = Runtime::new(
        unavailable_builder.finish().expect("assembly"),
        store.clone(),
    );
    let run_id = RunId::from_digest(DigestBytes::from_array([35; 32]));
    assert!(matches!(
        unavailable
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/concurrent-effect@1").expect("entry point"),
                    &EffectProgram,
                )
                .expect("Program"),
                Number { value: 5 },
            )
            .await,
        Err(RuntimeError::Unavailable)
    ));

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let barrier = Arc::clone(&barrier);
            let ids = Arc::clone(&ids);
            move |effect_id, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                ids.lock().expect("ids").push(effect_id.clone());
                let barrier = Arc::clone(&barrier);
                Box::pin(async move {
                    barrier.wait().await;
                    Ok(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Arc::new(Runtime::new(builder.finish().expect("assembly"), store));
    let left = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id).await })
    };
    let right = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id).await })
    };
    let left = left.await.expect("left task").expect("left resume");
    let right = right.await.expect("right task").expect("right resume");
    assert_eq!(left.head_sequence(), 3);
    assert_eq!(right.head_digest(), left.head_digest());
    let ids = ids.lock().expect("ids");
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

struct FaultStore {
    failure: StoreError,
}

impl Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Err(self.failure) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move { Err(self.failure) })
    }
}

#[tokio::test]
async fn store_failures_map_by_load_or_append_authority() {
    for (offset, (failure, load_error, append_error)) in [
        (
            StoreError::Capacity,
            RuntimeError::Internal,
            RuntimeError::Capacity,
        ),
        (
            StoreError::CorruptPhysicalState,
            RuntimeError::InvalidHistory,
            RuntimeError::InvalidHistory,
        ),
        (
            StoreError::Unavailable,
            RuntimeError::Unavailable,
            RuntimeError::Unavailable,
        ),
        (
            StoreError::Indeterminate,
            RuntimeError::Internal,
            RuntimeError::Indeterminate,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut builder = RuntimeAssemblyBuilder::new();
        builder.register_value::<Number>().expect("root value");
        let runtime = Runtime::new(
            builder.finish().expect("assembly"),
            Arc::new(FaultStore { failure }),
        );
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(offset + 10).expect("RunId byte"); 32],
        ));
        assert!(matches!(
            runtime.read(&run_id).await,
            Err(error) if error == load_error
        ));
        let program = expand_program(
            EntryPointId::new("mfm.test.runtime/fault@1").expect("entry point"),
            &EmptyProgram,
        )
        .expect("Program");
        assert!(matches!(
            runtime.start(run_id, program, Number { value: 1 }).await,
            Err(error) if error == append_error
        ));
    }
}
