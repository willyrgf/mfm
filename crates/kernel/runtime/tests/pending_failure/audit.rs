use super::*;

#[tokio::test]
async fn pending_retry_and_exhausted_stop_are_audited_without_changing_command_authority() {
    let store = Arc::new(scripted_store::ScriptedStore::recording());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Execute, Submit>().unwrap();
        builder.register_map::<Identity<Never>>().unwrap();
        builder.register_handler::<RetryUnknown>().unwrap();
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(
                Number { value: 1 },
                move |effect, command_ref, command| {
                    seen.lock()
                        .unwrap()
                        .push((effect.clone(), command_ref.clone(), command.value));
                    Box::pin(async {
                        Err::<EffectAdapterOutcome<Number>, _>(AdapterError::Operational(
                            Cause::Timeout { deadline_ms: 5000 },
                        ))
                    })
                },
            )
            .unwrap();
        builder.finish()
    };
    let runtime = Runtime::new(build(), store.clone());
    let run = RunId::from_digest(DigestBytes::from_array([71; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/audited@1").unwrap(),
        &Flow,
        &Number { value: 9 },
        ProgramLimits::new(1),
    )
    .unwrap();
    let first = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .unwrap();
    assert_eq!(first.head_sequence(), 4);
    let RunViewState::EffectPending {
        effect,
        latest_failure: Some(failure),
    } = first.state()
    else {
        panic!("pending retry")
    };
    assert_eq!(failure.1, RecoveryOutcome::Retry);
    assert!(matches!(
        failure.0.decode::<Cause>().unwrap(),
        Cause::Timeout { deadline_ms: 5000 }
    ));
    assert_eq!(effect.call().input().decode::<Number>().unwrap().value, 9);
    let cold = Runtime::new(build(), store.clone());
    let reread = cold.read(&run).await.unwrap();
    assert_eq!(reread.head_digest(), first.head_digest());
    assert_eq!(calls.lock().unwrap().len(), 1);
    let InvocationFailure::RecoveryStopped { observed, .. } =
        cold.resume(&run).await.err().unwrap()
    else {
        panic!("stopped")
    };
    assert_eq!(observed.head_sequence(), 6);
    let RunViewState::EffectPending {
        effect: stopped_effect,
        latest_failure: Some(failure),
    } = observed.state()
    else {
        panic!("pending stop")
    };
    assert_eq!(stopped_effect.call().position(), effect.call().position());
    assert_eq!(stopped_effect.effect_id(), effect.effect_id());
    assert_eq!(
        failure.1,
        RecoveryOutcome::Stop {
            reason: StopReason::Exhausted(RecoveryLimit::StateRetry),
            root: None
        }
    );
    let InvocationFailure::RecoveryStopped {
        observed: repeated, ..
    } = cold.resume(&run).await.err().unwrap()
    else {
        panic!("unresolved command remains available after exhausted recovery")
    };
    assert_eq!(repeated.head_sequence(), 8);
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|call| call == &calls[0]));
    }
    let retained = cold.read(&run).await.unwrap();
    assert_eq!(retained.head_digest(), repeated.head_digest());
    assert_eq!(original_count(&store.snapshot()), 3);
}

