#![warn(missing_docs)]
//! Thin Portfolio-only Application facade over the typed Runtime, plus the one trusted
//! Portfolio assembly composition every binary and Runtime test shares.

use std::sync::Arc;

use mfm_evm::{
    CheckChainIdentity, ConfirmBalanceAnchor, ConsolidateBalanceCollection, EvmAnchorRead,
    EvmBalanceRead, EvmChainIdentityRead, EvmPhysicalTarget, ReadInitialAnchor, ReadNativeBalance,
    ReadTokenBalance, ReadTokenDecimals, SelectBalanceAsset, EVM_BALANCE_SOURCE_LIMIT,
};
use mfm_evm_live::{register_evm_reads, EvmProvider};
use mfm_ids::RunId;
use mfm_portfolio::{
    plan_snapshot, ConsolidatePortfolio, EnterPortfolioCollection, InitializePortfolio,
    MapEvmBalanceFailure, PortfolioConfig, PortfolioContinuation, PortfolioError,
    PortfolioSnapshotSelector, ResumePortfolioCollection, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_runtime::{RunView, Runtime, RuntimeAssembly, RuntimeAssemblyBuilder, RuntimeError};

/// Registers every Portfolio and EVM State implementation the snapshot Program declares.
///
/// [`portfolio_assembly`] is the composition trusted callers want. This entry stays public
/// only for adapterless composition, where a caller deliberately finishes an assembly with
/// no Read callback to prove association rejects the Program before any Store IO.
pub fn register_portfolio_states(builder: &mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()> {
    builder.register_pure::<InitializePortfolio>()?;
    builder.register_pure::<EnterPortfolioCollection>()?;
    builder.register_pure::<ResumePortfolioCollection>()?;
    builder.register_pure::<MapEvmBalanceFailure>()?;
    builder.register_pure::<ConsolidatePortfolio>()?;
    builder.register_read::<CheckChainIdentity<PortfolioContinuation>, EvmChainIdentityRead>()?;
    builder.register_read::<ReadInitialAnchor<PortfolioContinuation>, EvmAnchorRead>()?;
    builder.register_pure::<SelectBalanceAsset<PortfolioContinuation>>()?;
    builder.register_read::<ReadNativeBalance<PortfolioContinuation>, EvmBalanceRead>()?;
    builder.register_read::<ReadTokenDecimals<PortfolioContinuation>, EvmBalanceRead>()?;
    builder.register_read::<ReadTokenBalance<PortfolioContinuation>, EvmBalanceRead>()?;
    builder.register_read::<ConfirmBalanceAnchor<PortfolioContinuation>, EvmAnchorRead>()?;
    builder.register_pure::<ConsolidateBalanceCollection<PortfolioContinuation>>()
}

/// Composes the complete Portfolio assembly for one EVM route and its provider handle.
pub fn portfolio_assembly(
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmProvider>,
) -> mfm_runtime::Result<RuntimeAssembly> {
    let mut builder = RuntimeAssemblyBuilder::new();
    register_portfolio_states(&mut builder)?;
    register_evm_reads(&mut builder, target, provider)?;
    builder.finish()
}

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
