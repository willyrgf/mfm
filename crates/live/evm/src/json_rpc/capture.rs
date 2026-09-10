//! Review concrete client sources before they leave the transport owner.

use mfm_diagnostics::*;
use std::error::Error;

pub(super) fn capture(
    response: Option<(ResponseContext, Vec<CaptureOmission>)>,
    source: Option<&(dyn Error + 'static)>,
) -> DiagnosticEvidence {
    DiagnosticEvidence::capture(response, source, ChainEnd::Complete, |error| {
        let mut omissions = vec![(OmittedField::SourceDetail, OmissionReason::Withheld, None)];
        let layer = if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            if error.url().is_some() {
                omissions.push((OmittedField::Url, OmissionReason::Withheld, None));
            }
            let kind = if error.is_timeout() {
                TransportFailureKind::Timeout
            } else if error.is_connect() {
                TransportFailureKind::Connect
            } else if error.is_body() {
                TransportFailureKind::Body
            } else if error.is_decode() {
                TransportFailureKind::Decode
            } else if error.is_redirect() {
                TransportFailureKind::Redirect
            } else {
                TransportFailureKind::Request
            };
            SourceLayer::transport(kind)
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            SourceLayer::os(os_kind(error.kind()), error.raw_os_error())
        } else if let Some(error) = error.downcast_ref::<serde_json::Error>() {
            let category = match error.classify() {
                serde_json::error::Category::Io => ParseCategory::Io,
                serde_json::error::Category::Syntax => ParseCategory::Syntax,
                serde_json::error::Category::Data => ParseCategory::Data,
                serde_json::error::Category::Eof => ParseCategory::Eof,
            };
            SourceLayer::parse(
                category,
                ParseLocation::LineColumn {
                    line: error.line() as u64,
                    column: error.column() as u64,
                },
            )
        } else {
            SourceLayer::opaque()
        };
        (layer, omissions)
    })
}

fn os_kind(kind: std::io::ErrorKind) -> OsFailureKind {
    use std::io::ErrorKind as Io;
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
