use super::*;

/// Pure state that creates the typed report-readiness value.
pub struct PortfolioInputsReadyState {
    config: PortfolioInputsReadyConfig,
}

impl StateSpec for PortfolioInputsReadyState {
    type Config = PortfolioInputsReadyConfig;
    type Context = NoContext;
    type Input = ();
    type Output = PortfolioInputsReady;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("portfolio_inputs_ready")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("portfolio_inputs_ready")
    }

    fn name() -> &'static str {
        "mfm.portfolio.portfolio_inputs_ready"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for PortfolioInputsReadyState {
    fn run(
        &self,
        _input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(PortfolioInputsReady::new(
            self.config.bitcoin_network_count(),
            self.config.evm_network_count(),
        ))
    }
}

/// State that resolves configured wallet subjects.
pub struct ResolveSubjectsState {
    config: ResolveSubjectsConfig,
}

impl StateSpec for ResolveSubjectsState {
    type Config = ResolveSubjectsConfig;
    type Context = NoContext;
    type Input = PortfolioInputsReady;
    type Output = ResolvedSubjects;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("resolve_subjects")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("resolve_subjects")
    }

    fn name() -> &'static str {
        "mfm.portfolio.resolve_subjects"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ResolveSubjectsState {
    fn run(
        &self,
        _input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(resolve_subjects_from_config(&self.config))
    }
}

/// State that selects required holdings from Platform facts (adapter-bound).
pub struct SelectHoldingsState {
    config: SelectHoldingsConfig,
}

impl SelectHoldingsState {
    /// Returns the validated config.
    pub const fn config(&self) -> &SelectHoldingsConfig {
        &self.config
    }

    /// Expands required holdings from config + resolved subjects.
    pub fn expand_requirements(
        &self,
        subjects: &ResolvedSubjects,
    ) -> Result<Vec<RequiredHoldingRequirement>, PortfolioHoldingSelectionError> {
        expand_required_holdings(&self.config, subjects)
    }

    /// Builds selection evidence for one holding query using selected claim ids.
    pub fn selection_evidence_for_claims(
        &self,
        row_claim_ids: &[String],
        selected_claim_ids: &BTreeSet<String>,
    ) -> Result<FactSelectionEvidence, PortfolioHoldingSelectionError> {
        let mut selected_indices = Vec::new();
        for (index, claim_id) in row_claim_ids.iter().enumerate() {
            if selected_claim_ids.contains(claim_id) {
                selected_indices.push(index as u64);
            }
        }
        FactSelectionEvidence::new(
            portfolio_holding_selection_policy_digest(),
            selected_indices,
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
    type Input = ResolvedSubjects;
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

/// State that resolves configured fixed unit-price valuation routes.
pub struct ResolveValuationsState {
    config: ResolveValuationsConfig,
}

impl StateSpec for ResolveValuationsState {
    type Config = ResolveValuationsConfig;
    type Context = NoContext;
    type Input = ();
    type Output = ResolvedValuations;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("resolve_valuations")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("resolve_valuations")
    }

    fn name() -> &'static str {
        "mfm.portfolio.resolve_valuations"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ResolveValuationsState {
    fn run(
        &self,
        _input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        resolve_valuations_from_config(&self.config)
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
        assemble_snapshot(&self.config, input, 0)
    }
}

/// State that projects the canonical public report from a snapshot.
pub struct ProjectReportState {
    config: ProjectReportConfig,
}

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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ProjectReportState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        project_report_from_snapshot(input.snapshot, self.config.report_version())
    }
}
