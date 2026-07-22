use super::*;

/// State that selects required holdings from committed facts (adapter-bound).
pub struct SelectHoldingsState {
    config: SelectHoldingsConfig,
}

impl SelectHoldingsState {
    /// Returns the validated config.
    pub const fn config(&self) -> &SelectHoldingsConfig {
        &self.config
    }
}

impl StateSpec for SelectHoldingsState {
    type Config = SelectHoldingsConfig;
    type Context = NoContext;
    type Input = SelectHoldingsInput;
    type Output = SelectedHoldings;
    type Effect = ReadExternal;
    type Caps = (FactQueryReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("select_holdings")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.portfolio.state.select_holdings.v2")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
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
    type Plan = SelectHoldingsReadPlan;
    type Evidence = SelectHoldingsReadEvidence;
    type Facts = ();

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        SelectHoldingsReadPlan::new(&self.config, input)
            .map_err(|error| StateError::Message(error.to_string()))
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<(Self::Output, Self::Facts)> {
        let plan = self.plan(input, context)?;
        holding_read::reduce_select_holdings(
            &plan,
            evidence.primary_evidence(),
            evidence.fact_query_evidence(),
        )
        .map(|output| (output, ()))
        .map_err(|error| StateError::Message(error.to_string()))
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
        StateVersion::new("mfm.portfolio.state.assemble_snapshot.v2")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
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
        StateVersion::new("mfm.portfolio.state.project_report.v2")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
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
