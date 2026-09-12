use mfm_diagnostics::{
    ChainEnd, DiagnosticEvidence, OmissionReason, OmittedField, OsFailureKind, SourceLayer,
};
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
    #[source]
    cause: Box<OutputIoError>,
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("output IO failed; unreviewed details withheld")]
struct OutputIoError {
    kind: OsFailureKind,
    os_code: Option<i32>,
    message: &'static str,
    sources: DiagnosticEvidence,
}

impl OutputIoError {
    fn capture(error: io::Error) -> Self {
        // get_ref retains the exposed custom owner itself; io::Error::source may skip that layer.
        // Neither arbitrary client objects nor rejected output buffers leave this IO owner.
        let source = error
            .get_ref()
            .map(|source| source as &dyn std::error::Error);
        let sources = DiagnosticEvidence::capture(
            None,
            source,
            if source.is_some() {
                ChainEnd::Complete
            } else {
                ChainEnd::Unavailable
            },
            |source| {
                let layer = match source.downcast_ref::<io::Error>() {
                    Some(source) => SourceLayer::os(os_kind(source.kind()), source.raw_os_error()),
                    None => SourceLayer::opaque(),
                };
                (
                    layer,
                    vec![(OmittedField::SourceDetail, OmissionReason::Withheld, None)],
                )
            },
        );
        Self {
            kind: os_kind(error.kind()),
            os_code: error.raw_os_error(),
            message: "withheld",
            sources,
        }
    }
}

pub(super) fn write_output(
    writer: &mut impl Write,
    stream: OutputStream,
    bytes: &[u8],
) -> Result<(), OutputWriteError> {
    writer.write_all(bytes).map_err(|error| OutputWriteError {
        stream,
        stage: WriteStage::Write,
        cause: Box::new(OutputIoError::capture(error)),
    })?;
    writer.flush().map_err(|error| OutputWriteError {
        stream,
        stage: WriteStage::Flush,
        cause: Box::new(OutputIoError::capture(error)),
    })
}

fn os_kind(kind: io::ErrorKind) -> OsFailureKind {
    use io::ErrorKind as Io;
    match kind {
        Io::NotFound => OsFailureKind::NotFound,
        Io::PermissionDenied => OsFailureKind::PermissionDenied,
        Io::ConnectionRefused => OsFailureKind::ConnectionRefused,
        Io::ConnectionReset => OsFailureKind::ConnectionReset,
        Io::ConnectionAborted => OsFailureKind::ConnectionAborted,
        Io::NotConnected => OsFailureKind::NotConnected,
        Io::AddrInUse => OsFailureKind::AddrInUse,
        Io::AddrNotAvailable => OsFailureKind::AddrNotAvailable,
        Io::BrokenPipe => OsFailureKind::BrokenPipe,
        Io::AlreadyExists => OsFailureKind::AlreadyExists,
        Io::WouldBlock => OsFailureKind::WouldBlock,
        Io::InvalidInput => OsFailureKind::InvalidInput,
        Io::InvalidData => OsFailureKind::InvalidData,
        Io::TimedOut => OsFailureKind::TimedOut,
        Io::WriteZero => OsFailureKind::WriteZero,
        Io::Interrupted => OsFailureKind::Interrupted,
        Io::UnexpectedEof => OsFailureKind::UnexpectedEof,
        Io::OutOfMemory => OsFailureKind::OutOfMemory,
        _ => OsFailureKind::Other,
    }
}
