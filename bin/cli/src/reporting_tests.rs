use super::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{Never, NoParams, Operation, OperationExpansion, ProgramLimits};
use mfm_runtime::{InvocationFailure, Runtime, RuntimeAssemblyBuilder, RuntimeError, Stage};
use std::io;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Empty;
impl Operation for Empty {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn validate_input(&self, _: &NoParams) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        _: &mut OperationExpansion<NoParams, NoParams, Never>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

fn run_id() -> RunId {
    RunId::from_digest(DigestBytes::from_array([39; 32]))
}

async fn view() -> RunView {
    let runtime = Runtime::new(
        RuntimeAssemblyBuilder::new().unwrap().finish(),
        Arc::new(mfm_store::MemoryStore::new()),
    );
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/cli-output@1").unwrap(),
        &Empty,
        &NoParams,
        ProgramLimits::new(0),
    )
    .unwrap();
    runtime.start(run_id(), program, NoParams).await.unwrap()
}

struct FailingWriter {
    bytes: Vec<u8>,
    accept: usize,
    flush_failure: bool,
    writes: usize,
}
impl FailingWriter {
    fn write_failure(accept: usize) -> Self {
        Self {
            bytes: Vec::new(),
            accept,
            flush_failure: false,
            writes: 0,
        }
    }
    fn flush_failure() -> Self {
        Self {
            bytes: Vec::new(),
            accept: usize::MAX,
            flush_failure: true,
            writes: 0,
        }
    }
}
impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.accept == 0 {
            return Err(io::Error::from_raw_os_error(32));
        }
        let count = bytes.len().min(self.accept);
        self.bytes.extend_from_slice(&bytes[..count]);
        self.accept -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.flush_failure {
            Err(io::Error::from_raw_os_error(5))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn successful_json_and_text_preserve_the_observed_value_and_exit() {
    for output in [OutputFormat::Json, OutputFormat::Text] {
        let view = view().await;
        let expected = serde_json::to_value(SerializableRunView::new(&view).unwrap()).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            view_to(output, view, &mut stdout, &mut stderr).unwrap(),
            ExitCode::SUCCESS
        );
        assert!(stderr.is_empty());
        match output {
            OutputFormat::Json => assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&stdout).unwrap(),
                expected
            ),
            OutputFormat::Text => {
                let text = String::from_utf8(stdout).unwrap();
                assert!(text.contains("state=succeeded\n"));
                assert!(text.contains("value=null\n"));
                assert!(text.contains(&format!("value_ref={}\n", expected["state"]["value_ref"])));
            }
        }
    }
}

#[tokio::test]
async fn stdout_write_and_flush_failures_report_historical_head_without_claiming_insertion() {
    for (mut stdout, stage, code) in [
        (FailingWriter::write_failure(3), "write", 32),
        (FailingWriter::flush_failure(), "flush", 5),
    ] {
        let view = view().await;
        let head = view.head_sequence();
        let mut stderr = Vec::new();
        assert_eq!(
            view_to(OutputFormat::Json, view, &mut stdout, &mut stderr).unwrap(),
            ExitCode::from(2)
        );
        let report: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(report["code"], "report_render_failed");
        assert_eq!(report["last_observed"]["head_sequence"], head);
        assert!(report.get("acknowledged").is_none());
        assert_eq!(report["report_failure"]["stage"], "deliver");
        assert_eq!(
            report["report_failure"]["omissions"][0]["reason"],
            "delivery_failed"
        );
        assert_eq!(report["report_failure"]["cause"]["stream"], "stdout");
        assert_eq!(report["report_failure"]["cause"]["stage"], stage);
        assert_eq!(report["report_failure"]["cause"]["cause"]["os_code"], code);
        if stage == "write" {
            assert_eq!(stdout.bytes.len(), 3);
        } else {
            assert_eq!(stdout.bytes.last(), Some(&b'\n'));
        }
    }
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("reviewed original")]
struct Cause {
    code: u64,
}

