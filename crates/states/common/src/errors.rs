use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, StateError};
use mfm_machine::ids::{ErrorCode, StateId};
use mfm_sdk::errors::SdkError;

/// Builds a stable [`ErrorInfo`] payload from the supplied fields.
pub fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.into(),
        details: None,
    }
}

/// Builds an [`SdkError`] from the supplied stable error fields.
pub fn sdk_error(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> SdkError {
    SdkError {
        info: info(code, category, retryable, message),
    }
}

/// Builds a non-retryable SDK parsing error.
pub fn sdk_parse_error(code: &'static str, message: &'static str) -> SdkError {
    sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

/// Builds a non-retryable SDK error in the unknown category.
pub fn sdk_unknown_error(code: &'static str, message: &'static str) -> SdkError {
    sdk_error(code, ErrorCategory::Unknown, false, message)
}

/// Builds a [`StateError`] that is not yet attached to a specific state id.
pub fn state_error(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> StateError {
    StateError {
        state_id: None,
        info: info(code, category, retryable, message),
    }
}

/// Builds a [`StateError`] that is attached to a specific state id.
pub fn state_error_with_state(
    state_id: StateId,
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> StateError {
    StateError {
        state_id: Some(state_id),
        info: info(code, category, retryable, message),
    }
}

/// Builds a non-retryable unknown-category [`StateError`] with a static message.
pub fn state_unknown(code: &'static str, message: &'static str) -> StateError {
    state_error(code, ErrorCategory::Unknown, false, message)
}

/// Builds a non-retryable unknown-category [`StateError`] with an owned message.
pub fn state_unknown_msg(code: &'static str, message: impl Into<String>) -> StateError {
    state_error(code, ErrorCategory::Unknown, false, message)
}

/// Converts an [`IoError`] into a [`StateError`] while preserving the original error info.
pub fn state_from_io(err: IoError) -> StateError {
    let info = match err {
        IoError::MissingFactKey(info)
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info)
        | IoError::MissingFact { info, .. } => info,
    };

    StateError {
        state_id: None,
        info,
    }
}

/// Maps keystore-specific error codes into coarse-grained state error categories.
pub fn keystore_error_category(code: &str) -> ErrorCategory {
    match code {
        "InvalidPrivateKey"
        | "InvalidMnemonic"
        | "InvalidDerivationPath"
        | "InvalidUuid"
        | "MissingArgument"
        | "AmbiguousLabel"
        | "KeyNotFound"
        | "InvalidRegex"
        | "InvalidPathConfig"
        | "OperationCancelled" => ErrorCategory::ParsingInput,
        _ => ErrorCategory::Unknown,
    }
}
