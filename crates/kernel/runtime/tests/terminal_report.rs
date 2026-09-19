use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    compile, load, Classification, ClassifyError, NoParams, ProgramEnvironment, ProgramLimits,
    ProposedStateOutcome, Pure, PureState, State, StopReason,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{Failure, Runtime};
use mfm_values::{InvocationDiagnostic, Object};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[allow(dead_code)]
#[path = "support/scripted_store.rs"]
mod scripted_store;

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Rejected {
    code: u64,
}
impl ClassifyError for Rejected {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
static EVALUATIONS: AtomicUsize = AtomicUsize::new(0);
struct Reject;
impl State for Reject {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Rejected;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.current-reject@1")?)
    }
}
impl PureState for Reject {
    fn evaluate(
        _: NoParams,
    ) -> Result<ProposedStateOutcome<NoParams, Rejected>, InvocationDiagnostic> {
        EVALUATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(ProposedStateOutcome::Failure {
            failure: Rejected { code: 71 },
        })
    }
}
struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = Pure<Reject>;
}

// A terminal report retains the State's exact original. Presentation is outside Runtime and cannot
// introduce another stored root value or cause the State to run again during inspection/resumption.
#[tokio::test]
async fn terminal_original_survives_cold_inspection_and_rejects_superseded_root_fields() {
    let store = Arc::new(scripted_store::ScriptedStore::recording());
    let runtime = Runtime::new(store.clone());
    let program = compile(
        EntryPointId::new("mfm.test/current-terminal@1").unwrap(),
        &Pure::<Reject>::default(),
        &NoParams,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([158; 32]));
    let stopped = runtime
        .start(run.clone(), &program, &NoParams)
        .await
        .unwrap();
    assert_eq!(stopped.head_sequence(), 3);
    let report = stopped.failure().unwrap();
    let Failure::Domain { original, .. } = report.failure() else {
        panic!("domain report")
    };
    assert_eq!(original.decode::<Rejected>().unwrap().code, 71);
    assert_eq!(report.reason(), &StopReason::Requested);
    let document = runtime.program_document(&run).await.unwrap();
    drop(program);
    let program = load(document.canonical_bytes(), &Resources).unwrap();
    let cold = Runtime::new(store.clone());
    let inspected = cold.read(&run, &program).await.unwrap();
    assert_eq!(
        report.canonical_bytes(),
        inspected.failure().unwrap().canonical_bytes()
    );
    assert_eq!(
        cold.resume(&run, &program).await.unwrap().head_digest(),
        stopped.head_digest()
    );
    assert_eq!(EVALUATIONS.load(Ordering::SeqCst), 1);
    let frames = store.snapshot();
    let frame = mfm_journal::decode_frame(frames.last().unwrap()).unwrap();
    let record: serde_json::Value = serde_json::from_slice(frame.payload().as_bytes()).unwrap();
    for root in [
        serde_json::Value::Null,
        serde_json::to_value(Object::from_value(&NoParams).unwrap()).unwrap(),
    ] {
        let mut record = record.clone();
        record["operation"]["recovered"]["outcome"]["stop"]["root"] = root;
        let payload =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&record.to_string()).unwrap();
        let changed = mfm_journal::seal_frame(
            &run,
            frame.run_sequence(),
            frame.previous_head_digest(),
            &payload,
        )
        .unwrap();
        let mut altered = frames.clone();
        *altered.last_mut().unwrap() = changed.canonical_bytes().to_vec();
        let reader = Runtime::new(Arc::new(scripted_store::RetainedStore(altered)));
        assert!(reader.read(&run, &program).await.is_err());
        assert!(reader.resume(&run, &program).await.is_err());
    }
    assert_eq!(EVALUATIONS.load(Ordering::SeqCst), 1);
}
