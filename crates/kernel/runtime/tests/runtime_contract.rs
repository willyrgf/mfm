use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::AdapterError;
use mfm_capabilities::EffectAdapterOutcome;
use mfm_ids::{DigestBytes, EffectId, EntryPointId, RunId};
use mfm_journal::{decode_frame, seal_frame};
use mfm_program::{compile, load, Identity, ProgramLimits};
use mfm_runtime::{InvocationFailure, RunViewState, Runtime, RuntimeError};
use mfm_store::{MemoryStore, Store, StoreError};
use mfm_values::{canonicalize_mfm_value, InvocationDiagnostic};

#[path = "support/callback_errors.rs"]
mod callback_errors;
#[path = "support/injection.rs"]
mod injection;
#[path = "support/program.rs"]
mod program;
#[path = "support/scripted_store.rs"]
mod scripted_store;

#[path = "support/resources.rs"]
mod resources;
use resources::*;
type Installed = (ReadSource, EffectSource, Identity<Number>);
use program::*;
use scripted_store::*;

// Mode is part of native ABI identity even when semantic and implementation IDs are equal.
#[test]
fn native_abi_distinguishes_capability_modes() {
    assert_ne!(
        mfm_program::NativeAbi::read::<ConflictingReadCapability, Native>().unwrap(),
        mfm_program::NativeAbi::effect::<Mutation, Native>().unwrap()
    );
}

#[tokio::test]
async fn missing_effect_adapter_is_rejected_before_store_io() {
    let resources = Resources::<Installed>::default();
    let runtime = Runtime::new(Arc::new(MemoryStore::new()));
    let run_id = RunId::from_digest(DigestBytes::from_array([29; 32]));
    let error = compile(
        EntryPointId::new("mfm.test.runtime/missing-effect-adapter@1").unwrap(),
        &EffectSource::new(Binding { route: 8 }),
        &Number { value: 1 },
        &resources,
        ProgramLimits::new(0),
    )
    .err()
    .unwrap();
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(diagnostic.contains("bind_effect"), "{diagnostic}");
    assert!(matches!(
        runtime.program_document(&run_id).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Absent,
            ..
        })
    ));
}

