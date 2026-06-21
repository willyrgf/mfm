//! Shared lightweight error type for low-level EVM utilities.
//!
//! `UtilError` is intentionally simple so helper modules can return stable machine-readable error
//! codes without pulling in transport- or runtime-specific error wrappers.

/// Error returned by low-level EVM utility helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct UtilError {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Human-readable explanation of the failure.
    pub message: String,
}

impl UtilError {
    /// Creates a new utility error.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
