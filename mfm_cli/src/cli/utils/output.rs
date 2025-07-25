use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use super::super::OutputFormat;

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct KeyDisplay {
    pub id: String,
    pub label: String,
    pub key_type: String,
    #[tabled(skip)]
    pub address: Option<String>,
    pub created: String,
}

/// Standardized success response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse<T> {
    pub status: String,
    pub data: T,
}

impl<T> SuccessResponse<T> {
    pub fn new(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data,
        }
    }
}

/// Standardized error response structure for JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub status: String,
    pub error: ErrorDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetails {
    pub code: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            status: "error".to_string(),
            error: ErrorDetails {
                code: code.to_string(),
                message: message.to_string(),
            },
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

pub fn format_keys(keys: Vec<KeyDisplay>, format: OutputFormat, show_addresses: bool) -> String {
    match format {
        OutputFormat::Text => {
            if show_addresses {
                let mut table_data = Vec::new();
                for key in keys {
                    table_data.push(KeyDisplayWithAddress {
                        id: key.id,
                        label: key.label,
                        key_type: key.key_type,
                        address: key.address.unwrap_or_else(|| "N/A".to_string()),
                        created: key.created,
                    });
                }
                Table::new(table_data).to_string()
            } else {
                Table::new(keys).to_string()
            }
        }
        OutputFormat::Json => {
            let data = if show_addresses {
                keys
            } else {
                keys.into_iter()
                    .map(|mut k| {
                        k.address = None;
                        k
                    })
                    .collect()
            };
            let response = SuccessResponse::new(data);
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| r#"{"status":"error","error":{"code":"SerializationError","message":"Failed to serialize response"}}"#.to_string())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
struct KeyDisplayWithAddress {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub address: String,
    pub created: String,
}