// Reading retained success must not call the provider again; an interrupted Read can later
// record a domain failure and its root mapping.
#[tokio::test]
async fn read_success_and_separate_failure_recovery_are_restorable_without_repeating_io() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let builder = Resources::<Installed>::read({
        let calls = Arc::clone(&calls);
        move |intent_value_ref, intent| {
            calls.fetch_add(1, Ordering::SeqCst);
            let intent_value_ref = intent_value_ref.clone();
            Box::pin(async move {
                Ok(Evidence {
                    intent_value_ref,
                    value: intent.value,
                    accepted: true,
                })
            })
        }
    });
    let runtime = Runtime::new(store);
    let run_id = RunId::from_digest(DigestBytes::from_array([3; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadSource::new(Binding { route: 7 }),
        &Number { value: 12 },
        &builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    let hot = runtime
        .start(run_id.clone(), &program, &Number { value: 12 })
        .await
        .expect("Read execution");
    assert!(matches!(hot.state(), RunViewState::Succeeded(_)));
    assert_eq!(hot.head_sequence(), 2);
    let cold = runtime.read(&run_id, &program).await.expect("cold read");
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let unavailable_store = Arc::new(MemoryStore::new());
    let unavailable_builder = Resources::<Installed>::read(|_, _| {
        Box::pin(async {
            Err(AdapterError::Invariant(InvocationDiagnostic::from_fields(
                "state_internal",
                "read_success_and_separate_failure_recovery_are_restorable_without_repeating_io",
                &(AdapterRejected),
                None,
            )))
        })
    });
    let unavailable = Runtime::new(unavailable_store.clone());
    let interrupted_run_id = RunId::from_digest(DigestBytes::from_array([4; 32]));
    let interrupted_program = compile(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadSource::new(Binding { route: 7 }),
        &Number { value: 21 },
        &unavailable_builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    assert!(matches!(
            unavailable
                .start(
    interrupted_run_id.clone(),
    &interrupted_program,
    &Number { value: 21 },
    )
                .await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Native {
                    operation: mfm_runtime::Operation::ReadAdapter,
                    stage: mfm_runtime::Stage::Execute,
                    cause,
                },
                ..
            }) if cause.details().as_value() == &serde_json::json!(null)
        ));
    let prefix = unavailable
        .read(&interrupted_run_id, &interrupted_program)
        .await
        .expect("durable prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable { .. }));

    let resumed_builder = Resources::<Installed>::read(|intent_value_ref, intent| {
        let intent_value_ref = intent_value_ref.clone();
        Box::pin(async move {
            Ok(Evidence {
                intent_value_ref,
                value: intent.value,
                accepted: false,
            })
        })
    });
    let interrupted_program =
        load(interrupted_program.canonical_bytes(), &resumed_builder).unwrap();
    let resumed = Runtime::new(unavailable_store)
        .resume(&interrupted_run_id, &interrupted_program)
        .await
        .expect("resume");
    let RunViewState::Failed(value) = resumed.state() else {
        panic!("rejected observation did not follow the Program failure path");
    };
    assert_eq!(resumed.head_sequence(), 3);
    let mfm_runtime::Failure::Domain { original, .. } = value.failure() else {
        panic!("domain result")
    };
    assert_eq!(original.decode::<Number>().unwrap().value, 21);
}

// Cancellation after adapter entry must leave a durable command that a rebuilt Runtime resumes
// with the same Effect identity.
#[tokio::test]
async fn effect_prepare_is_durable_before_adapter_entry_and_cold_resume_reuses_identity() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = Arc::new(MemoryStore::new());
    let builder = Resources::<Installed>::effect({
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        let observed = Arc::clone(&observed);
        move |effect_id, command_value_ref, command| {
            let effect_id = effect_id.clone();
            let command_value_ref = command_value_ref.clone();
            let value = command.value;
            observed.lock().expect("observations").push((
                effect_id.clone(),
                command_value_ref,
                value,
            ));
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            Box::pin(async move {
                entered.notify_one();
                release.notified().await;
                Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                }))
            })
        }
    });
    let runtime = Arc::new(Runtime::new(store.clone()));
    let run_id = RunId::from_digest(DigestBytes::from_array([30; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.runtime/effect@1").expect("entry point"),
        &EffectSource::new(Binding { route: 8 }),
        &Number { value: 34 },
        &builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    let task = {
        let runtime = Arc::clone(&runtime);
        let program = program.clone();
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.start(run_id, &program, &Number { value: 34 }).await })
    };
    entered.notified().await;

    let pending = runtime.read(&run_id, &program).await.expect("pending view");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("Effect task was not cancelled"),
    }
    release.notify_waiters();
    let (first_effect, first_command_value_ref) = {
        let observations = observed.lock().expect("observations");
        (observations[0].0.clone(), observations[0].1.clone())
    };
    assert_eq!(
        first_command_value_ref,
        canonicalize_mfm_value(&Command { value: 34 })
            .map(|(_, value_ref)| value_ref)
            .expect("command value ref")
    );

    let resumed_observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let resumed_builder = Resources::<Installed>::effect({
        let resumed_observed = Arc::clone(&resumed_observed);
        move |effect_id, command_value_ref, command| {
            let effect_id = effect_id.clone();
            let command_value_ref = command_value_ref.clone();
            let value = command.value;
            resumed_observed
                .lock()
                .expect("resumed observations")
                .push((effect_id.clone(), command_value_ref, value));
            Box::pin(async move {
                Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                }))
            })
        }
    });
    let program = load(program.canonical_bytes(), &resumed_builder).unwrap();
    let resumed_runtime = Runtime::new(store);
    let resumed = resumed_runtime
        .resume(&run_id, &program)
        .await
        .expect("cold resume");
    assert_eq!(resumed.head_sequence(), 4);
    assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
    {
        let resumed_calls = resumed_observed.lock().expect("resumed observations");
        assert_eq!(
            resumed_calls.as_slice(),
            &[(first_effect, first_command_value_ref, 34)]
        );
    }

    let cold = resumed_runtime
        .read(&run_id, &program)
        .await
        .expect("cold view");
    assert_eq!(cold.head_digest(), resumed.head_digest());
    assert_eq!(
        resumed_observed.lock().expect("resumed observations").len(),
        1
    );
}

