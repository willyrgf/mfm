use super::*;

#[tokio::test]
async fn pending_retry_and_exhausted_stop_are_audited_without_changing_command_authority() {
    let store = Arc::new(MemoryStore::new());
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
    assert_eq!(first.head_sequence(), 3);
    let RunViewState::EffectPending {
        position,
        effect_id,
        latest_failure: Some(failure),
    } = first.state()
    else {
        panic!("pending retry")
    };
    assert_eq!(failure.decision, PendingDecision::Retry);
    assert!(matches!(
        failure.incident.error.decode::<Cause>().unwrap(),
        Cause::Timeout { deadline_ms: 5000 }
    ));
    assert_eq!(
        failure
            .incident
            .state_context
            .decode::<Number>()
            .unwrap()
            .value,
        9
    );
    let cold = Runtime::new(build(), store.clone());
    let reread = cold.read(&run).await.unwrap();
    assert_eq!(reread.head_digest(), first.head_digest());
    assert_eq!(calls.lock().unwrap().len(), 1);
    let InvocationFailure::RecoveryStopped {
        observed, reason, ..
    } = cold.resume(&run).await.err().unwrap()
    else {
        panic!("stopped")
    };
    assert_eq!(reason, StopReason::Exhausted(RecoveryLimit::StateRetry));
    assert_eq!(observed.head_sequence(), 4);
    let RunViewState::EffectPending {
        position: stopped_position,
        effect_id: stopped_effect,
        latest_failure: Some(failure),
    } = observed.state()
    else {
        panic!("pending stop")
    };
    assert_eq!(stopped_position, position);
    assert_eq!(stopped_effect, effect_id);
    assert_eq!(
        failure.decision,
        PendingDecision::Stop {
            reason: StopCode::StateRetryExhausted
        }
    );
    let error = cold.resume(&run).await.err().unwrap();
    assert!(matches!(
        error,
        InvocationFailure::Execution {
            error: RuntimeError::SizeLimit {
                resource: SizeResource::PendingFailures,
                ..
            },
            ..
        }
    ));
    assert_eq!(calls.lock().unwrap().len(), 2);
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0], calls[1]);
    }
    let retained = cold.read(&run).await.unwrap();
    assert_eq!(retained.head_digest(), observed.head_digest());
    let history =
        JournalHistory::qualify(&run, store.load_run(&run).await.unwrap().unwrap()).unwrap();
    assert_eq!(
        history
            .records()
            .filter(|record| matches!(record, JournalRecord::EffectAdapterFailed { .. }))
            .count(),
        2
    );
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
    let InvocationFailure::RecoveryStopped {
        observed, reason, ..
    } = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .err()
        .unwrap()
    else {
        panic!("standard stop")
    };
    assert_eq!(reason, StopReason::Requested);
    assert_eq!(observed.head_sequence(), 3);
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
        failure.decision,
        PendingDecision::Stop {
            reason: StopCode::Requested
        }
    );
    assert!(matches!(
        failure.incident.error.decode::<Cause>().unwrap(),
        Cause::Timeout { deadline_ms: 5000 }
    ));
    assert_eq!(calls.lock().unwrap().len(), 1);
    let settled = cold.resume(&run).await.unwrap();
    assert_eq!(settled.head_sequence(), 4);
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
                    RunViewState::EffectPending {
                        latest_failure: Some(_),
                        ..
                    }
                ));
            }
            _ => {
                let InvocationFailure::Execution {
                    error: RuntimeError::Store(mfm_store::StoreError::Indeterminate),
                    last_observed: Some(observed),
                    ..
                } = result.err().unwrap()
                else {
                    panic!("indeterminate failure append")
                };
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
        let RunViewState::EffectPending {
            effect_id,
            latest_failure,
            ..
        } = cold.state()
        else {
            panic!("retained command")
        };
        assert_eq!(latest_failure.is_some(), committed);
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
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        Option<mfm_journal::StoredRunBytes>,
                        mfm_store::StoreError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run)
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
        let RunViewState::EffectPending {
            effect_id,
            latest_failure,
            ..
        } = retained.state()
        else {
            panic!("pending")
        };
        assert_eq!(latest_failure.is_some(), retain);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(effect_id, &calls.lock().unwrap()[0].0);
        let InvocationFailure::RecoveryStopped { observed, .. } =
            cold.resume(&run).await.err().unwrap()
        else {
            panic!("acknowledged resumed failure")
        };
        assert_eq!(observed.head_sequence(), retained.head_sequence() + 1);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
    }
}

