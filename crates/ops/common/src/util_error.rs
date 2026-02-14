#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtilError {
    pub code: &'static str,
    pub message: String,
}

impl UtilError {
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
