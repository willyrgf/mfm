use mfm_values::DiagnosticEvidence;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum WriteStage {
    Write,
    Flush,
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("output operation failed")]
pub(super) struct OutputWriteError {
    stream: OutputStream,
    stage: WriteStage,
    details: DiagnosticEvidence,
}

fn details(error: &io::Error) -> DiagnosticEvidence {
    let mut details = serde_json::json!({
        "message": error.to_string(),
        "os_kind": format!("{:?}", error.kind()),
        "os_code": error.raw_os_error(),
    });
    let mut sources = Vec::new();
    let mut addresses = Vec::new();
    // get_ref retains the custom owner itself; io::Error::source can skip that layer.
    let mut source = error
        .get_ref()
        .map(|source| source as &dyn std::error::Error);
    while let Some(error) = source {
        let address = error as *const dyn std::error::Error as *const ();
        if addresses.contains(&address) {
            details["source_cycle"] = true.into();
            break;
        }
        addresses.push(address);
        let mut layer = serde_json::json!({"message": error.to_string()});
        if let Some(error) = error.downcast_ref::<io::Error>() {
            layer["os_kind"] = format!("{:?}", error.kind()).into();
            layer["os_code"] = serde_json::json!(error.raw_os_error());
        }
        sources.push(layer);
        source = error.source();
    }
    details["sources"] = sources.into();
    DiagnosticEvidence::from_value(details)
}

pub(super) fn write_output<'a>(
    writer: &mut impl Write,
    stream: OutputStream,
    chunks: impl IntoIterator<Item = &'a [u8]>,
) -> Result<(), OutputWriteError> {
    for bytes in chunks {
        writer.write_all(bytes).map_err(|error| OutputWriteError {
            stream,
            stage: WriteStage::Write,
            details: details(&error),
        })?;
    }
    writer.flush().map_err(|error| OutputWriteError {
        stream,
        stage: WriteStage::Flush,
        details: details(&error),
    })
}