// A pending response must return control to the caller; only explicit resume may ask the adapter
// for settlement.
#[tokio::test]
async fn pending_yields_once_and_a_later_settlement_closes_the_same_prepare() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let builder = Resources::<Installed>::effect({
        let calls = Arc::clone(&calls);
        move |effect_id, _command_value_ref, command| {
            let invocation = calls.fetch_add(1, Ordering::SeqCst);
            let effect_id = effect_id.clone();
            let value = command.value;
            Box::pin(async move {
                if invocation == 0 {
                    Ok(EffectAdapterOutcome::Pending)
                } else {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                }
            })
        }
    });
    let runtime = Runtime::new(store);
    let run_id = RunId::from_digest(DigestBytes::from_array([45; 32]));

    let program = compile(
        EntryPointId::new("mfm.test.runtime/pending-effect@1").expect("entry point"),
        &EffectSource::new(Binding { route: 8 }),
        &Number { value: 21 },
        &builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    let pending = runtime
        .start(run_id.clone(), &program, &Number { value: 21 })
        .await
        .expect("pending is normal progress");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let cold = runtime
        .read(&run_id, &program)
        .await
        .expect("cold pending view");
    assert_eq!(cold.head_digest(), pending.head_digest());
    assert!(matches!(cold.state(), RunViewState::EffectPending { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let settled = runtime
        .resume(&run_id, &program)
        .await
        .expect("later settlement");
    assert_eq!(settled.head_sequence(), 4);
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// A preparation failure must prevent adapter entry, and invalid evidence must leave the prepared
// command unresolved.
#[tokio::test]
async fn effect_preparation_and_evidence_failures_append_no_conclusion() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let builder = Resources::<Installed>::effect({
        let calls = Arc::clone(&calls);
        move |effect_id, _command_value_ref, command| {
            calls.fetch_add(1, Ordering::SeqCst);
            let effect_id = effect_id.clone();
            let value = command.value;
            Box::pin(async move {
                Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                }))
            })
        }
    });
    let runtime = Runtime::new(store);
    let run_id = RunId::from_digest(DigestBytes::from_array([31; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.runtime/reject-effect@1").expect("entry point"),
        &EffectSource::new(Binding { route: 8 }),
        &Number {
            value: PREPARATION_FAILURE_SENTINEL,
        },
        &builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    assert!(matches!(
            runtime
                .start(
    run_id.clone(),
    &program,
    &Number {
                        value: PREPARATION_FAILURE_SENTINEL,
                    },
    )
                .await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Native {
                    operation: mfm_runtime::Operation::EffectPrepare,
                    stage: mfm_runtime::Stage::Execute,
                    cause,
                },
                ..
            }) if cause.details().as_value()["input"] == PREPARATION_FAILURE_SENTINEL
        ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runtime
            .read(&run_id, &program)
            .await
            .expect("genesis")
            .head_sequence(),
        1
    );

    let invalid_store = Arc::new(MemoryStore::new());
    let invalid_builder =
        Resources::<Installed>::effect(|_effect_id, _command_value_ref, command| {
            let value = command.value;
            Box::pin(async move {
                Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id: EffectId::from_digest(DigestBytes::from_array([99; 32])),
                    value,
                    accepted: true,
                }))
            })
        });
    let invalid = Runtime::new(invalid_store);
    let invalid_run_id = RunId::from_digest(DigestBytes::from_array([32; 32]));
    let invalid_program = compile(
        EntryPointId::new("mfm.test.runtime/invalid-evidence@1").expect("entry point"),
        &EffectSource::new(Binding { route: 8 }),
        &Number { value: 2 },
        &invalid_builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    assert!(matches!(
            invalid
                .start(
    invalid_run_id.clone(),
    &invalid_program,
    &Number { value: 2 },
    )
                .await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Native {
                    operation: mfm_runtime::Operation::EffectBind,
                    stage: mfm_runtime::Stage::Execute,
                    cause,
                },
                ..
            }) if cause.details().as_value() == &serde_json::json!("evidence_binding")
        ));
    assert_eq!(
        invalid
            .read(&invalid_run_id, &invalid_program)
            .await
            .expect("pending")
            .head_sequence(),
        2
    );
}