#[tokio::test]
async fn standard_unknown_stop_is_durable_and_explicit_resume_can_settle() {
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Execute, Submit>().unwrap();
        builder.register_handler::<StandardRecovery>().unwrap();
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(
                Number { value: 1 },
                move |effect, command_ref, command| {
                    let attempt = {
                        let mut seen = seen.lock().unwrap();
                        seen.push((effect.clone(), command_ref.clone(), command.value));
                        seen.len()
                    };
                    Box::pin(async move {
                        if attempt == 1 {
                            Err(AdapterError::Operational(Cause::Timeout {
                                deadline_ms: 5000,
                            }))
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
    let runtime = Runtime::new(build(), store.clone());
    let run = RunId::from_digest(DigestBytes::from_array([72; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/audited-stop@1").unwrap(),
        &StopFlow,
        &Number { value: 9 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let InvocationFailure::RecoveryStopped { observed, .. } = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .err()
        .unwrap()
    else {
        panic!("standard stop")
    };
    assert_eq!(observed.head_sequence(), 4);
    let cold = Runtime::new(build(), store.clone());
    let retained = cold.read(&run).await.unwrap();
    assert_eq!(retained.head_digest(), observed.head_digest());
    let RunViewState::EffectPending {
        latest_failure: Some(failure),
        ..
    } = retained.state()
    else {
        panic!("retained original failure")
    };
    assert_eq!(
        failure.1,
        RecoveryOutcome::Stop {
            reason: StopReason::Requested,
            root: None
        }
    );
    assert!(matches!(
        failure.0.decode::<Cause>().unwrap(),
        Cause::Timeout { deadline_ms: 5000 }
    ));
    assert_eq!(calls.lock().unwrap().len(), 1);
    let settled = cold.resume(&run).await.unwrap();
    assert_eq!(settled.head_sequence(), 6);
    assert!(
        matches!(settled.state(), RunViewState::Succeeded(value) if value.decode::<Number>().unwrap().value == 9)
    );
    let terminal = cold.resume(&run).await.unwrap();
    assert_eq!(terminal.head_digest(), settled.head_digest());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
}

#[tokio::test]
async fn ambiguous_failure_appends_acknowledge_neither_an_uncommitted_cause_nor_a_new_command() {
    use scripted_store::{AppendAction, ScriptedStore};
    for (index, action, committed) in [
        (0, AppendAction::Indeterminate, false),
        (1, AppendAction::RetainThenIndeterminate, true),
        (2, AppendAction::RetainThenNotInserted, true),
    ] {
        let store = Arc::new(ScriptedStore::new([(3, action)]));
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Execute, Submit>().unwrap();
        builder.register_handler::<StandardRecovery>().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(
                Number { value: 1 },
                move |id, command_ref, command| {
                    seen.lock()
                        .unwrap()
                        .push((id.clone(), command_ref.clone(), command.value));
                    Box::pin(async {
                        Err::<EffectAdapterOutcome<Number>, _>(AdapterError::Operational(
                            Cause::Timeout { deadline_ms: 5000 },
                        ))
                    })
                },
            )
            .unwrap();
        let runtime = Runtime::new(builder.finish(), store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([80 + index; 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test/ambiguous-failure@1").unwrap(),
            &StopFlow,
            &Number { value: 9 },
            ProgramLimits::new(0),
        )
        .unwrap();
        let result = runtime
            .start(run.clone(), program, Number { value: 9 })
            .await;
        match action {
            AppendAction::RetainThenNotInserted => {
                let winner = result.ok().unwrap();
                assert_eq!(winner.head_sequence(), 3);
                assert!(matches!(
                    winner.state(),
                    RunViewState::AwaitingRecovery { .. }
                ));
            }
            _ => {
                let InvocationFailure::Execution {
                    error: RuntimeError::Recording { failure, .. },
                    last_observed: Some(observed),
                    ..
                } = result.err().unwrap()
                else {
                    panic!("indeterminate failure append")
                };
                let mfm_runtime::RecordingFailure::Store {
                    original: Some(original),
                    candidate,
                    cause: mfm_store::StoreError::Indeterminate,
                } = failure.as_ref()
                else {
                    panic!("original and ambiguous candidate custody")
                };
                assert!(matches!(
                    original.original().decode::<Cause>().ok(),
                    Some(Cause::Timeout { deadline_ms: 5000 })
                ));
                assert_eq!(candidate.run_sequence(), 3);
                assert_eq!(observed.head_sequence(), 2);
                assert!(matches!(
                    observed.state(),
                    RunViewState::EffectPending {
                        latest_failure: None,
                        ..
                    }
                ));
            }
        }
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_sequence(), if committed { 3 } else { 2 });
        assert_eq!(calls.lock().unwrap().len(), 1);
        let effect_id = if committed {
            let RunViewState::AwaitingRecovery {
                failure: mfm_runtime::Failure::PendingEffect { effect, original },
            } = cold.state()
            else {
                panic!("committed original awaits policy")
            };
            assert!(matches!(
                original.decode::<Cause>().unwrap(),
                Cause::Timeout { deadline_ms: 5000 }
            ));
            effect.effect_id()
        } else {
            let RunViewState::EffectPending {
                effect,
                latest_failure: None,
                ..
            } = cold.state()
            else {
                panic!("unrecorded attempt retains command")
            };
            effect.effect_id()
        };
        assert_eq!(effect_id, &calls.lock().unwrap()[0].0);
        let stored = store.snapshot();
        assert_eq!(stored.len(), if committed { 3 } else { 2 });
    }
}

struct PausedFailureStore {
    inner: MemoryStore,
    entered: tokio::sync::Notify,
    retain: bool,
    paused: std::sync::atomic::AtomicBool,
}
impl Store for PausedFailureStore {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        Option<mfm_store::LoadedRun>,
                        mfm_store::StoreError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run, probe_sequence)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<mfm_store::AppendResult, mfm_store::StoreError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if frame.run_sequence() == 3
                && !self.paused.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                if self.retain {
                    self.inner.append_run(frame).await?;
                }
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            self.inner.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn cancellation_at_failure_append_exposes_only_the_complete_committed_prefix() {
    for retain in [false, true] {
        let store = Arc::new(PausedFailureStore {
            inner: MemoryStore::new(),
            entered: tokio::sync::Notify::new(),
            retain,
            paused: std::sync::atomic::AtomicBool::new(false),
        });
        let calls = Arc::new(Mutex::new(Vec::new()));
        let build = || {
            let mut builder = RuntimeAssemblyBuilder::new().unwrap();
            builder.register_effect::<Execute, Submit>().unwrap();
            builder.register_handler::<StandardRecovery>().unwrap();
            let seen = Arc::clone(&calls);
            builder
                .register_effect_adapter::<Submit, _, _>(
                    Number { value: 1 },
                    move |id, command_ref, command| {
                        seen.lock()
                            .unwrap()
                            .push((id.clone(), command_ref.clone(), command.value));
                        Box::pin(async {
                            Err::<EffectAdapterOutcome<Number>, _>(AdapterError::Operational(
                                Cause::Timeout { deadline_ms: 5000 },
                            ))
                        })
                    },
                )
                .unwrap();
            builder.finish()
        };
        let runtime = Runtime::new(build(), store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([90 + u8::from(retain); 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test/cancelled-failure@1").unwrap(),
            &StopFlow,
            &Number { value: 9 },
            ProgramLimits::new(0),
        )
        .unwrap();
        let task_run = run.clone();
        let task =
            tokio::spawn(
                async move { runtime.start(task_run, program, Number { value: 9 }).await },
            );
        tokio::time::timeout(std::time::Duration::from_secs(5), store.entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        let cold = Runtime::new(build(), store.clone());
        let retained = cold.read(&run).await.unwrap();
        assert_eq!(retained.head_sequence(), if retain { 3 } else { 2 });
        let effect_id = if retain {
            let RunViewState::AwaitingRecovery {
                failure: mfm_runtime::Failure::PendingEffect { effect, original },
            } = retained.state()
            else {
                panic!("committed original")
            };
            assert!(matches!(
                original.decode::<Cause>().unwrap(),
                Cause::Timeout { deadline_ms: 5000 }
            ));
            effect.effect_id()
        } else {
            let RunViewState::EffectPending {
                effect,
                latest_failure: None,
                ..
            } = retained.state()
            else {
                panic!("unchanged prepare")
            };
            effect.effect_id()
        };
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(effect_id, &calls.lock().unwrap()[0].0);
        let InvocationFailure::RecoveryStopped { observed, .. } =
            cold.resume(&run).await.err().unwrap()
        else {
            panic!("acknowledged resumed failure")
        };
        assert_eq!(observed.head_sequence(), 4);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), if retain { 1 } else { 2 });
        assert!(calls.iter().all(|call| call == &calls[0]));
    }
}

#[tokio::test]
async fn current_record_validates_pending_failure_position_input_request_and_stop_reason() {
    for (wrong_visit, reported_input, reason, run_limit, valid) in [
        (false, 8, StopReason::Requested, 0, false),
        (true, 9, StopReason::Requested, 0, false),
        (false, 9, StopReason::Requested, 0, true),
        (
            false,
            9,
            StopReason::Disallowed(RecoveryDenial::EffectBarrier),
            0,
            true,
        ),
        (
            false,
            9,
            StopReason::Exhausted(RecoveryLimit::StateRetry),
            0,
            true,
        ),
        (false, 9, StopReason::Exhausted(RecoveryLimit::Run), 0, true),
        (
            false,
            9,
            StopReason::Exhausted(RecoveryLimit::Run),
            1,
            false,
        ),
        (
            false,
            9,
            StopReason::Exhausted(RecoveryLimit::StateRestart),
            0,
            false,
        ),
        (
            false,
            9,
            StopReason::Disallowed(RecoveryDenial::PureRetry),
            0,
            false,
        ),
        (
            false,
            9,
            StopReason::Disallowed(RecoveryDenial::CheckpointUnavailable),
            0,
            false,
        ),
        (
            false,
            9,
            StopReason::Disallowed(RecoveryDenial::EffectSettled),
            0,
            false,
        ),
    ] {
        let store = Arc::new(MemoryStore::new());
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Execute, Submit>().unwrap();
        builder.register_handler::<StandardRecovery>().unwrap();
        builder.register_handler::<RetryUnknown>().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(Number { value: 1 }, move |_, _, _| {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Ok(EffectAdapterOutcome::<Number>::Pending) })
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([92; 32]));
        let entry = EntryPointId::new("mfm.test/mismatched-failure@1").unwrap();
        let program = if reason == StopReason::Exhausted(RecoveryLimit::Run) {
            expand_program(
                entry,
                &Flow,
                &Number { value: 9 },
                ProgramLimits::new(run_limit),
            )
        } else {
            expand_program(
                entry,
                &StopFlow,
                &Number { value: 9 },
                ProgramLimits::new(run_limit),
            )
        }
        .unwrap();
        let pending = runtime
            .start(run.clone(), program, Number { value: 9 })
            .await
            .unwrap();
        let loaded = store.load_run(&run, None).await.unwrap().unwrap();
        let latest = decode_frame(loaded.latest()).unwrap();
        let mut payload: serde_json::Value =
            serde_json::from_slice(latest.payload().as_bytes()).unwrap();
        let mut effect = payload["operation"]["effect_prepared"].clone();
        if wrong_visit {
            effect["call"]["position"]["visit"] = 1.into();
        }
        effect["call"]["input"] = serde_json::to_value(
            mfm_values::Object::from_value(&Number {
                value: reported_input,
            })
            .unwrap(),
        )
        .unwrap();
        let original =
            mfm_values::Object::from_value(&Cause::Timeout { deadline_ms: 5000 }).unwrap();
        let request = match reason {
            StopReason::Requested => RecoveryRequest::Stop,
            StopReason::Disallowed(
                RecoveryDenial::EffectBarrier | RecoveryDenial::CheckpointUnavailable,
            )
            | StopReason::Exhausted(RecoveryLimit::StateRestart) => {
                serde_json::from_str(r#"{"restart":0}"#).unwrap()
            }
            _ => RecoveryRequest::RetryState,
        };
        payload["operation"] = serde_json::json!({"recovered": {
            "failure": {"pending_effect": {"effect": effect, "original": original}},
            "classification": Classification::OutcomeUnknown,
            "request": request,
            "outcome": RecoveryOutcome::Stop { reason, root: None },
        }});
        let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&payload).unwrap(),
        )
        .unwrap();
        let frame = seal_frame(
            &run,
            pending.head_sequence() + 1,
            Some(pending.head_digest()),
            &payload,
        )
        .unwrap();
        assert_eq!(
            store.append_run(&frame).await.unwrap(),
            mfm_store::AppendResult::Inserted
        );
        let observed = runtime.read(&run).await;
        if !wrong_visit && reported_input != 9 {
            assert!(observed.is_ok());
            assert!(runtime.resume(&run).await.is_err());
        } else if valid {
            assert!(
                matches!(observed.unwrap().state(), RunViewState::EffectPending {
                    latest_failure: Some(failure), ..
                } if failure.1 == RecoveryOutcome::Stop { reason, root: None })
            );
        } else {
            assert!(observed.is_err());
        }
        if valid && reason == StopReason::Requested {
            let mut payload: serde_json::Value =
                serde_json::from_slice(frame.payload().as_bytes()).unwrap();
            payload["operation"]["recovered"]["outcome"]["stop"]["root"] =
                serde_json::to_value(mfm_values::Object::from_value(&Number { value: 9 }).unwrap())
                    .unwrap();
            let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
                &serde_json::to_string(&payload).unwrap(),
            )
            .unwrap();
            let forged = seal_frame(
                &run,
                frame.run_sequence() + 1,
                Some(frame.head_digest()),
                &payload,
            )
            .unwrap();
            assert_eq!(
                store.append_run(&forged).await.unwrap(),
                mfm_store::AppendResult::Inserted
            );
            assert!(runtime.read(&run).await.is_err());
            assert!(runtime.resume(&run).await.is_err());
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn competing_pending_failures_report_only_the_winning_exact_head_candidate() {
    let store = Arc::new(scripted_store::ScriptedStore::recording());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_effect::<Execute, Submit>().unwrap();
    builder.register_handler::<StandardRecovery>().unwrap();
    let seen = Arc::clone(&calls);
    builder
        .register_effect_adapter::<Submit, _, _>(Number { value: 1 }, move |_, _, _| {
            let attempt = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let barrier = Arc::clone(&barrier);
            Box::pin(async move {
                if attempt == 0 {
                    return Ok(EffectAdapterOutcome::<Number>::Pending);
                }
                barrier.wait().await;
                Err(AdapterError::Operational(Cause::Timeout {
                    deadline_ms: 5000 + attempt as u64,
                }))
            })
        })
        .unwrap();
    let runtime = Arc::new(Runtime::new(builder.finish(), store.clone()));
    let run = RunId::from_digest(DigestBytes::from_array([93; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/competing-failures@1").unwrap(),
        &StopFlow,
        &Number { value: 9 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let pending = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .unwrap();
    assert_eq!(pending.head_sequence(), 2);
    let left = {
        let runtime = Arc::clone(&runtime);
        let run = run.clone();
        tokio::spawn(async move { runtime.resume(&run).await })
    };
    let right = {
        let runtime = Arc::clone(&runtime);
        let run = run.clone();
        tokio::spawn(async move { runtime.resume(&run).await })
    };
    let mut stopped = 0;
    let mut excluded = 0;
    let mut observed = Vec::new();
    for result in [left.await.unwrap(), right.await.unwrap()] {
        let view = match result {
            Ok(winner) => winner,
            Err(InvocationFailure::RecoveryStopped { observed, .. }) => {
                stopped += 1;
                let RunViewState::EffectPending {
                    latest_failure: Some(failure),
                    ..
                } = observed.state()
                else {
                    panic!("audited failure")
                };
                assert!(matches!(
                    failure.0.decode::<Cause>().unwrap(),
                    Cause::Timeout {
                        deadline_ms: 5001 | 5002
                    }
                ));
                observed
            }
            Err(InvocationFailure::Execution {
                error: RuntimeError::Recording { failure, .. },
                last_observed: Some(view),
                ..
            }) => {
                let mfm_runtime::RecordingFailure::NotInserted {
                    original: Some(original),
                    candidate,
                    observation: Some((head, mfm_runtime::CandidatePresence::Excluded)),
                    reload_cause: None,
                } = failure.as_ref()
                else {
                    panic!("excluded candidate retains original")
                };
                assert!(matches!(
                    original.original().decode::<Cause>().ok(),
                    Some(Cause::Timeout {
                        deadline_ms: 5001 | 5002
                    })
                ));
                assert_eq!(candidate.run_sequence(), 3);
                assert_eq!(head.head_sequence(), view.head_sequence());
                excluded += 1;
                view
            }
            Err(_) => panic!("unexpected execution failure"),
        };
        assert!(matches!(view.head_sequence(), 3 | 4));
        observed.push(view);
    }
    assert_eq!(stopped, 1);
    assert_eq!(excluded, 1);
    assert!(observed.iter().any(|view| view.head_sequence() == 4));
    assert_eq!(runtime.read(&run).await.unwrap().head_sequence(), 4);
    assert_eq!(original_count(&store.snapshot()), 1);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
}

fn original_count(frames: &[Vec<u8>]) -> usize {
    frames
        .iter()
        .filter(|bytes| {
            let frame = decode_frame(bytes).unwrap();
            let payload: serde_json::Value =
                serde_json::from_slice(frame.payload().as_bytes()).unwrap();
            payload["operation"].get("failed").is_some()
        })
        .count()
}
