use super::{CliError, OutputFormat};
use mfm_app::{
    encode_response, RunRequestError, SerializableClientError, SerializableRunView, StartRunResult,
};
use mfm_runtime::{InvocationFailure, RunView, RunViewState};
use mfm_values::InvocationDiagnostic;
use serde::Serialize;
use serde_json::value::RawValue;
use std::io::Write;
use std::process::ExitCode;

#[path = "output.rs"]
mod output;
use output::{write_output, OutputStream};

pub(super) fn emit_start_result(output: OutputFormat, result: StartRunResult) -> ExitCode {
    let stdout = &mut std::io::stdout().lock();
    let stderr = &mut std::io::stderr().lock();
    match output {
        OutputFormat::Json => {
            let encoded = result.serializable().and_then(|model| {
                encode_response(&model).map_err(|cause| {
                    InvocationDiagnostic::from_fields(
                        "json_error",
                        "emit_start_result",
                        &cause,
                        None,
                    )
                })
            });
            json_view_to(result.run(), encoded, "emit_start_result", stdout, stderr)
        }
        OutputFormat::Text => {
            let encoded = run_text(result.run())
                .map(|body| super::render_config_summary(result.config()) + &body);
            text_view_to(result.run(), encoded, "emit_start_result", stdout, stderr)
        }
    }
}

pub(super) fn emit_run_view(output: OutputFormat, view: RunView) -> ExitCode {
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
) -> ExitCode {
    match output {
        OutputFormat::Json => {
            let encoded = SerializableRunView::new(&view).and_then(|model| {
                encode_response(&model).map_err(|cause| {
                    InvocationDiagnostic::from_fields("json_error", "emit_run_view", &cause, None)
                })
            });
            json_view_to(&view, encoded, "emit_run_view", stdout, stderr)
        }
        OutputFormat::Text => text_view_to(&view, run_text(&view), "emit_run_view", stdout, stderr),
    }
}

fn json_view_to(
    view: &RunView,
    encoded: Result<Box<RawValue>, InvocationDiagnostic>,
    operation: &'static str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    match encoded {
        Ok(encoded) => match write_output(
            stdout,
            OutputStream::Stdout,
            [encoded.get().as_bytes(), b"\n"],
        ) {
            Ok(()) => return super::run_exit(view),
            Err(cause) => {
                let diagnostic =
                    InvocationDiagnostic::from_fields("output_io", operation, &cause, None);
                let terminal =
                    SerializableClientError::failed_view_report(view, &diagnostic, Some(&encoded));
                final_json(stderr, &terminal);
            }
        },
        Err(diagnostic) => {
            let terminal = SerializableClientError::failed_view_report(view, &diagnostic, None);
            final_json(stderr, &terminal);
        }
    }
    ExitCode::from(2)
}

fn text_view_to(
    view: &RunView,
    encoded: Result<String, InvocationDiagnostic>,
    operation: &'static str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    match encoded {
        Ok(text) => match write_output(stdout, OutputStream::Stdout, [text.as_bytes()]) {
            Ok(()) => return super::run_exit(view),
            Err(cause) => {
                let diagnostic =
                    InvocationDiagnostic::from_fields("output_io", operation, &cause, None);
                let terminal = SerializableClientError::failed_view_report(view, &diagnostic, None);
                final_text(stderr, &terminal, Some(&text));
            }
        },
        Err(diagnostic) => {
            let terminal = SerializableClientError::failed_view_report(view, &diagnostic, None);
            final_text(stderr, &terminal, None);
        }
    }
    ExitCode::from(2)
}

fn final_json(stderr: &mut impl Write, terminal: &SerializableClientError<'_>) {
    let encoded = encode_response(terminal);
    if let Ok(encoded) = &encoded {
        let _delivery = write_output(
            stderr,
            OutputStream::Stderr,
            [encoded.get().as_bytes(), b"\n"],
        );
    }
}