fn effect_runtime_with_counting_adapter(
    store: Arc<dyn Store>,
    calls: Arc<AtomicUsize>,
    effect_ids: Arc<std::sync::Mutex<Vec<EffectId>>>,
) -> (Runtime, Resources<Installed>) {
    let builder = Resources::<Installed>::effect(move |effect_id, _command_value_ref, command| {
        calls.fetch_add(1, Ordering::SeqCst);
        let effect_id = effect_id.clone();
        effect_ids
            .lock()
            .expect("effect ids")
            .push(effect_id.clone());
        let value = command.value;
        Box::pin(async move {
            Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                effect_id,
                value,
                accepted: true,
            }))
        })
    });
    (Runtime::new(store), builder)
}

fn retained_effect_reader(
    frames: Vec<Vec<u8>>,
    calls: Arc<AtomicUsize>,
) -> (Runtime, Resources<Installed>) {
    effect_runtime_with_counting_adapter(
        Arc::new(RetainedStore(frames)),
        calls,
        Arc::new(std::sync::Mutex::new(Vec::new())),
    )
}

// A lost append acknowledgement must recover from what was actually stored at each Effect
// boundary, retaining the same command identity.
#[tokio::test]
async fn ambiguous_effect_appends_recover_from_exact_retained_facts() {
    // Expected retained heads and adapter counts are independent of the Store script.
    let cases = [
        ("prepare-absent", 2, AppendAction::Indeterminate, 1, 0, 1),
        (
            "prepare-retained",
            2,
            AppendAction::RetainThenIndeterminate,
            2,
            0,
            1,
        ),
        ("settlement-absent", 3, AppendAction::Indeterminate, 2, 1, 2),
        (
            "settlement-retained",
            3,
            AppendAction::RetainThenIndeterminate,
            3,
            1,
            1,
        ),
        (
            "interpretation-absent",
            4,
            AppendAction::Indeterminate,
            3,
            1,
            1,
        ),
        (
            "interpretation-retained",
            4,
            AppendAction::RetainThenIndeterminate,
            4,
            1,
            1,
        ),
    ];
    for (
        offset,
        (
            name,
            sequence,
            action,
            expected_head_after_start,
            expected_calls_after_start,
            expected_calls_after_resume,
        ),
    ) in cases.into_iter().enumerate()
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let effect_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
        let store = Arc::new(ScriptedStore::new([(sequence, action)]));
        let (runtime, builder) = effect_runtime_with_counting_adapter(
            store,
            Arc::clone(&calls),
            Arc::clone(&effect_ids),
        );
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(33 + offset).expect("RunId byte"); 32],
        ));
        let program = compile(
            EntryPointId::new(format!("mfm.test.runtime/ambiguous-{}@1", name))
                .expect("entry point"),
            &EffectSource::new(Binding { route: 8 }),
            &Number {
                value: u64::try_from(offset + 3).expect("input value"),
            },
            &builder,
            ProgramLimits::new(0),
        )
        .expect("Program");

        assert_store_recording(
            runtime
                .start(
                    run_id.clone(),
                    &program,
                    &Number {
                        value: u64::try_from(offset + 3).expect("input value"),
                    },
                )
                .await
                .err()
                .unwrap(),
            StoreError::Indeterminate(mfm_values::DiagnosticEvidence::from_value(
                serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
            )),
            sequence,
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls_after_start,
            "{} adapter entries after start",
            name
        );
        let after_start = runtime
            .read(&run_id, &program)
            .await
            .expect("retained prefix");
        assert_eq!(
            after_start.head_sequence(),
            expected_head_after_start,
            "{} retained head after start",
            name
        );
        match expected_head_after_start {
            1 => assert!(matches!(after_start.state(), RunViewState::Runnable { .. })),
            2 => assert!(matches!(
                after_start.state(),
                RunViewState::EffectPending { .. }
            )),
            3 => assert!(matches!(
                after_start.state(),
                RunViewState::AwaitingInterpretation { .. }
            )),
            4 => assert!(matches!(after_start.state(), RunViewState::Succeeded(_))),
            _ => unreachable!("fixture head"),
        }

        let completed = runtime
            .resume(&run_id, &program)
            .await
            .expect("ambiguous recovery");
        assert_eq!(completed.head_sequence(), 4, "{} terminal head", name);
        assert!(
            matches!(completed.state(), RunViewState::Succeeded(_)),
            "{} terminal state",
            name
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls_after_resume,
            "{} adapter entries after resume",
            name
        );
        let effect_ids = effect_ids.lock().expect("effect ids");
        assert!(effect_ids.windows(2).all(|pair| pair[0] == pair[1]));
    }
}

