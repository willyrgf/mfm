use serde::{Deserialize, Serialize};
use std::fmt;

use crate::commands::{result::CommandResult, OutputFormat};

/// Standardized error response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Top-level response status.
    pub status: ResponseStatus,
    /// Structured error details payload.
    pub error: ErrorDetails,
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

/// Machine-readable error details for JSON responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetails {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

impl ErrorResponse {
    /// Builds a JSON error response from a code and message.
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            status: ResponseStatus::Error,
            error: ErrorDetails {
                code: code.to_string(),
                message: message.to_string(),
            },
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
    match format {
        OutputFormat::Text => {
            // For text format, we assume the data implements Display or similar
            // This is handled by the calling code
        }
        OutputFormat::Json => {
            let response = SuccessResponse::new(data);
            if let Ok(json) = serde_json::to_string_pretty(&response) {
                println!("{json}");
            } else {
                eprintln!(
                    r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize response"}}}}"#
                );
            }
        }
    }
}

/// Print an error response in the specified format  
pub(crate) fn print_error(code: &str, message: &str, format: &OutputFormat) {
    match format {
        OutputFormat::Text => {
            eprintln!("Error: {message}");
        }
        OutputFormat::Json => {
            let response = ErrorResponse::new(code, message);
            if let Ok(json) = serde_json::to_string_pretty(&response) {
                eprintln!("{json}");
            } else {
                eprintln!(
                    r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize error response"}}}}"#
                );
            }
        }
    }
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
            match format {
                OutputFormat::Text => {
                    if let Some(message) = output.message {
                        println!("{message}");
                    } else {
                        println!("{}", output.data);
                    }
                }
                OutputFormat::Json => {
                    print_success(output.data, format);
                }
            }
            std::process::exit(0);
        }
        Err(error) => {
            print_error(&error.code, &error.message, format);
            std::process::exit(error.exit_code);
        }
    }
}
