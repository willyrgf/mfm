use serde::{Deserialize, Serialize};
use std::fmt;

use crate::commands::{
    result::{CommandResult, PublicError},
    OutputFormat,
};

/// Standardized error response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Top-level response status.
    pub status: ResponseStatus,
    /// Structured error details payload.
    pub error: PublicError,
}

/// Status field used in JSON responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseStatus {
    /// The command completed successfully.
    Success,
    /// The command failed.
    Error,
}

impl ErrorResponse {
    /// Builds a JSON error response from the shared public error payload.
    pub fn new(error: PublicError) -> Self {
        Self {
            status: ResponseStatus::Error,
            error,
        }
    }
}

/// Public key metadata used by text and JSON keystore list output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyDisplay {
    /// Stable key identifier.
    pub id: String,
    /// Human-readable key label.
    pub label: String,
    /// Normalized key type string.
    pub key_type: String,
    /// Optional derived address.
    pub address: Option<String>,
    /// Creation timestamp string.
    pub created: String,
}

impl fmt::Display for KeyDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ID: {}, Label: {}, Type: {}, Created: {}",
            self.id, self.label, self.key_type, self.created
        )
    }
}

/// Standardized success response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse<T> {
    /// Top-level response status.
    pub status: ResponseStatus,
    /// Structured success payload.
    pub data: T,
}

impl<T> SuccessResponse<T> {
    /// Builds a JSON success response from the supplied payload.
    pub fn new(data: T) -> Self {
        Self {
            status: ResponseStatus::Success,
            data,
        }
    }
}

/// Print a success response in the specified format
pub(crate) fn print_success<T: Serialize>(data: T, format: &OutputFormat) {
    if !format.is_json() {
        return;
    }

    let response = SuccessResponse::new(data);
    if let Ok(json) = serde_json::to_string_pretty(&response) {
        println!("{json}");
    } else {
        eprintln!(
            r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize response"}}}}"#
        );
    }
}

/// Print an error response in the specified format  
pub(crate) fn print_error(error: PublicError, format: &OutputFormat) {
    if !format.is_json() {
        eprintln!("{}", error_text(&error));
        return;
    }

    let response = ErrorResponse::new(error);
    if let Ok(json) = serde_json::to_string_pretty(&response) {
        eprintln!("{json}");
    } else {
        eprintln!(
            r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize error response"}}}}"#
        );
    }
}

fn error_text(error: &PublicError) -> String {
    if error.code() == "RuntimeConfigRequired" {
        format!(
            "{}: {}; pass --runtime-config",
            error.code(),
            error.message()
        )
    } else {
        format!("{}: {}", error.code(), error.message())
    }
}

/// Renders a clap parse error using the requested output format and exits with clap's code.
pub(crate) fn handle_cli_parse_error(error: clap::Error, format: &OutputFormat) -> ! {
    if !format.is_json() {
        error.exit();
    }

    let exit_code = error.exit_code();
    let message = sanitize_clap_error_message(&error.to_string());
    print_error(PublicError::bad_request("CliParseError", message), format);
    std::process::exit(exit_code);
}

fn sanitize_clap_error_message(message: &str) -> String {
    strip_ansi_escape_sequences(message).trim().to_string()
}

fn strip_ansi_escape_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }

    output
}

/// Formats a keystore list response as an ASCII table for text output.
pub fn format_keys_table(keys: &[KeyDisplay], show_addresses: bool) -> String {
    if show_addresses {
        let headers = ["id", "label", "key_type", "address", "created"];
        let rows: Vec<Vec<String>> = keys
            .iter()
            .map(|key| {
                vec![
                    key.id.clone(),
                    key.label.clone(),
                    key.key_type.clone(),
                    key.address.clone().unwrap_or_else(|| "N/A".to_string()),
                    key.created.clone(),
                ]
            })
            .collect();
        format_ascii_table(&headers, &rows)
    } else {
        let headers = ["id", "label", "key_type", "created"];
        let rows: Vec<Vec<String>> = keys
            .iter()
            .map(|key| {
                vec![
                    key.id.clone(),
                    key.label.clone(),
                    key.key_type.clone(),
                    key.created.clone(),
                ]
            })
            .collect();
        format_ascii_table(&headers, &rows)
    }
}

fn format_ascii_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let widths = compute_column_widths(headers, rows);
    let border = build_border(&widths);
    let mut output = String::new();

    output.push_str(&border);
    output.push('\n');
    output.push_str(&build_row(headers, &widths));
    output.push('\n');

    if rows.is_empty() {
        output.push_str(&border);
        return output;
    }

    output.push_str(&border);
    for row in rows {
        output.push('\n');
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        output.push_str(&build_row(&cells, &widths));
    }
    output.push('\n');
    output.push_str(&border);
    output
}

fn compute_column_widths(headers: &[&str], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (idx, value) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(value.chars().count());
        }
    }
    widths
}

fn build_border(widths: &[usize]) -> String {
    let mut border = String::new();
    border.push('+');
    for width in widths {
        border.push_str(&"-".repeat(*width + 2));
        border.push('+');
    }
    border
}

fn build_row(cells: &[&str], widths: &[usize]) -> String {
    let mut row = String::new();
    row.push('|');
    for (cell, width) in cells.iter().zip(widths) {
        row.push(' ');
        row.push_str(cell);
        row.push_str(&" ".repeat(width.saturating_sub(cell.chars().count())));
        row.push(' ');
        row.push('|');
    }
    row
}