// Losing an append race must return the winning record and yield before executing the next step
// on its behalf.
#[tokio::test]
async fn effect_not_inserted_returns_the_winner_without_entering_its_new_visit() {
    for (offset, sequence) in [2_u64, 3, 4].into_iter().enumerate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(ScriptedStore::new([(
            sequence,
            AppendAction::RetainThenNotInserted,
        )]));
        let (runtime, builder) = effect_runtime_with_counting_adapter(
            store,
            Arc::clone(&calls),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        );
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(38 + offset).expect("RunId byte"); 32],
        ));
        let program = compile(
            EntryPointId::new(format!("mfm.test.runtime/not-inserted-{sequence}@1"))
                .expect("entry point"),
            &EffectSource::new(Binding { route: 8 }),
            &Number { value: 8 },
            &builder,
            ProgramLimits::new(0),
        )
        .expect("Program");
        let completed = runtime
            .start(run_id.clone(), &program, &Number { value: 8 })
            .await
            .expect("converged Effect");
        assert_eq!(completed.head_sequence(), sequence);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if sequence == 2 { 0 } else { 1 }
        );
        if sequence == 2 {
            assert!(matches!(
                completed.state(),
                RunViewState::EffectPending { .. }
            ));
        } else if sequence == 3 {
            assert!(matches!(
                completed.state(),
                RunViewState::AwaitingInterpretation { .. }
            ));
        } else {
            assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        }
        let resumed = runtime.resume(&run_id, &program).await.unwrap();
        assert_eq!(resumed.head_sequence(), 4);
        assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

// Declared adapter failures require durable audit records; internal errors and panics must leave
// the command pending without a fabricated outcome.
#[tokio::test]
async fn operational_failures_are_audited_while_internal_failures_preserve_prepare() {
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
        let builder = Resources::<Installed>::effect({
            let calls = Arc::clone(&calls);
            move |_effect_id, _command_value_ref, _command| {
                calls.fetch_add(1, Ordering::SeqCst);
                match mode {
                    FailureMode::Unavailable => Box::pin(async {
                        Err(AdapterError::Operational(OperationalFailure::Unavailable))
                    }),
                    FailureMode::Internal => Box::pin(async {
                        Err(AdapterError::Invariant(InvocationDiagnostic::from_fields("state_internal", "operational_failures_are_audited_while_internal_failures_preserve_prepare", &(AdapterRejected), None)))
                    }),
                    FailureMode::Panic => panic!("adapter panic"),
                }
            }
        });
        let runtime = Runtime::new(store);
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(40 + offset).expect("RunId byte"); 32],
        ));
        let program = compile(
            EntryPointId::new(format!("mfm.test.runtime/adapter-failure-{offset}@1"))
                .expect("entry point"),
            &EffectSource::new(Binding { route: 8 }),
            &Number { value: 9 },
            &builder,
            ProgramLimits::new(0),
        )
        .expect("Program");
        let result = runtime
            .start(run_id.clone(), &program, &Number { value: 9 })
            .await;
        match mode {
            FailureMode::Unavailable => assert!(matches!(
                result,
                Err(InvocationFailure::RecoveryStopped { .. })
            )),
            FailureMode::Internal | FailureMode::Panic => {
                let Err(InvocationFailure::Execution {
                    error:
                        RuntimeError::Native {
                            operation: mfm_runtime::Operation::EffectAdapter,
                            stage: mfm_runtime::Stage::Execute,
                            cause,
                        },
                    ..
                }) = result
                else {
                    panic!("adapter failure context")
                };
                match mode {
                    FailureMode::Internal => {
                        assert!(cause.details().as_value() == &serde_json::json!(null))
                    }
                    FailureMode::Panic => {
                        assert_eq!(cause.details().as_value(), &serde_json::json!("panicked"))
                    }
                    FailureMode::Unavailable => unreachable!(),
                }
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pending = runtime.read(&run_id, &program).await.expect("pending view");
        assert_eq!(
            pending.head_sequence(),
            if matches!(mode, FailureMode::Unavailable) {
                4
            } else {
                2
            }
        );
        assert!(matches!(
            pending.state(),
            RunViewState::EffectPending { .. }
        ));
    }
}

