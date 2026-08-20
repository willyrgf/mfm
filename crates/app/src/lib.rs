#![warn(missing_docs)]
//! Thin Portfolio-only Application facade over the typed Runtime, plus the one trusted
//! Portfolio assembly composition every binary and Runtime test shares.

use std::sync::Arc;

use mfm_catalog::RunIndex;
use mfm_evm::{
    CheckChainIdentity, ConfirmBalanceAnchor, ConsolidateBalanceCollection, EvmAnchorRead,
    EvmBalanceRead, EvmChainIdentityRead, EvmEndpoint, EvmPhysicalTarget, ReadInitialAnchor,
    ReadNativeBalance, ReadTokenBalance, ReadTokenDecimals, SelectBalanceAsset,
    EVM_BALANCE_SOURCE_LIMIT,
};
use mfm_evm_live::{register_evm_reads, EvmProvider};
use mfm_ids::{ContentRef, RunId};
use mfm_portfolio::{
    plan_snapshot, ConsolidatePortfolio, EnterPortfolioCollection, InitializePortfolio,
    MapEvmBalanceFailure, PortfolioConfig, PortfolioContinuation, PortfolioError,
    PortfolioSnapshotSelector, ResumePortfolioCollection, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_runtime::{RunView, Runtime, RuntimeAssemblyBuilder, RuntimeError};
use mfm_store::Store;

/// Maximum number of EVM capability bindings in one composed Runtime.
pub const MAX_EVM_BINDINGS: usize = 256;

/// Registers every Portfolio and EVM State implementation the snapshot Program declares.
///
/// [`ComposedRuntime`] is the composition trusted callers want. This entry stays public only for
/// adapterless composition, where a caller deliberately finishes an assembly with no Read callback
/// to prove association rejects the Program before any Store IO.
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

/// Redaction-safe live composition failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("application composition is invalid")]
pub struct ComposeError;

/// One public capability binding derived from the exact typed live binding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicBindingView {
    /// One EVM observational route.
    Evm {
        /// Public chain identity.
        chain_id: u64,
        /// Stable public endpoint name.
        endpoint_id: String,
        /// Exact adapter binding reference derived from the physical target.
        binding_ref: ContentRef,
    },
}

struct BoundEvmRoute {
    target: EvmPhysicalTarget,
    endpoint_id: String,
    provider: Arc<dyn EvmProvider>,
}

/// Checked typed capability bindings consumed by one [`ComposedRuntime`].
///
/// The set is opaque so callers cannot supply public discovery views independently from the typed
/// targets and provider handles used to register Runtime adapters.
pub struct BoundCapabilitySet {
    evm: Vec<BoundEvmRoute>,
}

impl BoundCapabilitySet {
    /// Checks one stable, strictly ordered EVM route set.
    ///
    /// Each tuple contains the public chain id, checked endpoint identity, and its private provider
    /// handle. Empty sets are valid; at most 256 routes are accepted.
    pub fn new(
        routes: Vec<(u64, EvmEndpoint, Arc<dyn EvmProvider>)>,
    ) -> std::result::Result<Self, ComposeError> {
        if routes.len() > MAX_EVM_BINDINGS
            || routes.windows(2).any(|pair| {
                (pair[0].0, pair[0].1.endpoint_id()) >= (pair[1].0, pair[1].1.endpoint_id())
            })
        {
            return Err(ComposeError);
        }
        let mut evm = Vec::new();
        evm.try_reserve_exact(routes.len())
            .map_err(|_| ComposeError)?;
        for (chain_id, endpoint, provider) in routes {
            let endpoint_ref = endpoint.endpoint_ref().map_err(|_| ComposeError)?;
            let target =
                EvmPhysicalTarget::new(chain_id, endpoint_ref).map_err(|_| ComposeError)?;
            evm.push(BoundEvmRoute {
                target,
                endpoint_id: endpoint.endpoint_id().to_owned(),
                provider,
            });
        }
        Ok(Self { evm })
    }
}

/// One Runtime, RunIndex, typed planning targets, and exact public binding views built together.
pub struct ComposedRuntime {
    runtime: Runtime,
    _run_index: Arc<dyn RunIndex>,
    targets: Vec<EvmPhysicalTarget>,
    bindings: Vec<PublicBindingView>,
}

impl ComposedRuntime {
    /// Builds the complete Portfolio assembly from one backend and checked binding set.
    pub fn compose<B>(
        backend: Arc<B>,
        bindings: BoundCapabilitySet,
    ) -> std::result::Result<Self, ComposeError>
    where
        B: Store + RunIndex + 'static,
    {
        let mut builder = RuntimeAssemblyBuilder::new();
        register_portfolio_states(&mut builder).map_err(|_| ComposeError)?;
        let mut targets = Vec::new();
        let mut views = Vec::new();
        targets
            .try_reserve_exact(bindings.evm.len())
            .map_err(|_| ComposeError)?;
        views
            .try_reserve_exact(bindings.evm.len())
            .map_err(|_| ComposeError)?;
        for binding in bindings.evm {
            let binding_ref = binding.target.binding_ref().map_err(|_| ComposeError)?;
            views.push(PublicBindingView::Evm {
                chain_id: binding.target.chain_id(),
                endpoint_id: binding.endpoint_id,
                binding_ref,
            });
            targets.push(binding.target.clone());
            register_evm_reads(&mut builder, binding.target, binding.provider)
                .map_err(|_| ComposeError)?;
        }
        let assembly = builder.finish().map_err(|_| ComposeError)?;
        let store: Arc<dyn Store> = backend.clone();
        let run_index: Arc<dyn RunIndex> = backend;
        Ok(Self {
            runtime: Runtime::new(assembly, store),
            _run_index: run_index,
            targets,
            bindings: views,
        })
    }

    /// Returns the stable public binding list derived during composition.
    pub fn bindings(&self) -> &[PublicBindingView] {
        &self.bindings
    }
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
    composed: ComposedRuntime,
}

impl Application {
    /// Constructs the facade from one checked composed Runtime.
    pub fn new(composed: ComposedRuntime) -> Self {
        Self { composed }
    }

    /// Returns the stable public binding list derived during composition.
    pub fn bindings(&self) -> &[PublicBindingView] {
        self.composed.bindings()
    }

    /// Plans, admits, and progresses one Portfolio snapshot.
    pub async fn start_portfolio(
        &self,
        run_id: RunId,
        selector: PortfolioSnapshotSelector,
        config: &PortfolioConfig,
        targets: &[EvmPhysicalTarget],
    ) -> Result<RunView> {
        if targets.len() > EVM_BALANCE_SOURCE_LIMIT
            || targets
                .iter()
                .any(|target| !self.composed.targets.contains(target))
        {
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
        self.composed
            .runtime
            .start(run_id, program, c0)
            .await
            .map_err(ApplicationError::Runtime)
    }

    /// Resumes and progresses one retained run.
    pub async fn resume(&self, run_id: &RunId) -> Result<RunView> {
        self.composed
            .runtime
            .resume(run_id)
            .await
            .map_err(ApplicationError::Runtime)
    }

    /// Reads one retained run without progression.
    pub async fn read(&self, run_id: &RunId) -> Result<RunView> {
        self.composed
            .runtime
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