#[test]
fn failed_stderr_retains_the_original_invocation_and_its_acknowledged_head() {
    let frame = mfm_journal::seal_frame(
        &run_id(),
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let acknowledged = mfm_store::RunSummary::new(
        run_id(),
        1,
        frame.head_digest().clone(),
        frame.canonical_bytes().len() as u64,
    )
    .unwrap();
    let expected = acknowledged.clone();
    let original = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: run_id(),
        last_observed: None,
        error: RuntimeError::Projection {
            acknowledged: Box::new(acknowledged),
            cause: NativeCause::from_error(Cause { code: 71 }),
        },
    });
    let mut stderr = FailingWriter::write_failure(0);
    let CliError::Reporting(failure) =
        run_error_to(OutputFormat::Json, original, &mut stderr).unwrap_err()
    else {
        panic!("terminal custody")
    };
    let failure = failure
        .downcast_ref::<ReportFailure<RunRequestError>>()
        .unwrap();
    let RunRequestError::Invocation(InvocationFailure::Execution {
        error: RuntimeError::Projection {
            acknowledged,
            cause,
        },
        ..
    }) = failure.original()
    else {
        panic!("known insertion retained")
    };
    assert_eq!(acknowledged.as_ref(), &expected);
    assert_eq!(cause.downcast_ref::<Cause>().unwrap().code, 71);
    let delivery: serde_json::Value =
        serde_json::from_str(failure.cause().project().unwrap().get()).unwrap();
    assert_eq!(delivery["stream"], "stderr");
    assert_eq!(stderr.writes, 1);
}

#[test]
fn failed_delivery_of_incomplete_report_retains_both_failures_without_projector_retry() {
    #[derive(Debug, thiserror::Error)]
    #[error("reviewed original projection failed")]
    struct Original(Arc<AtomicUsize>);
    let attempts = Arc::new(AtomicUsize::new(0));
    let original = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: run_id(),
        last_observed: None,
        error: RuntimeError::Native {
            operation: mfm_runtime::Operation::ReadPrepare,
            stage: Stage::Decode,
            cause: NativeCause::from_error_with(Original(attempts.clone()), |original| {
                original.0.fetch_add(1, Ordering::SeqCst);
                Err(NativeCause::from_error(Cause { code: 72 }))
            }),
        },
    });
    let mut stderr = FailingWriter::write_failure(0);
    let CliError::Reporting(failure) =
        run_error_to(OutputFormat::Json, original, &mut stderr).unwrap_err()
    else {
        panic!("terminal custody")
    };
    let failure = failure
        .downcast_ref::<ReportFailure<ReportFailure<RunRequestError>>>()
        .unwrap();
    assert_eq!(
        failure
            .original()
            .cause()
            .downcast_ref::<Cause>()
            .unwrap()
            .code,
        72
    );
    let RunRequestError::Invocation(InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        ..
    }) = failure.original().original()
    else {
        panic!("original custody")
    };
    assert!(cause.downcast_ref::<Original>().is_some());
    assert!(failure
        .cause()
        .downcast_ref::<output::OutputWriteError>()
        .is_some());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(stderr.writes, 1);
}

#[test]
fn output_owner_retains_reviewed_nested_os_facts_and_withholds_custom_input() {
    #[derive(Debug, thiserror::Error)]
    #[error("secret-shaped custom output input")]
    struct Secret(#[source] io::Error);
    struct Writer;
    impl Write for Writer {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::Other,
                Secret(io::Error::from_raw_os_error(13)),
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            panic!("write failed before flush")
        }
    }
    let error = write_output(
        &mut Writer,
        OutputStream::Stdout,
        b"rejected private buffer",
    )
    .unwrap_err();
    let native = NativeCause::from_error(error);
    let projection = native.project().unwrap();
    let json = projection.get();
    assert!(!json.contains("secret-shaped"));
    assert!(!json.contains("rejected private buffer"));
    assert!(!format!("{native:?}").contains("secret-shaped"));
    let report: serde_json::Value = serde_json::from_str(json).unwrap();
    let sources = &report["cause"]["sources"];
    assert_eq!(sources["sources"]["layers"][0]["kind"], "opaque");
    assert_eq!(sources["sources"]["layers"][1]["kind"], "os");
    assert_eq!(
        sources["sources"]["layers"][1]["facts"][1]["OsCode"]["code"],
        13
    );
    assert_eq!(sources["omissions"][0]["reason"], "withheld");
}

#[test]
fn ordinary_output_projection_failure_retains_its_actual_prepare_stage_without_writing() {
    let original = CliError::Output(NativeCause::from_error_with(Cause { code: 81 }, |_| {
        Err(NativeCause::from_error(Cause { code: 82 }))
    }));
    let mut stderr = Vec::new();
    let CliError::Reporting(failure) =
        error_to(OutputFormat::Json, original, &mut stderr).unwrap_err()
    else {
        panic!("terminal native reporting custody")
    };
    let failure = failure.downcast_ref::<ReportFailure<CliError>>().unwrap();
    assert_eq!(failure.stage(), ReportStage::Prepare);
    assert_eq!(failure.cause().downcast_ref::<Cause>().unwrap().code, 82);
    let CliError::Output(cause) = failure.original() else {
        panic!("original output cause")
    };
    assert_eq!(cause.downcast_ref::<Cause>().unwrap().code, 81);
    assert!(stderr.is_empty());
}
