use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_journal::{EncodedRunFrame, StoredRunBytes};
use mfm_program::{
    expand_program, CapabilityInjection, Never, Operation, OperationExpansion, ProgramError,
    ProposedStateOutcome, PureState, ReadPreparationError, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{ReadAdapterError, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError};
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
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
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
            Box::pin(async { Err(ReadAdapterError::Unavailable) })
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
