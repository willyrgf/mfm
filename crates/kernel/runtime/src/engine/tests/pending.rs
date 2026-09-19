use super::*;
use tokio::sync::Notify;

// A provider controls readiness. Runtime must await it while retaining the prepared command.
#[derive(Clone)]
struct Waiting {
    calls: Arc<Mutex<Vec<EffectId>>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl ProgramEnvironment for Waiting {
    type Sources = mfm_program::Operation<Flow<false>, Policy>;
}
impl CapabilityFamily<Mutation> for Waiting {
    type Implementations = (Native,);
}
impl Resolve<bool, Mutation> for Waiting {
    fn implementation(_: &bool) -> mfm_program::Result<StableId> {
        Ok(Native::implementation_id()?)
    }
}
impl BindEffect<Mutation, Native> for Waiting {
    type Adapter = Self;
    fn bind_effect(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl EffectAdapter<NoParams, NativeReceipt, Outage> for Waiting {
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a NoParams,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<
                        EffectAdapterOutcome<NativeReceipt>,
                        AdapterError<Outage>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let attempt = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(effect.clone());
                calls.len()
            };
            self.entered.notify_one();
            self.release.notified().await;
            Ok(if attempt == 1 {
                EffectAdapterOutcome::Pending
            } else {
                EffectAdapterOutcome::Settled(NativeReceipt { accepted: true })
            })
        })
    }
}

#[tokio::test]
async fn automatic_pending_waits_and_cancellation_preserves_the_exact_command_for_cold_resume() {
    for cancel in [false, true] {
        let resources = Waiting {
            calls: Arc::new(Mutex::new(Vec::new())),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        let store = Arc::new(MemoryStore::new());
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/automatic-pending@1").unwrap(),
            &mfm_program::Operation::<Flow<false>, Policy>::default(),
            &NoParams,
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([80 + u8::from(cancel); 32]));
        let executing_store = store.clone();
        let executing_run = run.clone();
        let executing_program = program.clone();
        let task = tokio::spawn(async move {
            Runtime::new(executing_store)
                .execute(executing_run, &executing_program, &NoParams)
                .await
        });
        resources.entered.notified().await;
        resources.release.notify_one();
        resources.entered.notified().await;
        assert!(!task.is_finished());
        let runtime = Runtime::new(store.clone());
        let waiting = runtime.read(&run, &program).await.unwrap();
        assert_eq!(waiting.head_sequence(), 2);
        let RunViewState::EffectPending {
            effect,
            latest_failure: None,
        } = waiting.state()
        else {
            panic!("waiting must retain prepared authority without inventing a failure")
        };
        {
            let calls = resources.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|call| call == effect.effect_id()));
        }
        if cancel {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
            assert_eq!(
                runtime.read(&run, &program).await.unwrap().head_digest(),
                waiting.head_digest()
            );
        } else {
            resources.release.notify_one();
            task.await
                .unwrap()
                .unwrap()
                .success()
                .unwrap()
                .decode::<NoParams>()
                .unwrap();
        }
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let cold_resources = Resources {
            calls: resources.calls.clone(),
            operational: false,
            settled: true,
        };
        let cold = mfm_program::load(document.canonical_bytes(), &cold_resources).unwrap();
        let completed = Runtime::new(store).resume(&run, &cold).await.unwrap();
        completed.success().unwrap().decode::<NoParams>().unwrap();
        assert_eq!(completed.head_sequence(), 4);
        let calls = resources.calls.lock().unwrap();
        assert_eq!(calls.len(), if cancel { 3 } else { 2 });
        assert!(calls.iter().all(|call| call == effect.effect_id()));
    }
}
