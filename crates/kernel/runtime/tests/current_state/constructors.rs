use super::*;
use mfm_values::{MfmValue as ValueContract, SchemaDescriptor};

#[derive(Debug, Serialize, thiserror::Error)]
enum RangeError {
    #[error("input below its minimum")]
    Below { minimum: u64, actual: u64 },
    #[error("input above its maximum")]
    Above { maximum: u64, actual: u64 },
}
#[derive(Debug, Serialize)]
struct CheckedInput {
    value: u64,
}
impl CheckedInput {
    fn new(value: u64) -> Result<Self, RangeError> {
        if value == 0 {
            Err(RangeError::Below {
                minimum: 1,
                actual: value,
            })
        } else if value > 10 {
            Err(RangeError::Above {
                maximum: 10,
                actual: value,
            })
        } else {
            Ok(Self { value })
        }
    }
}
impl<'de> Deserialize<'de> for CheckedInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Request::deserialize(deserializer)?;
        Self::new(raw.value).map_err(serde::de::Error::custom)
    }
}
impl ValueContract for CheckedInput {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Request::schema_descriptor()
    }
    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        Request::semantic_id()
    }
    fn decode_native(bytes: &[u8]) -> Result<Self, NativeCause> {
        let raw: Request = serde_json::from_slice(bytes)
            .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))?;
        Self::new(raw.value).map_err(NativeCause::from_error)
    }
}
static EVALUATIONS: AtomicUsize = AtomicUsize::new(0);
struct Checked;
impl State for Checked {
    type Input = CheckedInput;
    type Output = CheckedInput;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-checked@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Checked {
    fn evaluate(
        input: CheckedInput,
    ) -> Result<ProposedStateOutcome<CheckedInput, Never>, NativeCause> {
        EVALUATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
struct CheckedFlow;
impl Operation for CheckedFlow {
    type Input = CheckedInput;
    type Output = CheckedInput;
    type Failure = Never;
    fn validate_input(&self, _: &CheckedInput) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<CheckedInput, CheckedInput, Never>,
    ) -> mfm_program::Result<()> {
        body.pure::<Checked, Identity<Never>>(NoParams, Occurrence::new())
    }
}
#[tokio::test]
async fn native_constructor_causes_reach_the_callback_boundary_without_a_parallel_codec() {
    EVALUATIONS.store(0, Ordering::SeqCst);
    for value in [0, 11] {
        // Construct a structurally valid but natively invalid fixture to exercise operation entry.
        let input = CheckedInput { value };
        let program = mfm_program::expand_program(
            EntryPointId::new("mfm.test/current-checked@1").unwrap(),
            &CheckedFlow,
            &input,
            ProgramLimits::new(0),
        )
        .unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_pure::<Checked>().unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([150 + value as u8; 32]));
        let error = runtime
            .start(run.clone(), program, input)
            .await
            .err()
            .unwrap();
        let InvocationFailure::Execution {
            error,
            last_observed: Some(observed),
            ..
        } = error
        else {
            panic!("native constructor error")
        };
        let RuntimeError::Native {
            stage: mfm_runtime::Stage::Decode,
            cause,
            ..
        } = &error
        else {
            panic!("decode stage")
        };
        assert!(matches!(
            (value, cause.downcast_ref::<RangeError>()),
            (
                0,
                Some(RangeError::Below {
                    minimum: 1,
                    actual: 0
                })
            ) | (
                11,
                Some(RangeError::Above {
                    maximum: 10,
                    actual: 11
                })
            )
        ));
        let projection =
            serde_json::from_str::<serde_json::Value>(error.project().unwrap().get()).unwrap();
        assert_eq!(
            projection["native"]["cause"][if value == 0 { "Below" } else { "Above" }]["actual"],
            value
        );
        assert_eq!(observed.head_sequence(), 1);
        // Inspection checks canonical/schema contracts; it does not eagerly materialize native input.
        assert_eq!(
            runtime.read(&run).await.unwrap().head_digest(),
            observed.head_digest()
        );
        assert!(runtime.resume(&run).await.is_err());
    }
    assert_eq!(EVALUATIONS.load(Ordering::SeqCst), 0);
}
