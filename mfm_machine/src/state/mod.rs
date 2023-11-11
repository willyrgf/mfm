use anyhow::{anyhow, Error, Result};
use std::fmt;

pub mod context;
pub mod state_machine;

use context::Context;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Tag(String);

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Label(String);

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

#[derive(Debug, Clone, PartialEq)]
pub enum DependencyStrategy {
    Latest,
}

pub trait StateConfig {
    fn label(&self) -> &Label;
    fn tags(&self) -> &[Tag];
    fn depends_on(&self) -> &[Tag];
    fn depends_on_strategy(&self) -> &DependencyStrategy;
}

type StateResult = Result<(), StateError>;

pub trait StateHandler: StateConfig + Send + Sync {
    fn handler(&self, context: &mut dyn Context) -> StateResult;
}

#[derive(Debug, Clone)]
pub enum StateErrorRecoverability {
    Recoverable,
    Unrecoverable,
}

#[derive(Debug)]
pub enum StateError {
    Unknown(StateErrorRecoverability, anyhow::Error),
    ParsingInput(StateErrorRecoverability, anyhow::Error),
    OnChainError(StateErrorRecoverability, anyhow::Error),
    OffChainError(StateErrorRecoverability, anyhow::Error),
    RpcConnection(StateErrorRecoverability, anyhow::Error),
    StorageAccess(StateErrorRecoverability, anyhow::Error),
}

impl StateError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::Unknown(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::RpcConnection(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::StorageAccess(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OnChainError(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OffChainError(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::ParsingInput(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
        }
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(r, e) => write!(
                f,
                "unknown error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::RpcConnection(r, e) => write!(
                f,
                "RPC connection error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::StorageAccess(r, e) => write!(
                f,
                "storage access error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::OnChainError(r, e) => write!(
                f,
                "on-chain error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::OffChainError(r, e) => write!(
                f,
                "off-chain error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::ParsingInput(r, e) => write!(
                f,
                "parsing input error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
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
        state_error_to_anyhow(StateError::Unknown(
            StateErrorRecoverability::Unrecoverable,
            anyhow!("test error"),
        ));
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
        let s = "this shouldnt work".to_string();
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
