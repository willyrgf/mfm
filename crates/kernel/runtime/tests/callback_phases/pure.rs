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
            20 => panic!("callback-payload-marker"),
            21 => {
                return Err(InvocationDiagnostic::from_fields(
                    "fixture_evaluate",
                    "evaluate",
                    &serde_json::json!({"cause": {"code": 103}}),
                    None,
                ))
            }
            30 | 31 => {
                return Ok(ProposedStateOutcome::Failure {
                    failure: custody::FaultOriginal {
                        code: if input.mode == 30 { 87 } else { 88 },
                    },
                })
            }
            22 => 13,
            23 => 14,
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
        (1, "decode", "decode_checked"),
        (2, "decode", "decode"),
        (20, "execute", "execute"),
        (21, "execute", "evaluate"),
        (22, "encode", "encode"),
        (23, "encode", "encode"),
        (30, "encode", "encode_failure"),
        (31, "encode", "encode_failure"),
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
            1 => assert_eq!(cause.details().as_value()["cause"]["code"], 73),
            21 => assert_eq!(cause.details().as_value()["cause"]["code"], 103),
            23 => assert!(rendered.contains("reviewed evidence encoder failure")),
            30 => assert!(rendered.contains("reviewed original encoder failure")),
            _ => assert_eq!(cause.code(), "task_failure"),
        }
        if mode >= 30 {
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
