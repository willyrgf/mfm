use super::*;
use mfm_runtime::FailureCauseView;

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
static MAP_AVAILABLE: AtomicBool = AtomicBool::new(false);
struct Reject;
impl State for Reject {
    type Input = Input;
    type Output = Input;
    type Failure = Rejected;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-reject@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Reject {
    fn evaluate(_: Input) -> Result<ProposedStateOutcome<Input, Rejected>, NativeCause> {
        EVALUATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(ProposedStateOutcome::Failure {
            failure: Rejected { code: 71 },
        })
    }
}
struct RootMap;
impl mfm_program::ValueMap for RootMap {
    type Input = Rejected;
    type Output = Rejected;
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-root-map@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn apply(_: &NoParams, original: Rejected) -> Result<Rejected, NativeCause> {
        if MAP_AVAILABLE.load(Ordering::SeqCst) {
            Ok(Rejected {
                code: original.code + 1,
            })
        } else {
            Err(NativeCause::from_error(PolicyUnavailable {
                operation: "map_root",
            }))
        }
    }
}
struct RejectFlow;
impl mfm_program::Operation for RejectFlow {
    type Input = Input;
    type Output = Input;
    type Failure = Rejected;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, Rejected>,
    ) -> mfm_program::Result<()> {
        body.pure::<Reject, RootMap>(NoParams, Occurrence::new())
    }
}

#[tokio::test]
async fn terminal_mapping_failure_leaves_the_original_committed_and_resume_maps_without_evaluation()
{
    EVALUATIONS.store(0, Ordering::SeqCst);
    MAP_AVAILABLE.store(false, Ordering::SeqCst);
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_pure::<Reject>().unwrap();
    builder.register_map::<RootMap>().unwrap();
    let runtime = Runtime::new(builder.finish(), store.clone());
    let input = Input {
        value: 9,
        continuation: "terminal original custody".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-terminal@1").unwrap(),
        &RejectFlow,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([158; 32]));
    let error = runtime
        .start(run.clone(), program, input)
        .await
        .err()
        .unwrap();
    let InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        last_observed: Some(view),
        ..
    } = error
    else {
        panic!("native root mapping failure")
    };
    assert_eq!(
        cause.downcast_ref::<PolicyUnavailable>().unwrap().operation,
        "map_root"
    );
    assert_eq!(view.head_sequence(), 2);
    let RunViewState::AwaitingRecovery { failure } = view.state() else {
        panic!("committed original")
    };
    assert_eq!(failure.original().decode::<Rejected>().unwrap().code, 71);
    assert_eq!(runtime.read(&run).await.unwrap().head_sequence(), 2);
    MAP_AVAILABLE.store(true, Ordering::SeqCst);
    let stopped = runtime.resume(&run).await.unwrap();
    assert_eq!(stopped.head_sequence(), 3);
    let RunViewState::Failed(report) = stopped.state() else {
        panic!("terminal mapped report")
    };
    let FailureCauseView::Domain { original, root } = report.cause() else {
        panic!("domain report")
    };
    assert_eq!(original.decode::<Rejected>().unwrap().code, 71);
    assert_eq!(root.decode::<Rejected>().unwrap().code, 72);
    assert_eq!(report.reason(), &mfm_program::StopReason::Requested);
    let cold = runtime.read(&run).await.unwrap();
    let RunViewState::Failed(cold_report) = cold.state() else {
        panic!("cold report")
    };
    assert_eq!(report.canonical_bytes(), cold_report.canonical_bytes());
    assert_eq!(
        runtime.resume(&run).await.unwrap().head_digest(),
        stopped.head_digest()
    );
    assert_eq!(EVALUATIONS.load(Ordering::SeqCst), 1);
}
