use mfm_machine::errors::StateError;

use crate::errors::state_unknown;

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

pub fn expect_array<'a>(
    response: &'a serde_json::Value,
    code: &'static str,
    message: &'static str,
) -> Result<&'a Vec<serde_json::Value>, StateError> {
    response
        .as_array()
        .ok_or_else(|| state_unknown(code, message))
}

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
