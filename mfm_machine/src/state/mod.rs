use anyhow::{anyhow, Error, Result};
use std::{
    fmt,
    sync::{Arc, Mutex},
};

pub mod context;

use context::Context;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Tag(&'static str);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Label(&'static str);

fn ensure_nonempty_ascii_lowercase_underscore(input: &'static str) -> Result<&'static str, Error> {
    if input.is_empty() {
        return Err(anyhow!("empty string; this string should be non empty, lowercase and use underscore as separator"));
    }

    if !input.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(anyhow!("invalid char in '{}'; this string should be non empty, lowercase and use underscore as separator", input));
    }

    Ok(input)
}

impl Tag {
    pub fn new(s: &'static str) -> Result<Self, Error> {
        let input = s.as_ref();
        match ensure_nonempty_ascii_lowercase_underscore(input) {
            Ok(validated_input) => Ok(Self(validated_input)),
            Err(e) => Err(e),
        }
    }
}

impl Label {
    pub fn new(s: &'static str) -> Result<Self, Error> {
        let input = s.as_ref();
        match ensure_nonempty_ascii_lowercase_underscore(input) {
            Ok(validated_input) => Ok(Self(validated_input.into())),
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum DependencyStrategy {
    Latest,
}

pub trait StateMetadata {
    fn label(&self) -> Label;
    fn tags(&self) -> Vec<Tag>;
    fn depends_on(&self) -> Vec<Tag>;
    fn depends_on_strategy(&self) -> DependencyStrategy;
}

pub type StateResult = Result<(), StateError>;

pub trait StateHandler: StateMetadata + Send + Sync {
    fn handler(&self, context: Arc<Mutex<Box<dyn Context>>>) -> StateResult;
}

pub type States = Arc<[Box<dyn StateHandler>]>;

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
        let inputs = vec!["this_should_work", "this_should_work_also", "thisalso"];

        inputs.iter().for_each(|input| {
            let result = ensure_nonempty_ascii_lowercase_underscore(input);
            assert!(result.is_ok());
            assert_eq!(&result.unwrap(), input);
        })
    }

    #[test]
    fn test_invalid_spaces_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this shouldnt work";
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "";
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_special_char_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this_should_not_work_@";
        let result = ensure_nonempty_ascii_lowercase_underscore(&s);
        assert!(result.is_err());
    }
}
