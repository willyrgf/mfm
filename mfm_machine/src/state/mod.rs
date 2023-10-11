use std::fmt;

#[derive(Debug)]
enum States<T: StateHandler> {
    Setup(T),
    Report(T),
}

trait StateHandler {
    fn handler(&self);
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

struct SetupState;
impl StateHandler for SetupState {
    fn handler(&self) {
        println!("handling setup state")
    }
}

struct ReportState;
impl StateHandler for ReportState {
    fn handler(&self) {
        println!("handling report state")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn setup_state_initialization() {
        let state: States<SetupState> = States::Setup(SetupState);
        match state {
            States::Setup(t) => t.handler(),
            _ => panic!("expected Setup state"),
        }
    }

    #[test]
    fn custom_error_to_anyhow_error() {
        let f = |error: StateError| -> anyhow::Error { error.into() };
        f(StateError::Unknown(StateErrorRecoverability::Unrecoverable));
    }
}
