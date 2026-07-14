use serde::Serialize;

use mfm_app::PublicSafeMessage;
use mfm_evm_core::util_error::UtilError;

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
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{message}")]
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
    pub(crate) fn new(code: impl Into<String>, message: impl Into<PublicSafeMessage>) -> Self {
        Self {
            code: code.into(),
            message: message.into().into_string(),
            exit_code: 1,
        }
    }

    /// Builds a command error for lower-level failures without exposing backend details.
    pub(crate) fn backend(code: impl Into<String>, message: &'static str) -> Self {
        Self::new(code, PublicSafeMessage::backend(message))
    }
}

impl From<Box<dyn std::error::Error>> for CommandError {
    fn from(_err: Box<dyn std::error::Error>) -> Self {
        Self::backend("InternalError", "Command failed")
    }
}

impl From<mfm_app::AppError> for CommandError {
    fn from(error: mfm_app::AppError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<UtilError> for CommandError {
    fn from(error: UtilError) -> Self {
        Self::new(error.code, error.message)
    }
}
