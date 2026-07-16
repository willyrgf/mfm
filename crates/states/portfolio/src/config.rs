use super::*;

/// Config for subject resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.resolve_subjects",
    validate = "validate_resolve_subjects_config"
)]
pub struct ResolveSubjectsConfig {
    /// The normalized aggregate portfolio authority.
    pub(super) portfolio: PortfolioConfig,
}

impl ResolveSubjectsConfig {
    /// Creates validated subject-resolution config from the sole portfolio authority.
    pub fn new(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self { portfolio };
        validate_resolve_subjects_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the normalized aggregate portfolio authority.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

/// Config for Platform fact-backed holding selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.select_holdings",
    validate = "validate_select_holdings_config"
)]
pub struct SelectHoldingsConfig {
    /// Portfolio requirements used for subject projection.
    pub(super) portfolio: PortfolioConfig,
    /// Store scope for Platform fact-index reads.
    pub(super) store_scope: String,
    /// Fixed certified selection policy id.
    pub(super) selection_policy_id: String,
    /// Fixed N + 1 scan limit used to prove candidate exhaustion.
    pub(super) candidate_scan_limit: NonZeroU64,
}

impl SelectHoldingsConfig {
    /// Creates validated receipt-pinned selection config with closed version-1 policy values.
    pub fn new(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self {
            portfolio,
            store_scope: PORTFOLIO_STORE_SCOPE.to_owned(),
            selection_policy_id: PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID.to_owned(),
            candidate_scan_limit: NonZeroU64::new(PORTFOLIO_FACT_SCAN_LIMIT)
                .expect("portfolio receipt scan limit is non-zero"),
        };
        validate_select_holdings_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the portfolio requirements.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns the store scope.
    pub fn store_scope(&self) -> &str {
        &self.store_scope
    }

    /// Returns the certified selection policy id.
    pub fn selection_policy_id(&self) -> &str {
        &self.selection_policy_id
    }

    /// Returns the fixed N + 1 candidate scan limit.
    pub const fn candidate_scan_limit(&self) -> u64 {
        self.candidate_scan_limit.get()
    }

    /// Returns the fixed number of candidates permitted before exhaustion proof fails.
    pub const fn candidate_bound(&self) -> u64 {
        PORTFOLIO_FACT_CANDIDATE_BOUND
    }
}

/// Config for valuation resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.resolve_valuations",
    validate = "validate_resolve_valuations_config"
)]
pub struct ResolveValuationsConfig {
    /// The normalized aggregate portfolio authority.
    pub(super) portfolio: PortfolioConfig,
}

impl ResolveValuationsConfig {
    /// Creates validated valuation-resolution config from the sole portfolio authority.
    pub fn new(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self { portfolio };
        validate_resolve_valuations_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the normalized aggregate portfolio authority.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

/// Config for snapshot assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.config.assemble_snapshot",
    validate = "validate_assemble_snapshot_config"
)]
pub struct AssembleSnapshotConfig {
    /// Portfolio config carried into the public snapshot.
    pub(super) portfolio: PortfolioConfig,
}

impl AssembleSnapshotConfig {
    /// Creates validated snapshot-assembly config.
    pub fn new(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self { portfolio };
        validate_assemble_snapshot_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the portfolio config carried into the public snapshot.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

/// Config for report projection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.config.project_report")]
pub struct ProjectReportConfig {}

fn validate_resolve_subjects_config(config: &ResolveSubjectsConfig) -> Result<(), String> {
    validate_normalized_portfolio(&config.portfolio)
}

fn validate_select_holdings_config(config: &SelectHoldingsConfig) -> Result<(), String> {
    validate_normalized_portfolio(&config.portfolio)?;
    StoreScopeRef::new(&config.store_scope).map_err(|error| error.to_string())?;
    if config.store_scope != PORTFOLIO_STORE_SCOPE {
        return Err("portfolio store scope did not match the closed version-1 policy".to_owned());
    }
    if config.selection_policy_id != PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID {
        return Err(format!(
            "unsupported selection policy id `{}`",
            config.selection_policy_id
        ));
    }
    if config.candidate_scan_limit.get() != PORTFOLIO_FACT_SCAN_LIMIT {
        return Err(
            "portfolio candidate scan limit did not match the closed version-1 policy".to_owned(),
        );
    }
    Ok(())
}

fn validate_resolve_valuations_config(config: &ResolveValuationsConfig) -> Result<(), String> {
    validate_normalized_portfolio(&config.portfolio)
}

fn validate_assemble_snapshot_config(config: &AssembleSnapshotConfig) -> Result<(), String> {
    validate_normalized_portfolio(&config.portfolio)
}

fn validate_normalized_portfolio(portfolio: &PortfolioConfig) -> Result<(), String> {
    let normalized = ValidatedPortfolioConfig::new(portfolio.clone())
        .map_err(|error| error.to_string())?
        .into_config();
    if normalized != *portfolio {
        return Err(
            "portfolio config must be normalized before it is used as state authority".to_owned(),
        );
    }
    Ok(())
}