#[tokio::test]
async fn cold_fold_rejects_failure_position_mismatch_before_adapter_entry() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_effect::<Execute, Submit>().unwrap();
    builder.register_handler::<StandardRecovery>().unwrap();
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
    let program = expand_program(
        EntryPointId::new("mfm.test/mismatched-failure@1").unwrap(),
        &StopFlow,
        &Number { value: 9 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let pending = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .unwrap();
    let RunViewState::EffectPending { position, .. } = pending.state() else {
        panic!("prepared")
    };
    let mut wrong = *position;
    wrong.visit = wrong.visit.checked_next().unwrap();
    let (error, error_ref) =
        mfm_values::canonicalize_mfm_value(&Cause::Timeout { deadline_ms: 5000 }).unwrap();
    let (context, context_ref) = mfm_values::canonicalize_mfm_value(&Number { value: 9 }).unwrap();
    let history =
        JournalHistory::qualify(&run, store.load_run(&run).await.unwrap().unwrap()).unwrap();
    let frame = history
        .encode_effect_failure(
            wrong,
            mfm_journal::JournalObject::new(&error_ref, error.as_bytes()).unwrap(),
            mfm_journal::JournalObject::new(&context_ref, context.as_bytes()).unwrap(),
            PendingDecision::Stop {
                reason: StopCode::Requested,
            },
        )
        .unwrap();
    assert_eq!(
        store.append_run(&frame).await.unwrap(),
        mfm_store::AppendResult::Inserted
    );
    assert!(matches!(
        runtime.read(&run).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::InvalidHistory,
            ..
        })
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn competing_pending_failures_report_only_the_winning_exact_head_candidate() {
    let store = Arc::new(MemoryStore::new());
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
    let mut observed = Vec::new();
    for result in [left.await.unwrap(), right.await.unwrap()] {
        let view = match result {
            Ok(winner) => winner,
            Err(InvocationFailure::RecoveryStopped {
                observed, incident, ..
            }) => {
                stopped += 1;
                let RunViewState::EffectPending {
                    latest_failure: Some(failure),
                    ..
                } = observed.state()
                else {
                    panic!("audited failure")
                };
                assert_eq!(
                    incident.error.value_ref(),
                    failure.incident.error.value_ref()
                );
                observed
            }
            Err(_) => panic!("unexpected execution failure"),
        };
        assert_eq!(view.head_sequence(), 3);
        observed.push(view);
    }
    assert_eq!(stopped, 1);
    assert_eq!(observed[0].head_digest(), observed[1].head_digest());
    let history =
        JournalHistory::qualify(&run, store.load_run(&run).await.unwrap().unwrap()).unwrap();
    assert_eq!(
        history
            .records()
            .filter(|record| matches!(record, JournalRecord::EffectAdapterFailed { .. }))
            .count(),
        1
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn undersized_failure_frame_rejects_acknowledgement_and_preserves_settlement_authority() {
    struct Undersized;
    impl Operation for Undersized {
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
            scope.effect::<Execute, Submit, Identity<Never>>(
                &Number { value: 1 },
                NoParams,
                Occurrence::new(),
                EffectBounds::new(65536, 65536, 1, 1)?,
            )
        }
    }
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_effect::<Execute, Submit>().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&calls);
    builder
        .register_effect_adapter::<Submit, _, _>(
            Number { value: 1 },
            move |id, reference, command| {
                let attempt = {
                    let mut seen = seen.lock().unwrap();
                    seen.push((id.clone(), reference.clone(), command.value));
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
    let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
    let run = RunId::from_digest(DigestBytes::from_array([97; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/failure-bound@1").unwrap(),
        &Undersized,
        &Number { value: 9 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let error = runtime
        .start(run.clone(), program, Number { value: 9 })
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        InvocationFailure::Execution {
            error: RuntimeError::SizeLimit {
                resource: SizeResource::DeclaredFrame,
                ..
            },
            ..
        }
    ));
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_sequence(), 2);
    assert!(matches!(
        cold.state(),
        RunViewState::EffectPending {
            latest_failure: None,
            ..
        }
    ));
    let settled = runtime.resume(&run).await.unwrap();
    assert_eq!(settled.head_sequence(), 3);
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
}
