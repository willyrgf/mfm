use serde::Serialize;
use std::fmt;

/// Standardized result type for all CLI commands
pub type CommandResult<T> = Result<CommandOutput<T>, CommandError>;

/// Standardized success output for commands
#[derive(Debug, Clone, Serialize)]
pub struct CommandOutput<T> {
    pub data: T,
    pub message: Option<String>,
}

impl<T> CommandOutput<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            message: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_message(data: T, message: impl Into<String>) -> Self {
        Self {
            data,
            message: Some(message.into()),
        }
    }
}

/// Standardized error type for all CLI commands
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub exit_code: i32,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code: 1,
        }
    }

    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }

    pub fn invalid_uuid(message: impl Into<String>) -> Self {
        Self::new("InvalidUuid", message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<Box<dyn std::error::Error>> for CommandError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Self::new("InternalError", err.to_string())
    }
}
