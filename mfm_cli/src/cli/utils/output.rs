use serde::{Deserialize, Serialize};
use std::fmt;
use tabled::{Table, Tabled};

use super::super::{command_result::CommandResult, OutputFormat};

/// Standardized error response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub status: ResponseStatus,
    pub error: ErrorDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetails {
    pub code: String,
    pub message: String,
}

impl ErrorResponse {
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

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct KeyDisplay {
    pub id: String,
    pub label: String,
    pub key_type: String,
    #[tabled(skip)]
    pub address: Option<String>,
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
    pub status: ResponseStatus,
    pub data: T,
}

impl<T> SuccessResponse<T> {
    pub fn new(data: T) -> Self {
        Self {
            status: ResponseStatus::Success,
            data,
        }
    }
}

/// Print a success response in the specified format
pub fn print_success<T: Serialize>(data: T, format: &OutputFormat) {
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
pub fn print_error(code: &str, message: &str, format: &OutputFormat) {
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

pub fn format_keys_table(keys: &[KeyDisplay], show_addresses: bool) -> String {
    if show_addresses {
        let table_data: Vec<KeyDisplayWithAddress> = keys
            .iter()
            .map(|key| KeyDisplayWithAddress {
                id: key.id.clone(),
                label: key.label.clone(),
                key_type: key.key_type.clone(),
                address: key.address.clone().unwrap_or_else(|| "N/A".to_string()),
                created: key.created.clone(),
            })
            .collect();
        Table::new(table_data).to_string()
    } else {
        Table::new(keys).to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct KeyDisplayWithAddress {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub address: String,
    pub created: String,
}

/// Handles the output formatting for command results
pub fn handle_command_result<T>(result: CommandResult<T>, format: &OutputFormat) -> !
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