/// Handles the output formatting for command results
pub(crate) fn handle_command_result<T>(result: CommandResult<T>, format: &OutputFormat) -> !
where
    T: Serialize + fmt::Display,
{
    match result {
        Ok(output) => {
            if format.is_json() {
                print_success(output.data, format);
            } else if let Some(message) = output.message {
                println!("{message}");
            } else {
                println!("{}", output.data);
            }
            std::process::exit(0);
        }
        Err(error) => {
            print_error(error, format);
            std::process::exit(1);
        }
    }
}

/// Renders a new ops/run success directly, without the legacy CLI success envelope.
pub(crate) fn handle_public_result<T>(
    result: Result<T, PublicError>,
    format: &OutputFormat,
    text: impl FnOnce(&T) -> Result<String, PublicError>,
) -> !
where
    T: mfm_app::PublicJsonResponse,
{
    match result {
        Ok(output) => match render_public_result(&output, format, text) {
            Ok(rendered) => {
                print!("{rendered}");
                std::process::exit(0);
            }
            Err(error) => {
                print_error(error, format);
                std::process::exit(1);
            }
        },
        Err(error) => {
            print_error(error, format);
            std::process::exit(1);
        }
    }
}

/// Renders one reviewed ops/run success without performing process IO.
pub(crate) fn render_public_result<T>(
    output: &T,
    format: &OutputFormat,
    text: impl FnOnce(&T) -> Result<String, PublicError>,
) -> Result<String, PublicError>
where
    T: mfm_app::PublicJsonResponse,
{
    if !format.is_json() {
        return text(output);
    }

    output
        .public_json()
        .and_then(|value| {
            serde_json::to_string_pretty(&value).map_err(|_| {
                PublicError::internal("SerializationError", "Failed to serialize response payload")
            })
        })
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReviewedResponse;

    impl mfm_app::PublicJsonResponse for ReviewedResponse {
        fn public_json(&self) -> Result<serde_json::Value, PublicError> {
            Ok(serde_json::json!({
                "at_journal_head": {
                    "commit_digest": "sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000",
                    "run_sequence": 1,
                },
                "next_cursor": null,
                "transitions": [],
            }))
        }
    }

    #[test]
    fn runtime_config_text_adds_cli_remediation() {
        let error = PublicError::new(
            mfm_app::ErrorClass::ServiceUnavailable,
            "RuntimeConfigRequired",
            "test/primary requires EVM and Bitcoin runtime routes",
        );

        assert_eq!(
            error_text(&error),
            "RuntimeConfigRequired: test/primary requires EVM and Bitcoin runtime routes; pass --runtime-config"
        );
    }

    #[test]
    fn other_error_text_has_no_runtime_config_remediation() {
        let error = PublicError::bad_request("InvalidInput", "input is invalid");
        assert_eq!(error_text(&error), "InvalidInput: input is invalid");
    }

    #[test]
    fn public_result_renderer_preserves_reviewed_json_and_text() {
        let output = ReviewedResponse;
        assert_eq!(
            render_public_result(&output, &OutputFormat::Json, |_| unreachable!())
                .expect("render reviewed JSON"),
            concat!(
                "{\n",
                "  \"at_journal_head\": {\n",
                "    \"commit_digest\": \"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000\",\n",
                "    \"run_sequence\": 1\n",
                "  },\n",
                "  \"next_cursor\": null,\n",
                "  \"transitions\": []\n",
                "}\n",
            )
        );
        assert_eq!(
            render_public_result(&output, &OutputFormat::Text, |_| {
                Ok("reviewed text\n".to_owned())
            })
            .expect("render reviewed text"),
            "reviewed text\n"
        );
    }

    #[test]
    fn runtime_error_json_uses_the_exact_transport_envelope() {
        let bytes = RUNTIME_STORE_FAULT_ERROR_RESPONSE_WIRE.as_bytes();
        let expected: serde_json::Value =
            serde_json::from_slice(bytes).expect("error response JSON");
        let error: PublicError =
            serde_json::from_value(expected["error"].clone()).expect("frozen public Runtime error");
        let response = ErrorResponse::new(error.clone());
        assert_eq!(
            serde_json::to_value(&response).expect("CLI error envelope"),
            expected
        );
        assert_eq!(
            error_text(&error),
            "ReplayVerificationFailed: Recorded run evidence failed verification"
        );
        let encoded = serde_json::to_string(&response).expect("CLI Runtime error JSON");
        assert!(!encoded.contains("implementation"));
        assert!(!encoded.contains("diagnostic"));
    }

    /// One complete error response carrying a store fault attribution.
    const RUNTIME_STORE_FAULT_ERROR_RESPONSE_WIRE: &str = concat!(
        r#"{"error":{"code":"ReplayVerificationFailed","#,
        r#""message":"Recorded run evidence failed verification","#,
        r#""runtime_fault":{"occurrence_id":null,"phase":"load_history","#,
        r#""pre_fault_head":null,"run_id":"run:sha256-jcs-v1:"#,
        "0000000000000000000000000000000000000000000000000000000000000000",
        r#"","subject":{"kind":"store","store_epoch":"7","#,
        r#""store_scope_id":"mfm.store_scope.v1:77777777777777777777777777777777"}}},"#,
        r#""status":"error"}"#,
    );
}