// Inspecting a retained Effect must reject inconsistent command or output facts without
// contacting its adapter.
#[tokio::test]
async fn retained_effect_facts_are_validated_without_adapter_io() {
    for settled in [false, true] {
        let actions = if settled {
            vec![]
        } else {
            vec![(2, AppendAction::RetainThenNotInserted)]
        };
        let store = Arc::new(if settled {
            ScriptedStore::recording()
        } else {
            ScriptedStore::new(actions)
        });
        let (runtime, builder) = effect_runtime_with_counting_adapter(
            store.clone(),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        );
        let run_id =
            RunId::from_digest(DigestBytes::from_array([if settled { 37 } else { 36 }; 32]));
        let program = compile(
            EntryPointId::new("mfm.test.runtime/retained-effect@1").unwrap(),
            &EffectSource::new(Binding { route: 8 }),
            &Number { value: 7 },
            &builder,
            ProgramLimits::new(0),
        )
        .unwrap();
        let hot = runtime
            .start(run_id.clone(), &program, &Number { value: 7 })
            .await
            .unwrap();
        let frames = store.snapshot();
        let calls = Arc::new(AtomicUsize::new(0));
        let (reader, reader_resources) = retained_effect_reader(frames.clone(), calls.clone());
        let program = load(program.canonical_bytes(), &reader_resources).unwrap();
        assert_eq!(
            reader.read(&run_id, &program).await.unwrap().head_digest(),
            hot.head_digest()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let latest = decode_frame(frames.last().unwrap()).unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(latest.payload().as_bytes()).unwrap();
        for mutation in 0..if settled { 1 } else { 2 } {
            let mut payload = original.clone();
            if settled {
                let wrong_output = serde_json::to_value(
                    mfm_values::Object::from_value(&Command { value: 7 }).unwrap(),
                )
                .unwrap();
                payload["operation"]["succeeded"]["output"] = wrong_output;
            } else {
                let (name, replacement) = if mutation == 0 {
                    (
                        "effect_id",
                        serde_json::to_value(EffectId::from_digest(DigestBytes::from_array(
                            [88; 32],
                        )))
                        .unwrap(),
                    )
                } else {
                    (
                        "command",
                        serde_json::to_value(
                            mfm_values::Object::from_value(&Command { value: 99 }).unwrap(),
                        )
                        .unwrap(),
                    )
                };
                payload["operation"]["effect_prepared"][name] = replacement;
            }
            let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
                &serde_json::to_string(&payload).unwrap(),
            )
            .unwrap();
            let changed = seal_frame(
                &run_id,
                latest.run_sequence(),
                latest.previous_head_digest(),
                &payload,
            )
            .unwrap();
            let mut changed_frames = frames.clone();
            *changed_frames.last_mut().unwrap() = changed.canonical_bytes().to_vec();
            let calls = Arc::new(AtomicUsize::new(0));
            let (reader, reader_resources) = retained_effect_reader(changed_frames, calls.clone());
            let program = load(program.canonical_bytes(), &reader_resources).unwrap();
            assert!(
                reader.read(&run_id, &program).await.is_err(),
                "settled={settled}, mutation={mutation}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
}

// Two callers may reconcile the same command, but their competing appends must converge on one
// retained settlement and completion.
#[tokio::test]
async fn concurrent_pending_effect_callers_converge_on_one_conclusion() {
    let store = Arc::new(MemoryStore::new());
    let unavailable_builder = Resources::<Installed>::effect(|_, _, _| {
        Box::pin(async { Err(AdapterError::Operational(OperationalFailure::Unavailable)) })
    });
    let unavailable = Runtime::new(store.clone());
    let run_id = RunId::from_digest(DigestBytes::from_array([35; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.runtime/concurrent-effect@1").expect("entry point"),
        &EffectSource::new(Binding { route: 8 }),
        &Number { value: 5 },
        &unavailable_builder,
        ProgramLimits::new(0),
    )
    .expect("Program");
    assert!(matches!(
        unavailable
            .start(run_id.clone(), &program, &Number { value: 5 },)
            .await,
        Err(InvocationFailure::RecoveryStopped { .. })
    ));

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let builder = Resources::<Installed>::effect({
        let barrier = Arc::clone(&barrier);
        let ids = Arc::clone(&ids);
        move |effect_id, _command_value_ref, command| {
            let effect_id = effect_id.clone();
            let value = command.value;
            ids.lock().expect("ids").push(effect_id.clone());
            let barrier = Arc::clone(&barrier);
            Box::pin(async move {
                barrier.wait().await;
                Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                    effect_id,
                    value,
                    accepted: true,
                }))
            })
        }
    });
    let runtime = Arc::new(Runtime::new(store));
    let program = load(program.canonical_bytes(), &builder).unwrap();
    let left = {
        let runtime = Arc::clone(&runtime);
        let program = program.clone();
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id, &program).await })
    };
    let right = {
        let runtime = Arc::clone(&runtime);
        let program = program.clone();
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id, &program).await })
    };
    let left = left.await.expect("left task").expect("left resume");
    let right = right.await.expect("right task").expect("right resume");
    for view in [&left, &right] {
        assert!(matches!(view.head_sequence(), 5 | 6));
        if view.head_sequence() == 5 {
            assert!(matches!(
                view.state(),
                RunViewState::AwaitingInterpretation { .. }
            ));
        } else {
            assert!(matches!(view.state(), RunViewState::Succeeded(_)));
        }
    }
    assert!(left.head_sequence() == 6 || right.head_sequence() == 6);
    assert_eq!(
        runtime
            .read(&run_id, &program)
            .await
            .unwrap()
            .head_sequence(),
        6
    );
    let ids = ids.lock().expect("ids");
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

