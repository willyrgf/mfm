use serde::Serialize;
use std::fmt;

/// Standardized result type for all CLI commands
pub(crate) type CommandResult<T> = Result<CommandOutput<T>, CommandError>;

/// Standardized success output for commands
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CommandOutput<T> {
    /// Structured payload to render for the command.
    pub(crate) data: T,
    /// Optional text message used for text-mode rendering.
    pub(crate) message: Option<String>,
}

impl<T> CommandOutput<T> {
    /// Builds a successful command output with no explicit text message.
    pub(crate) fn new(data: T) -> Self {
        Self {
            data,
            message: None,
        }
    }
}

/// Standardized error type for all CLI commands
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CommandError {
    /// Stable machine-readable error code.
    pub(crate) code: String,
    /// Human-readable error message.
    pub(crate) message: String,
    /// Process exit code to use when terminating.
    pub(crate) exit_code: i32,
}

impl CommandError {
    /// Builds a command error with the default non-zero exit code.
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code: 1,
        }
    }

    /// Overrides the exit code for this command error.
    pub(crate) fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }

    /// Builds the standard invalid-UUID command error.
    pub(crate) fn invalid_uuid(message: impl Into<String>) -> Self {
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
