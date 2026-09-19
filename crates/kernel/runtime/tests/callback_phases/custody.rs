use super::*;

static ENCODINGS: AtomicUsize = AtomicUsize::new(0);
static CLASSIFICATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(decode_native = "Self::decode_checked")]
pub(super) struct FaultOriginal {
    pub(super) code: u64,
}
impl Serialize for FaultOriginal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.code == 83 {
            ENCODINGS.fetch_add(1, Ordering::SeqCst);
        }
        if self.code == 87 {
            return Err(serde::ser::Error::custom(
                "reviewed original encoder failure",
            ));
        }
        assert_ne!(self.code, 88, "callback-payload-marker");
        let mut value = serializer.serialize_struct("FaultOriginal", 1)?;
        value.serialize_field("code", &self.code)?;
        value.end()
    }
}
impl FaultOriginal {
    fn decode_checked(bytes: &[u8]) -> Result<Self, InvocationDiagnostic> {
        let original: Self = serde_json::from_slice(bytes).map_err(|source| {
            InvocationDiagnostic::from_fields(
                "json_error",
                "decode_checked",
                &mfm_canonical::JsonError::new(source),
                None,
            )
        })?;
        match original.code {
            84 => Err(InvocationDiagnostic::from_fields(
                "fixture_original_decode",
                "decode_checked",
                &serde_json::json!({"cause": {"code": 107}}),
                None,
            )),
            85 => panic!("callback-payload-marker"),
            _ => Ok(original),
        }
    }
}
impl ClassifyError for FaultOriginal {
    fn classify(&self) -> Classification {
        if self.code == 83 {
            CLASSIFICATIONS.fetch_add(1, Ordering::SeqCst);
        }
        assert_ne!(self.code, 86, "callback-payload-marker");
        Classification::OutcomeUnknown
    }
}

#[tokio::test]
async fn encoded_original_is_not_classified_until_its_append_is_acknowledged() {
    for acknowledge in [false, true] {
        ENCODINGS.store(0, Ordering::SeqCst);
        CLASSIFICATIONS.store(0, Ordering::SeqCst);
        let store: Arc<dyn Store> = if acknowledge {
            Arc::new(MemoryStore::new())
        } else {
            // The Effect preparation is sequence 2; refuse its original at sequence 3.
            Arc::new(RefuseFailure {
                inner: MemoryStore::new(),
                loads: AtomicUsize::new(0),
            })
        };
        let resources = Resources::<EffectSource>::default();
        let runtime = Runtime::new(store.clone());
        let input = Input {
            value: 8,
            continuation: "original custody".into(),
        };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/callback-custody@1").unwrap(),
            &EffectSource::new(NoParams),
            &input,
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([182; 32]));
        let error = runtime
            .start(run.clone(), &program, &input)
            .await
            .err()
            .unwrap();
        if acknowledge {
            let InvocationFailure::RecoveryStopped { observed, .. } = error else {
                panic!("default Stop retains the acknowledged original")
            };
            let RunViewState::EffectPending {
                latest_failure: Some((original, _)),
                ..
            } = observed.state()
            else {
                panic!("pending authority and original must remain available")
            };
            assert_eq!(original.canonical_bytes(), br#"{"code":83}"#);
            assert_eq!(original.decode::<FaultOriginal>().unwrap().code, 83);
            assert_eq!(observed.head_sequence(), 4);
        } else {
            let InvocationFailure::Execution {
                error: RuntimeError::Recording { failure, .. },
                last_observed: Some(observed),
                ..
            } = error
            else {
                panic!("failed Store must retain invocation custody")
            };
            let mfm_runtime::RecordingFailure::Store {
                original: Some(original),
                cause: mfm_store::StoreError::Unavailable(_),
                ..
            } = failure.as_ref()
            else {
                panic!("original and failed physical append must remain distinct")
            };
            assert_eq!(original.original().canonical_bytes(), br#"{"code":83}"#);
            assert_eq!(observed.head_sequence(), 2);
        }
        let cold = runtime.read(&run, &program).await.unwrap();
        assert_eq!(cold.head_sequence(), if acknowledge { 4 } else { 2 });
        assert_eq!(
            store
                .load_run(&run, None)
                .await
                .unwrap()
                .unwrap()
                .head()
                .head_digest(),
            cold.head_digest()
        );
        assert_eq!(ENCODINGS.load(Ordering::SeqCst), 1);
        assert_eq!(
            CLASSIFICATIONS.load(Ordering::SeqCst),
            usize::from(acknowledge)
        );
    }
}

#[tokio::test]
async fn classifier_failures_preserve_the_acknowledged_original_for_cold_inspection() {
    for (code, expected_stage) in [(84, "decode"), (85, "decode"), (86, "execute")] {
        let store = Arc::new(MemoryStore::new());
        let resources = Resources::<EffectSource> {
            original: Some(code),
            ..Default::default()
        };
        let runtime = Runtime::new(store.clone());
        let input = Input {
            value: 0,
            continuation: "classification fault".into(),
        };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/classifier-phases@1").unwrap(),
            &EffectSource::new(NoParams),
            &input,
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([184; 32]));
        let Err(InvocationFailure::Execution {
            error:
                RuntimeError::Native {
                    operation: mfm_runtime::Operation::Recovery,
                    stage,
                    cause,
                },
            last_observed: Some(observed),
            ..
        }) = runtime.start(run.clone(), &program, &input).await
        else {
            panic!("classification must fail after original acknowledgement");
        };
        assert_eq!(serde_json::to_value(stage).unwrap(), expected_stage);
        let rendered = serde_json::to_string(&cause).unwrap();
        assert!(!rendered.contains("callback-payload-marker"));
        if code == 84 {
            assert_eq!(cause.details().as_value()["cause"]["code"], 107);
        } else {
            assert_eq!(cause.code(), "task_failure");
        }
        assert_eq!(observed.head_sequence(), 3);
        let cold = runtime.read(&run, &program).await.unwrap();
        assert_eq!(cold.head_digest(), observed.head_digest());
        let RunViewState::AwaitingRecovery {
            failure: Failure::PendingEffect { original, .. },
        } = cold.state()
        else {
            panic!("cold inspection retains the original without invoking its failing classifier");
        };
        assert_eq!(
            original.canonical_bytes(),
            format!("{{\"code\":{code}}}").as_bytes()
        );
        assert_eq!(
            store
                .load_run(&run, None)
                .await
                .unwrap()
                .unwrap()
                .head()
                .head_sequence(),
            3
        );
    }
}

struct RefuseFailure {
    inner: MemoryStore,
    loads: AtomicUsize,
}
impl Store for RefuseFailure {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<mfm_store::LoadedRun>, mfm_store::StoreError>,
                > + Send
                + 'a,
        >,
    > {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_run(run, probe)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<mfm_store::AppendResult, mfm_store::StoreError>>
                + Send
                + 'a,
        >,
    > {
        if frame.run_sequence() == 3 {
            Box::pin(async {
                Err(mfm_store::StoreError::Unavailable(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.store", "injected": "Unavailable"}),
                    ),
                ))
            })
        } else {
            self.inner.append_run(frame)
        }
    }
}
