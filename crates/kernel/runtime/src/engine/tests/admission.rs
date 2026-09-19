use super::*;
use serde::ser::SerializeStruct;
use std::sync::atomic::{AtomicUsize, Ordering};

static ENCODINGS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    value: u64,
}
impl Serialize for Input {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        ENCODINGS.fetch_add(1, Ordering::SeqCst);
        match self.value {
            1 => return Err(serde::ser::Error::custom("admission encoder failure")),
            2 => panic!("admission panic payload marker"),
            _ => {}
        }
        let mut state = serializer.serialize_struct("Input", 1)?;
        state.serialize_field("value", &self.value)?;
        state.end()
    }
}
struct Installed;
impl ProgramEnvironment for Installed {
    type Sources = Identity<Input>;
}

#[tokio::test]
async fn borrowed_admission_encodes_once_and_rejects_encoding_faults_before_append() {
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/borrowed-encoding@1").unwrap(),
        &Identity::<Input>::default(),
        &Input { value: 7 },
        &Installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store.clone());
    for (case, value) in [7, 1, 2].into_iter().enumerate() {
        let run = RunId::from_digest(DigestBytes::from_array([70 + case as u8; 32]));
        let input = Input { value };
        let before = ENCODINGS.load(Ordering::SeqCst);
        let result = runtime.execute(run.clone(), &program, &input).await;
        assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
        assert_eq!(input.value, value);
        if value == 7 {
            assert_eq!(
                result
                    .unwrap()
                    .success()
                    .unwrap()
                    .decode::<Input>()
                    .unwrap()
                    .value,
                7
            );
        } else {
            let Err(InvocationFailure::Execution {
                run_id,
                error,
                last_observed: None,
            }) = result
            else {
                panic!("encoder failure must precede admission")
            };
            assert_eq!(run_id, run);
            let RuntimeError::Native {
                operation: Operation::Admission,
                stage: Stage::Encode,
                cause,
            } = error
            else {
                panic!("retain actual admission encoding phase")
            };
            let diagnostic = serde_json::to_string(&cause).unwrap();
            if value == 1 {
                assert!(diagnostic.contains("admission encoder failure"));
            } else {
                assert!(diagnostic.contains("panicked"));
                assert!(!diagnostic.contains("admission panic payload marker"));
            }
            assert!(store.load_run(&run, None).await.unwrap().is_none());
        }
    }
}
