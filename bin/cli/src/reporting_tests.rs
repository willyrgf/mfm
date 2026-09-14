use super::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{Never, NoParams, Operation, OperationExpansion, ProgramLimits};
use mfm_runtime::{Runtime, RuntimeAssemblyBuilder};
use std::io;
use std::sync::Arc;

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
            view_to(output, view, &mut stdout, &mut stderr),
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
                assert!(text.contains("canonical=null\n"));
                assert!(text.contains(&format!("value_ref={}\n", expected["state"]["value_ref"])));
            }
        }
    }
}

#[tokio::test]
async fn stdout_write_and_flush_failure_preserve_original_report_and_observed_head() {
    for (mut stdout, stage, code) in [
        (FailingWriter::write_failure(3), "write", 32),
        (FailingWriter::flush_failure(), "flush", 5),
    ] {
        let view = view().await;
        let expected = serde_json::to_value(SerializableRunView::new(&view).unwrap()).unwrap();
        let head = view.head_sequence();
        let mut stderr = Vec::new();
        assert_eq!(
            view_to(OutputFormat::Json, view, &mut stdout, &mut stderr),
            ExitCode::from(2)
        );
        let report: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(report["code"], "report_render_failed");
        assert_eq!(report["last_observed"]["head_sequence"], head);
        assert!(report.get("acknowledged").is_none());
        assert_eq!(report["original_report"], expected);
        assert_eq!(report["diagnostic"]["code"], "output_io");
        assert_eq!(report["diagnostic"]["details"]["stream"], "stdout");
        assert_eq!(report["diagnostic"]["details"]["stage"], stage);
        assert_eq!(report["diagnostic"]["details"]["details"]["os_code"], code);
        if stage == "write" {
            assert_eq!(stdout.bytes.len(), 3);
        } else {
            assert_eq!(stdout.bytes.last(), Some(&b'\n'));
        }
    }
}

#[tokio::test]
async fn stdout_text_failure_keeps_the_already_formatted_buffer() {
    let view = view().await;
    let expected = run_text(&view).unwrap();
    let mut stdout = FailingWriter::flush_failure();
    let mut stderr = Vec::new();
    assert_eq!(
        view_to(OutputFormat::Text, view, &mut stdout, &mut stderr),
        ExitCode::from(2)
    );
    let text = String::from_utf8(stderr).unwrap();
    assert!(text.contains(&format!("original_text={expected}")));
    assert!(text.contains("\"stream\":\"stdout\""));
    assert!(text.contains("\"stage\":\"flush\""));
}

#[tokio::test]
async fn stderr_failure_ends_both_normal_and_final_presentations_without_retry() {
    let mut stderr = FailingWriter::write_failure(0);
    let error = RunRequestError::Request(mfm_app::RequestError::RunAppendNotInserted);
    assert_eq!(
        run_error_to(OutputFormat::Json, error, &mut stderr),
        ExitCode::from(2)
    );
    assert_eq!(stderr.writes, 1);
    let mut stdout = FailingWriter::write_failure(0);
    let mut stderr = FailingWriter::write_failure(0);
    assert_eq!(
        view_to(OutputFormat::Json, view().await, &mut stdout, &mut stderr),
        ExitCode::from(2)
    );
    assert_eq!(stdout.writes, 1);
    assert_eq!(stderr.writes, 1);
}

#[tokio::test]
async fn normal_encoding_failure_reports_unavailable_original_without_stdout() {
    let view = view().await;
    let cause = mfm_canonical::JsonError::new(serde_json::from_str::<bool>("bad").unwrap_err());
    let diagnostic = InvocationDiagnostic::from_fields("json_error", "emit_run_view", &cause, None);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        json_view_to(
            &view,
            Err(diagnostic),
            "emit_run_view",
            &mut stdout,
            &mut stderr
        ),
        ExitCode::from(2)
    );
    assert!(stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
    assert!(report["original_report"].is_null());
    assert_eq!(
        report["last_observed"]["head_sequence"],
        view.head_sequence()
    );
    assert_eq!(report["diagnostic"]["code"], "json_error");
}

#[test]
fn output_custom_source_is_retained_and_cycles_stop_before_duplicate_layers() {
    #[derive(Debug)]
    struct Cycle;
    impl std::fmt::Display for Cycle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("exposed custom source")
        }
    }
    impl std::error::Error for Cycle {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self)
        }
    }
    struct Writer;
    impl Write for Writer {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other(Cycle))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let error =
        write_output(&mut Writer, OutputStream::Stdout, [b"report".as_slice()]).unwrap_err();
    let report = serde_json::to_value(error).unwrap();
    assert_eq!(report["details"]["message"], "exposed custom source");
    assert_eq!(report["details"]["os_kind"], "Other");
    assert!(report["details"]["os_code"].is_null());
    assert_eq!(
        report["details"]["sources"],
        serde_json::json!([{"message": "exposed custom source"}])
    );
    assert_eq!(report["details"]["source_cycle"], true);
}