fn final_text(
    stderr: &mut impl Write,
    terminal: &SerializableClientError<'_>,
    original: Option<&str>,
) {
    // The borrowed final model contains known facts only; no normal formatter is retried.
    let encoded = encode_response(terminal);
    if let Ok(encoded) = &encoded {
        let chunks = [
            b"error: report presentation failed\ndetail=".as_slice(),
            encoded.get().as_bytes(),
            b"\noriginal_text=",
            original.unwrap_or("unavailable").as_bytes(),
            b"\n",
        ];
        let _delivery = write_output(stderr, OutputStream::Stderr, chunks);
    }
}

fn json_field(
    text: &mut String,
    name: &str,
    value: &impl Serialize,
) -> Result<(), InvocationDiagnostic> {
    let encoded = encode_response(value).map_err(|cause| {
        InvocationDiagnostic::from_fields("json_error", "json_field", &cause, None)
    })?;
    text.push_str(name);
    text.push('=');
    text.push_str(encoded.get());
    text.push('\n');
    Ok(())
}

fn run_text(view: &RunView) -> Result<String, InvocationDiagnostic> {
    let mut text = format!(
        "run_id={}\nhead_sequence={}\nhead_digest={}\n",
        view.run_id(),
        view.head_sequence(),
        view.head_digest()
    );
    match view.state() {
        RunViewState::Runnable { position, reason } => {
            text.push_str("state=runnable\n");
            json_field(&mut text, "position", position)?;
            match reason {
                mfm_runtime::RunnableReason::Advance => text.push_str("reason=advance\n"),
                mfm_runtime::RunnableReason::Retry => text.push_str("reason=retry\n"),
                mfm_runtime::RunnableReason::Restart { checkpoint } => {
                    text.push_str("reason=restart\n");
                    json_field(&mut text, "checkpoint", checkpoint)?;
                }
            }
        }
        RunViewState::EffectPending {
            effect,
            latest_failure,
        } => {
            text.push_str("state=effect_pending\n");
            json_field(&mut text, "position", effect.call().position())?;
            json_field(&mut text, "effect_id", effect.effect_id())?;
            json_field(&mut text, "latest_failure", latest_failure)?;
        }
        RunViewState::AwaitingRecovery { failure } => {
            text.push_str("state=awaiting_recovery\n");
            json_field(&mut text, "position", failure.call().position())?;
            json_field(&mut text, "failure", failure)?;
        }
        RunViewState::AwaitingInterpretation { settlement } => {
            text.push_str("state=awaiting_interpretation\n");
            json_field(&mut text, "settlement", settlement)?;
        }
        RunViewState::Succeeded(value) => {
            text.push_str("state=succeeded\n");
            json_field(
                &mut text,
                "contract_ref",
                &value
                    .contract_ref()
                    .map_err(|error| error.into_diagnostic("run_text"))?,
            )?;
            json_field(&mut text, "value_ref", value.value_ref())?;
            text.push_str("canonical=");
            text.push_str(&String::from_utf8_lossy(value.canonical_bytes()));
            text.push('\n');
        }
        RunViewState::Failed(report) => {
            text.push_str("state=failed\n");
            json_field(&mut text, "value_ref", report.value_ref())?;
            text.push_str("report=");
            text.push_str(&String::from_utf8_lossy(report.canonical_bytes()));
            text.push('\n');
        }
    }
    Ok(text)
}

pub(super) fn emit_error(output: OutputFormat, error: CliError) -> ExitCode {
    error_to(output, error, &mut std::io::stderr().lock())
}

