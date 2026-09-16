//! Extract exposed client sources at the transport owner.

use mfm_values::DiagnosticEvidence;
use std::error::Error;

pub(super) fn capture(
    response: Option<serde_json::Value>,
    mut source: Option<&(dyn Error + 'static)>,
) -> DiagnosticEvidence {
    let mut details = serde_json::json!({"response": response});
    let mut sources = Vec::new();
    let mut pointers = Vec::new();
    while let Some(error) = source {
        // Compare interface pointers, not concrete identities; inline children can share addresses.
        let pointer = error as *const dyn Error;
        if pointers
            .iter()
            .any(|previous| std::ptr::eq(*previous, pointer))
        {
            details["source_cycle"] = true.into();
            break;
        }
        pointers.push(pointer);
        let mut layer = serde_json::json!({"message": error.to_string()});
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            let kind = if error.is_timeout() {
                "timeout"
            } else if error.is_connect() {
                "connect"
            } else if error.is_body() {
                "body"
            } else if error.is_decode() {
                "decode"
            } else if error.is_redirect() {
                "redirect"
            } else {
                "request"
            };
            layer["transport_kind"] = kind.into();
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            layer["os_kind"] = format!("{:?}", error.kind()).into();
            layer["os_code"] = serde_json::json!(error.raw_os_error());
        } else if let Some(error) = error.downcast_ref::<serde_json::Error>() {
            layer["category"] = match error.classify() {
                serde_json::error::Category::Io => "io",
                serde_json::error::Category::Syntax => "syntax",
                serde_json::error::Category::Data => "data",
                serde_json::error::Category::Eof => "eof",
            }
            .into();
            layer["line"] = error.line().into();
            layer["column"] = error.column().into();
        }
        sources.push(layer);
        source = error.source();
    }
    details["sources"] = sources.into();
    DiagnosticEvidence::from_value(details)
}
