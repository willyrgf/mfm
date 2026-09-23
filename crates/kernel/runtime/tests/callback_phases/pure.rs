use super::*;

struct PureFault;
impl State for PureFault {
    type Input = FaultValue;
    type Output = FaultValue;
    type Failure = custody::FaultOriginal;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("test.callback.pure@1")?)
    }
}
impl PureState for PureFault {
    fn evaluate(
        input: FaultValue,
    ) -> Result<ProposedStateOutcome<FaultValue, custody::FaultOriginal>, InvocationDiagnostic>
    {
        let mode = match input.mode {
            Fault::ExecutePanic => panic!("callback-payload-marker"),
            Fault::ExecuteFailure => {
                return Err(InvocationDiagnostic::from_fields(
                    "fixture_evaluate",
                    "evaluate",
                    &serde_json::json!({"cause": {"code": 103}}),
                    None,
                ))
            }
            Fault::OriginalEncodeFailure | Fault::OriginalEncodePanic => {
                return Ok(ProposedStateOutcome::Failure {
                    failure: custody::FaultOriginal {
                        code: if input.mode == Fault::OriginalEncodeFailure {
                            custody::OriginalFault::EncodeFailure
                        } else {
                            custody::OriginalFault::EncodePanic
                        },
                    },
                })
            }
            Fault::ResultEncodePanic => Fault::ValueEncodePanic,
            Fault::ResultEncodeFailure => Fault::ValueEncodeFailure,
            mode => mode,
        };
        Ok(ProposedStateOutcome::Success {
            output: FaultValue { mode },
        })
    }
}
#[tokio::test]
async fn pure_decode_execution_and_output_encoding_failures_preserve_admission() {
    for (mode, expected_stage, diagnostic_operation) in [
        (Fault::DecodeFailure, "decode", "decode_checked"),
        (Fault::DecodePanic, "decode", "decode"),
        (Fault::ExecutePanic, "execute", "execute"),
        (Fault::ExecuteFailure, "execute", "evaluate"),
        (Fault::ResultEncodePanic, "encode", "encode"),
        (Fault::ResultEncodeFailure, "encode", "encode"),
        (Fault::OriginalEncodeFailure, "encode", "encode_failure"),
        (Fault::OriginalEncodePanic, "encode", "encode_failure"),
    ] {
        let store = Arc::new(MemoryStore::new());
        let runtime = Runtime::new(store.clone());
        let input = FaultValue { mode };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/pure-phases@1").unwrap(),
            &mfm_program::Pure::<PureFault>::default(),
            &input,
            &Resources::<mfm_program::Pure<PureFault>>::default(),
            ProgramLimits::new(0),
        )
        .unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([183; 32]));
        let Err(InvocationFailure::Execution {
            error:
                RuntimeError::Native {
                    operation: mfm_runtime::Operation::PureEvaluate,
                    stage,
                    cause,
                },
            last_observed: Some(observed),
            ..
        }) = runtime.start(run.clone(), &program, &input).await
        else {
            panic!("Pure callback must preserve operation and observation");
        };
        assert_eq!(serde_json::to_value(stage).unwrap(), expected_stage);
        assert_eq!(cause.operation(), diagnostic_operation);
        let rendered = serde_json::to_string(&cause).unwrap();
        assert!(!rendered.contains("callback-payload-marker"));
        match mode {
            Fault::DecodeFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 73),
            Fault::ExecuteFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 103),
            Fault::ResultEncodeFailure => {
                assert!(rendered.contains("reviewed evidence encoder failure"))
            }
            Fault::OriginalEncodeFailure => {
                assert!(rendered.contains("reviewed original encoder failure"))
            }
            _ => assert_eq!(cause.code(), "task_failure"),
        }
        if matches!(
            mode,
            Fault::OriginalEncodeFailure | Fault::OriginalEncodePanic
        ) {
            let fields = cause.details().as_value();
            assert_eq!(fields["original_detail"], "unavailable");
            assert_eq!(fields["original_identity"], "unavailable");
            assert_eq!(
                fields["failure_contract"],
                serde_json::to_value(
                    mfm_program::nominal_contract_ref::<custody::FaultOriginal>().unwrap()
                )
                .unwrap()
            );
        }
        let head = store.load_run(&run, None).await.unwrap().unwrap();
        assert_eq!(head.head().head_sequence(), 1);
        assert_eq!(head.head().head_digest(), observed.head_digest());
        let cold = runtime.read(&run, &program).await.unwrap();
        assert_eq!(cold.head_digest(), observed.head_digest());
        assert!(matches!(cold.state(), RunViewState::Runnable { .. }));
    }
}
