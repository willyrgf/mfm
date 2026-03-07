/// Error returned by low-level EVM utility helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl std::fmt::Display for UtilError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for UtilError {}
