use super::{CliError, OutputFormat};
use mfm_app::{
    encode_response, ReportFailure, ReportStage, RunRequestError, SerializableClientError,
    SerializableRunView, StartRunResult,
};
use mfm_runtime::RunView;
use mfm_values::NativeCause;
use serde::Serialize;
use std::io::Write;
use std::process::ExitCode;

#[path = "output.rs"]
mod output;
use output::{write_output, OutputStream};

pub(super) fn emit_start_result(
    output: OutputFormat,
    result: StartRunResult,
) -> Result<ExitCode, CliError> {
    let exit = super::run_exit(result.run());
    let encoded = match output {
        OutputFormat::Json => result
            .serializable()
            .map_err(|cause| (ReportStage::Prepare, cause))
            .and_then(|model| json_bytes(&model).map_err(|cause| (ReportStage::Encode, cause))),
        OutputFormat::Text => run_bytes(output, result.run()).map(|bytes| {
            let mut text = super::render_config_summary(result.config()).into_bytes();
            text.extend(bytes);
            text
        }),
    };
    let delivered = encoded.and_then(|bytes| {
        write_output(&mut std::io::stdout().lock(), OutputStream::Stdout, &bytes)
            .map_err(|cause| (ReportStage::Deliver, NativeCause::from_error(cause)))
    });
    match delivered {
        Ok(()) => Ok(exit),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(result, stage, cause);
            let encoded = error_bytes(output, &failure.incomplete());
            finish_failure(&mut std::io::stderr().lock(), failure, encoded)
        }
    }
}

pub(super) fn emit_run_view(output: OutputFormat, view: RunView) -> Result<ExitCode, CliError> {
    view_to(
        output,
        view,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}

fn view_to(
    output: OutputFormat,
    view: RunView,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, CliError> {
    let exit = super::run_exit(&view);
    let delivered = run_bytes(output, &view).and_then(|bytes| {
        write_output(stdout, OutputStream::Stdout, &bytes)
            .map_err(|cause| (ReportStage::Deliver, NativeCause::from_error(cause)))
    });
    match delivered {
        Ok(()) => Ok(exit),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(view, stage, cause);
            let encoded = error_bytes(output, &failure.incomplete());
            finish_failure(stderr, failure, encoded)
        }
    }
}

fn run_bytes(output: OutputFormat, view: &RunView) -> Result<Vec<u8>, (ReportStage, NativeCause)> {
    let model = SerializableRunView::new(view).map_err(|cause| (ReportStage::Prepare, cause))?;
    let encoded = match output {
        OutputFormat::Json => json_bytes(&model),
        OutputFormat::Text => (|| {
            let bytes = encode_response(&model)?;
            let root = fields(&bytes)?;
            let state = fields(field(&root, "state")?.get().as_bytes())?;
            let kind: String =
                serde_json::from_str(field(&state, "kind")?.get()).map_err(json_cause)?;
            let mut text = format!(
                "run_id={}\nhead_sequence={}\nhead_digest={}\nstate={kind}\n",
                view.run_id(),
                view.head_sequence(),
                view.head_digest(),
            );
            for (name, value) in state {
                if name != "kind" {
                    text.push_str(&format!("{name}={}\n", value.get()));
                }
            }
            Ok(text.into_bytes())
        })(),
    };
    encoded.map_err(|cause| (ReportStage::Encode, cause))
}

pub(super) fn emit_error(output: OutputFormat, error: CliError) -> Result<(), CliError> {
    error_to(output, error, &mut std::io::stderr().lock())
}

fn error_to(
    output: OutputFormat,
    error: CliError,
    stderr: &mut impl Write,
) -> Result<(), CliError> {
    match error {
        CliError::Reporting(_) => Err(error),
        CliError::RunRequest(error) => run_error_to(output, *error, stderr),
        error => {
            let message = error.message();
            let prepared = (|| {
                #[derive(Serialize)]
                struct Ordinary<'a> {
                    #[serde(flatten)]
                    error: SerializableClientError<'a>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    cause: Option<Box<serde_json::value::RawValue>>,
                }
                let cause = match &error {
                    CliError::Output(cause) => Some(
                        cause
                            .project()
                            .map_err(|cause| (ReportStage::Prepare, cause))?,
                    ),
                    _ => None,
                };
                error_bytes(
                    output,
                    &Ordinary {
                        error: SerializableClientError::new(error.code(), &message),
                        cause,
                    },
                )
                .map_err(|cause| (ReportStage::Encode, cause))
            })();
            let bytes = match prepared {
                Ok(bytes) => bytes,
                Err((stage, cause)) => {
                    return Err(terminal(ReportFailure::new(error, stage, cause)))
                }
            };
            write_output(stderr, OutputStream::Stderr, &bytes).map_err(|cause| {
                terminal(ReportFailure::new(
                    error,
                    ReportStage::Deliver,
                    NativeCause::from_error(cause),
                ))
            })
        }
    }
}