fn error_to(output: OutputFormat, error: CliError, stderr: &mut impl Write) -> ExitCode {
    if let CliError::RunRequest(error) = error {
        return run_error_to(output, *error, stderr);
    }
    let message = error.message();
    let mut model = SerializableClientError::new(error.code(), &message);
    if let CliError::Output(diagnostic) = &error {
        model = model.with_diagnostic(diagnostic);
    }
    match output {
        OutputFormat::Json => final_json(stderr, &model),
        OutputFormat::Text => {
            let mut text = format!("error: {message}\ncode={}\n", error.code());
            if let CliError::Output(diagnostic) = &error {
                if json_field(&mut text, "diagnostic", diagnostic).is_err() {
                    return ExitCode::from(2);
                }
            }
            let _delivery = write_output(stderr, OutputStream::Stderr, [text.as_bytes()]);
        }
    }
    ExitCode::from(2)
}

fn run_error_to(output: OutputFormat, error: RunRequestError, stderr: &mut impl Write) -> ExitCode {
    let message = error.to_string();
    match output {
        OutputFormat::Json => {
            let encoded = SerializableClientError::for_run(&error, &message).and_then(|model| {
                encode_response(&model).map_err(|cause| {
                    InvocationDiagnostic::from_fields("json_error", "emit_error", &cause, None)
                })
            });
            match encoded {
                Ok(encoded) => {
                    let _delivery = write_output(
                        stderr,
                        OutputStream::Stderr,
                        [encoded.get().as_bytes(), b"\n"],
                    );
                }
                Err(diagnostic) => {
                    let terminal = SerializableClientError::failed_run_report(
                        &error,
                        &message,
                        &diagnostic,
                        None,
                    );
                    final_json(stderr, &terminal);
                }
            }
        }
        OutputFormat::Text => {
            let encoded = (|| {
                let mut text = format!("error: {message}\ncode={}\n", error.code());
                if let Some(recovery) = error.recovery() {
                    json_field(&mut text, "recovery", recovery)?;
                }
                let invocation = match &error {
                    RunRequestError::Request(_) => return Ok(text),
                    RunRequestError::Invocation(invocation)
                    | RunRequestError::AppendIndeterminate { invocation, .. } => invocation,
                };
                match invocation {
                    InvocationFailure::Execution {
                        run_id,
                        error,
                        last_observed,
                    } => {
                        json_field(&mut text, "run_id", run_id)?;
                        if let Some(view) = last_observed {
                            text.push_str(&run_text(view)?);
                        }
                        json_field(&mut text, "cause", error)?;
                    }
                    InvocationFailure::RecoveryStopped { observed } => {
                        text.push_str(&run_text(observed)?)
                    }
                }
                Ok::<_, InvocationDiagnostic>(text)
            })();
            match encoded {
                Ok(text) => {
                    let _delivery = write_output(stderr, OutputStream::Stderr, [text.as_bytes()]);
                }
                Err(diagnostic) => {
                    let terminal = SerializableClientError::failed_run_report(
                        &error,
                        &message,
                        &diagnostic,
                        None,
                    );
                    final_text(stderr, &terminal, None);
                }
            }
        }
    }
    ExitCode::from(2)
}

pub(super) fn write_json_stdout(value: &impl Serialize) -> Result<(), CliError> {
    let encoded = encode_response(value).map_err(|cause| {
        CliError::Output(InvocationDiagnostic::from_fields(
            "json_error",
            "write_json_stdout",
            &cause,
            None,
        ))
    })?;
    write_output(
        &mut std::io::stdout().lock(),
        OutputStream::Stdout,
        [encoded.get().as_bytes(), b"\n"],
    )
    .map_err(|cause| {
        CliError::Output(InvocationDiagnostic::from_fields(
            "output_io",
            "write_json_stdout",
            &cause,
            None,
        ))
    })
}

pub(super) fn write_stdout(bytes: &[u8]) -> Result<(), CliError> {
    write_output(&mut std::io::stdout().lock(), OutputStream::Stdout, [bytes]).map_err(|cause| {
        CliError::Output(InvocationDiagnostic::from_fields(
            "output_io",
            "write_stdout",
            &cause,
            None,
        ))
    })
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod tests;
