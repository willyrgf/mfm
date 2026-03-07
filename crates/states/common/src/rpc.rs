use mfm_machine::errors::StateError;

use crate::errors::state_unknown;

/// Maps low-level validation parse failures to stable user-facing messages.
pub fn validation_assertion_error_message(err: &str) -> &'static str {
    match err {
        "read assertion did not match ABI" => "read assertion did not match ABI",
        "event assertion referenced unknown event" => "event assertion referenced unknown event",
        "anonymous events are not supported for validation" => {
            "anonymous events are not supported for validation"
        }
        "invalid from_block" => "invalid from_block",
        "invalid to_block" => "invalid to_block",
        _ => "invalid evm_validate op_config",
    }
}

/// Expects a JSON string response and maps mismatches to a stable state error.
pub fn expect_string(
    response: &serde_json::Value,
    code: &'static str,
    message: &'static str,
) -> Result<String, StateError> {
    response
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| state_unknown(code, message))
}

/// Expects a JSON array response and maps mismatches to a stable state error.
pub fn expect_array<'a>(
    response: &'a serde_json::Value,
    code: &'static str,
    message: &'static str,
) -> Result<&'a Vec<serde_json::Value>, StateError> {
    response
        .as_array()
        .ok_or_else(|| state_unknown(code, message))
}

/// Returns `Ok(())` when `condition` is true, otherwise returns a stable state error.
pub fn assert_condition(
    condition: bool,
    code: &'static str,
    message: &'static str,
) -> Result<(), StateError> {
    if condition {
        return Ok(());
    }
    Err(state_unknown(code, message))
}

#[cfg(test)]
mod tests {
    use super::validation_assertion_error_message;

    #[test]
    fn validation_assertion_known_messages_are_stable() {
        assert_eq!(
            validation_assertion_error_message("read assertion did not match ABI"),
            "read assertion did not match ABI"
        );
        assert_eq!(
            validation_assertion_error_message("event assertion referenced unknown event"),
            "event assertion referenced unknown event"
        );
        assert_eq!(
            validation_assertion_error_message("anonymous events are not supported for validation"),
            "anonymous events are not supported for validation"
        );
        assert_eq!(
            validation_assertion_error_message("invalid from_block"),
            "invalid from_block"
        );
        assert_eq!(
            validation_assertion_error_message("invalid to_block"),
            "invalid to_block"
        );
    }

    #[test]
    fn validation_assertion_unknown_maps_to_generic_message() {
        assert_eq!(
            validation_assertion_error_message("any other parse failure"),
            "invalid evm_validate op_config"
        );
    }
}
