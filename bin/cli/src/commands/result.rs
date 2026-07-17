use serde::Serialize;

pub(crate) use mfm_app::PublicError;

/// Standardized result type for all CLI commands
pub(crate) type CommandResult<T> = Result<CommandOutput<T>, PublicError>;

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
