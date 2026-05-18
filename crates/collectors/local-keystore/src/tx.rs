//! Local keystore transaction-flow helpers.
//!
//! Generic EVM transaction models and encoding live in `mfm_evm_core::tx`. This module only keeps
//! keystore-specific helper errors and context-key conventions used by local signing flows.

use mfm_machine::ids::ContextKey;

/// Error returned by local keystore transaction helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeystoreTxError {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Human-readable message that is safe to surface to callers.
    pub message: String,
}

impl KeystoreTxError {
    /// Creates a new helper error.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for KeystoreTxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for KeystoreTxError {}

/// Returns the standard context key used to store a keystore transaction report.
pub fn output_context_key(op_path: &str) -> ContextKey {
    ContextKey(format!("{op_path}.out.report"))
}
