use super::*;

/// State that selects required holdings from Platform facts (adapter-bound).
pub struct SelectHoldingsState {
    config: SelectHoldingsConfig,
}

impl SelectHoldingsState {
    /// Returns the validated config.
    pub const fn config(&self) -> &SelectHoldingsConfig {
        &self.config
    }

    /// Builds receipt-pinned selection evidence for one identity-matching claim row.
    pub fn selection_evidence_for_index(
        &self,
        selected_index: usize,
    ) -> Result<FactSelectionEvidence, PortfolioHoldingSelectionError> {
        let selected_index = u64::try_from(selected_index).map_err(|_| {
            PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::AmbiguousFacts,
                "selected fact query row index overflowed u64",
                None,
                None,
            )
        })?;
        FactSelectionEvidence::new(
            portfolio_holding_selection_policy_digest(),
            vec![selected_index],
            None,
        )
        .map_err(|error| {
            PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::AmbiguousFacts,
                error.to_string(),
                None,
                None,
            )
        })
    }
}

impl StateSpec for SelectHoldingsState {
    type Config = SelectHoldingsConfig;
    type Context = NoContext;
    type Input = SelectHoldingsInput;
    type Output = SelectedHoldings;
    type Effect = ReadExternal;
    type Caps = (FactIndexReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("select_holdings")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("select_holdings")
    }

    fn name() -> &'static str {
        "mfm.portfolio.select_holdings"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for SelectHoldingsState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(StateError::Message(format!(
            "{} requires adapter-bound Platform fact-index execution",
            Self::name()
        ))))
    }
}

/// State that assembles the canonical snapshot.
pub struct AssembleSnapshotState {
    config: AssembleSnapshotConfig,
}

impl StateSpec for AssembleSnapshotState {
    type Config = AssembleSnapshotConfig;
    type Context = NoContext;
    type Input = AssembleSnapshotInput;
    type Output = PortfolioSnapshot;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("assemble_snapshot")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("assemble_snapshot")
    }

    fn name() -> &'static str {
        "mfm.portfolio.assemble_snapshot"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for AssembleSnapshotState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_snapshot(&self.config, input)
    }
}

/// State that projects the canonical public report from a snapshot.
pub struct ProjectReportState;

impl StateSpec for ProjectReportState {
    type Config = ProjectReportConfig;
    type Context = NoContext;
    type Input = ProjectReportInput;
    type Output = PortfolioReport;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("project_report")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("project_report")
    }

    fn name() -> &'static str {
        "mfm.portfolio.project_report"
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for ProjectReportState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        project_report_from_snapshot(input.snapshot)
    }
}
