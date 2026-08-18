#![warn(missing_docs)]
//! Thin Portfolio-only Application facade over the typed Runtime.

use mfm_evm::{EvmPhysicalTarget, EVM_BALANCE_SOURCE_LIMIT};
use mfm_ids::RunId;
use mfm_portfolio::{
    plan_snapshot, PortfolioConfig, PortfolioError, PortfolioSnapshotSelector,
    PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_runtime::{RunView, Runtime, RuntimeError};

/// Result type for Application operations.
pub type Result<T> = std::result::Result<T, ApplicationError>;

/// Redaction-safe Application failure.
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    /// Caller selector or selector/config matching was invalid.
    #[error("application request is invalid")]
    InvalidRequest,
    /// Trusted composition or Program authoring failed.
    #[error("application internal failure")]
    Internal,
    /// Runtime returned its reviewed typed failure.
    #[error("runtime operation failed")]
    Runtime(#[source] RuntimeError),
}

/// Portfolio-only facade with no independent execution lifecycle.
pub struct Application {
    runtime: Runtime,
}

impl Application {
    /// Constructs the facade from the already-composed Runtime.
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }

    /// Plans, admits, and progresses one Portfolio snapshot.
    pub async fn start_portfolio(
        &self,
        run_id: RunId,
        selector: PortfolioSnapshotSelector,
        config: &PortfolioConfig,
        targets: &[EvmPhysicalTarget],
    ) -> Result<RunView> {
        if targets.len() > EVM_BALANCE_SOURCE_LIMIT {
            return Err(ApplicationError::Internal);
        }
        let owned_config = config.clone();
        let owned_targets = targets.to_vec();
        let planned = tokio::task::spawn_blocking(move || {
            plan_snapshot(selector, &owned_config, &owned_targets)
        })
        .await
        .map_err(|_| ApplicationError::Internal)?;
        let (program, c0) = planned.map_err(map_portfolio_error)?;
        if program.entry_point_id().as_str() != PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID {
            return Err(ApplicationError::Internal);
        }
        self.runtime
            .start(run_id, program, c0)
            .await
            .map_err(ApplicationError::Runtime)
    }

    /// Resumes and progresses one retained run.
    pub async fn resume(&self, run_id: &RunId) -> Result<RunView> {
        self.runtime
            .resume(run_id)
            .await
            .map_err(ApplicationError::Runtime)
    }

    /// Reads one retained run without progression.
    pub async fn read(&self, run_id: &RunId) -> Result<RunView> {
        self.runtime
            .read(run_id)
            .await
            .map_err(ApplicationError::Runtime)
    }
}

fn map_portfolio_error(error: PortfolioError) -> ApplicationError {
    match error {
        PortfolioError::InvalidValue => ApplicationError::InvalidRequest,
        PortfolioError::InvalidContinuation | PortfolioError::Program => ApplicationError::Internal,
    }
}