// A Store failure must preserve its mechanical cause and candidate without claiming an
// observation that was never obtained.
#[tokio::test]
async fn store_failures_preserve_mechanical_source_and_unknown_observation() {
    for (offset, failure) in [
        StoreError::ArithmeticOverflow,
        StoreError::CorruptPhysicalState(mfm_values::DiagnosticEvidence::from_value(
            serde_json::json!({"operation": "test.store", "injected": "CorruptPhysicalState"}),
        )),
        StoreError::Unavailable(mfm_values::DiagnosticEvidence::from_value(
            serde_json::json!({"operation": "test.store", "injected": "Unavailable"}),
        )),
        StoreError::Indeterminate(mfm_values::DiagnosticEvidence::from_value(
            serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
        )),
    ]
    .into_iter()
    .enumerate()
    {
        let builder = Resources::<Installed>::default();
        let runtime = Runtime::new(Arc::new(FaultStore {
            failure: failure.clone(),
        }));
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(offset + 10).expect("RunId byte"); 32],
        ));
        assert!(matches!(
            runtime.program_document(&run_id).await,
            Err(InvocationFailure::Execution { error: RuntimeError::Store(source), last_observed: None, .. }) if source == failure
        ));
        let program = compile(
            EntryPointId::new("mfm.test.runtime/fault@1").expect("entry point"),
            &Identity::<Number>::default(),
            &Number { value: 1 },
            &builder,
            ProgramLimits::new(0),
        )
        .expect("Program");
        assert_store_recording(
            runtime
                .start(run_id, &program, &Number { value: 1 })
                .await
                .err()
                .unwrap(),
            failure,
            1,
        );
    }
}

fn assert_store_recording(failure: InvocationFailure, expected: StoreError, sequence: u64) {
    let InvocationFailure::Execution {
        error: RuntimeError::Recording { failure, .. },
        ..
    } = failure
    else {
        panic!("recording custody")
    };
    let mfm_runtime::RecordingFailure::Store {
        original: None,
        candidate,
        cause: source,
    } = failure.as_ref()
    else {
        panic!("Store outcome without speculative probe")
    };
    assert_eq!(source, &expected);
    assert_eq!(candidate.run_sequence(), sequence);
}
