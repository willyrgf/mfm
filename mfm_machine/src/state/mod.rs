use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

trait Context {
    type Output: Deserialize<'static>;
    type Input: Serialize;

    fn read(&self) -> Self::Output;
    fn write(&self, ctx_input: &Self::Input);
}

trait StateHandler {
    type InputContext: Context;
    type OutputContext: Context;

    fn handler(&self, context: Self::InputContext) -> Result<Self::OutputContext, Error>;
}

// Those states are mfm-specific states, and should be moved to the app side
#[derive(Debug)]
enum States<T> {
    Setup(T),
    Report(T),
}

#[derive(Debug)]
struct ContextInput {}

#[derive(Debug)]
struct ContextOutput {}

impl Context for ContextInput {
    type Output = String;
    type Input = String;

    fn read(&self) -> Self::Output {
        "hello".to_string()
    }

    fn write(&self, ctx_input: &Self::Input) {
        let _x = ctx_input;
    }
}

impl Context for ContextOutput {
    type Input = String;
    type Output = String;

    fn read(&self) -> Self::Output {
        "hello".to_string()
    }

    fn write(&self, ctx_input: &Self::Input) {
        let _x = ctx_input;
    }
}

struct SetupState;
impl StateHandler for SetupState {
    type InputContext = ContextInput;
    type OutputContext = ContextOutput;

    fn handler(&self, context: ContextInput) -> Result<ContextOutput, Error> {
        let _data = context.read();
        let data = "some new data".to_string();
        let ctx_output = ContextOutput {};
        ctx_output.write(&data);
        Ok(ctx_output)
    }
}

struct ReportState;
impl StateHandler for ReportState {
    type InputContext = ContextInput;
    type OutputContext = ContextOutput;

    fn handler(&self, context: ContextInput) -> Result<ContextOutput, Error> {
        let _data = context.read();
        let data = "some new data".to_string();
        let ctx_output = ContextOutput {};
        ctx_output.write(&data);
        Ok(ctx_output)
    }
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
    fn setup_state_initialization() {
        let state: States<SetupState> = States::Setup(SetupState);
        let ctx_input = ContextInput {};
        match state {
            States::Setup(t) => match t.handler(ctx_input) {
                Ok(ctx_output) => println!("got an ctx_output: {:?}", ctx_output),
                Err(e) => println!("got an error: {:?}", e),
            },
            _ => panic!("expected Setup state"),
        }
    }

    #[test]
    fn custom_error_to_anyhow_error() {
        let state_error_to_anyhow = |error: StateError| -> anyhow::Error { error.into() };
        state_error_to_anyhow(StateError::Unknown(StateErrorRecoverability::Unrecoverable));
    }
}
