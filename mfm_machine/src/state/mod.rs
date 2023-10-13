use anyhow::{anyhow, Error, Result};
use std::fmt;

pub mod context;
pub mod state_machine;
pub mod states;

use context::Context;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Tag(String);

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Label(String);

fn ensure_nonempty_ascii_lowercase_underscore(input: &str) -> Result<String, Error> {
    if input.is_empty() {
        return Err(anyhow!("empty string; this string should be non empty, lowercase and use underscore as separator"));
    }

    if !input.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(anyhow!("invalid char in '{}'; this string should be non empty, lowercase and use underscore as separator", input));
    }

    Ok(input.to_string())
}

impl Tag {
    pub fn new<S: AsRef<str>>(s: S) -> Result<Self, Error> {
        let input = s.as_ref();
        match ensure_nonempty_ascii_lowercase_underscore(input) {
            Ok(validated_input) => Ok(Self(validated_input)),
            Err(e) => Err(e),
        }
    }
}

impl Label {
    pub fn new<S: AsRef<str>>(s: S) -> Result<Self, Error> {
        let input = s.as_ref();
        match ensure_nonempty_ascii_lowercase_underscore(input) {
            Ok(validated_input) => Ok(Self(validated_input)),
            Err(e) => Err(e),
        }
    }
}

trait StateMetadata {
    fn label(&self) -> &Label;
    fn tags(&self) -> &[Tag];
}

trait StateHandler: StateMetadata {
    type InputContext: Context;
    type OutputContext: Context;

    fn handler(&self, context: Self::InputContext) -> Result<Self::OutputContext, Error>;
}

// Those states are mfm-specific states, and should be moved to the app side
#[derive(Debug)]
enum State<T> {
    Setup(T),
    Report(T),
}

#[derive(Debug)]
enum StateErrorRecoverability {
    Recoverable,
    Unrecoverable,
}

#[derive(Debug)]
enum StateError {
    Unknown(StateErrorRecoverability),
    RpcConnection(StateErrorRecoverability),
    StorageAccess(StateErrorRecoverability),
    OnChainError(StateErrorRecoverability),
    OffChainError(StateErrorRecoverability),
    ParsingInput(StateErrorRecoverability),
}

impl StateError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::Unknown(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::RpcConnection(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::StorageAccess(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OnChainError(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OffChainError(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::ParsingInput(recov) => matches!(recov, StateErrorRecoverability::Recoverable),
        }
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(_) => write!(f, "unknown error"),
            Self::RpcConnection(_) => write!(f, "RPC connection error"),
            Self::StorageAccess(_) => write!(f, "storage access error"),
            Self::OnChainError(_) => write!(f, "on-chain error"),
            Self::OffChainError(_) => write!(f, "off-chain error"),
            Self::ParsingInput(_) => write!(f, "parsing input error"),
        }
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn custom_error_to_anyhow_error() {
        let state_error_to_anyhow = |error: StateError| -> anyhow::Error { error.into() };
        state_error_to_anyhow(StateError::Unknown(StateErrorRecoverability::Unrecoverable));
    }

    #[test]
    fn test_valid_input_ensure_nonempty_ascii_lowercase_underscore() {
        let inputs = vec![
            "this_should_work".to_string(),
            "this_should_work_also".to_string(),
            "thisalso".to_string(),
        ];

        inputs.iter().for_each(|input| {
            let result = ensure_nonempty_ascii_lowercase_underscore(input);
            assert!(result.is_ok());
            assert_eq!(&result.unwrap(), input);
        })
    }

    #[test]
    fn test_invalid_spaces_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this should work".to_string();
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "".to_string();
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_special_char_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this_should_not_work_@".to_string();
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }
}