fn run_error_to(
    output: OutputFormat,
    error: RunRequestError,
    stderr: &mut impl Write,
) -> Result<(), CliError> {
    let message = error.to_string();
    let encoded = SerializableClientError::for_run(&error, &message)
        .map_err(|cause| (ReportStage::Prepare, cause))
        .and_then(|model| {
            error_bytes(output, &model).map_err(|cause| (ReportStage::Encode, cause))
        });
    match encoded {
        Ok(bytes) => write_output(stderr, OutputStream::Stderr, &bytes).map_err(|cause| {
            terminal(ReportFailure::new(
                error,
                ReportStage::Deliver,
                NativeCause::from_error(cause),
            ))
        }),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(error, stage, cause);
            let encoded = error_bytes(output, &failure.incomplete());
            finish_failure(stderr, failure, encoded).map(|_| ())
        }
    }
}

fn finish_failure<T: Send + Sync + 'static>(
    stderr: &mut impl Write,
    failure: ReportFailure<T>,
    encoded: Result<Vec<u8>, NativeCause>,
) -> Result<ExitCode, CliError> {
    let bytes = match encoded {
        Ok(bytes) => bytes,
        Err(cause) => {
            return Err(terminal(ReportFailure::new(
                failure,
                ReportStage::Encode,
                cause,
            )))
        }
    };
    match write_output(stderr, OutputStream::Stderr, &bytes) {
        Ok(()) => Ok(ExitCode::from(2)),
        Err(cause) => Err(terminal(ReportFailure::new(
            failure,
            ReportStage::Deliver,
            NativeCause::from_error(cause),
        ))),
    }
}

fn terminal<T: Send + Sync + 'static>(failure: ReportFailure<T>) -> CliError {
    CliError::Reporting(Box::new(failure))
}

fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, NativeCause> {
    let mut bytes = encode_response(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn json_cause(cause: serde_json::Error) -> NativeCause {
    NativeCause::from_error(mfm_canonical::JsonError::new(cause))
}

type Fields<'a> = std::collections::BTreeMap<&'a str, &'a serde_json::value::RawValue>;

fn fields(bytes: &[u8]) -> Result<Fields<'_>, NativeCause> {
    serde_json::from_slice(bytes).map_err(json_cause)
}

fn field<'a>(
    fields: &Fields<'a>,
    name: &'static str,
) -> Result<&'a serde_json::value::RawValue, NativeCause> {
    #[derive(Debug, Serialize, thiserror::Error)]
    #[error("prepared transport field is absent")]
    struct MissingField {
        field: &'static str,
    }
    fields
        .get(name)
        .copied()
        .ok_or_else(|| NativeCause::from_error(MissingField { field: name }))
}

fn error_bytes(output: OutputFormat, value: &impl Serialize) -> Result<Vec<u8>, NativeCause> {
    let mut bytes = encode_response(value)?;
    match output {
        OutputFormat::Json => {
            bytes.push(b'\n');
            Ok(bytes)
        }
        OutputFormat::Text => {
            let model = fields(&bytes)?;
            let message: String =
                serde_json::from_str(field(&model, "message")?.get()).map_err(json_cause)?;
            let mut text = format!("error: {message}\n");
            for (name, value) in model {
                if name != "code" && name != "message" {
                    text.push_str(&format!("{name}={}\n", value.get()));
                }
            }
            Ok(text.into_bytes())
        }
    }
}

pub(super) fn write_json_stdout(value: &impl Serialize) -> Result<(), CliError> {
    let bytes = json_bytes(value).map_err(CliError::Output)?;
    write_stdout(&bytes)
}

pub(super) fn write_stdout(bytes: &[u8]) -> Result<(), CliError> {
    write_output(&mut std::io::stdout().lock(), OutputStream::Stdout, bytes)
        .map_err(|cause| CliError::Output(NativeCause::from_error(cause)))
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod tests;
