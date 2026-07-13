use super::*;

/// Root workflow config for the portfolio snapshot operation.
#[derive(Debug, Clone, Serialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.workflow",
    validate = "validate_portfolio_workflow_config"
)]
pub struct PortfolioWorkflowConfig {
    /// Canonical portfolio config.
    pub(super) portfolio: PortfolioConfig,
}

impl PortfolioWorkflowConfig {
    /// Creates a validated root portfolio workflow config.
    pub fn new(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self { portfolio };
        validate_portfolio_workflow_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the canonical portfolio config.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

impl<'de> Deserialize<'de> for PortfolioWorkflowConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawPortfolioWorkflowConfig {
            portfolio: PortfolioConfig,
        }

        let raw = RawPortfolioWorkflowConfig::deserialize(deserializer)?;
        Ok(Self {
            portfolio: raw.portfolio,
        })
    }
}

impl From<PortfolioSnapshotCanonicalConfig> for PortfolioWorkflowConfig {
    fn from(canonical: PortfolioSnapshotCanonicalConfig) -> Self {
        Self::new(canonical.portfolio).expect("canonical portfolio snapshot config must validate")
    }
}

/// Config for subject resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.resolve_subjects",
    validate = "validate_resolve_subjects_config"
)]
pub struct ResolveSubjectsConfig {
    /// Wallets to resolve.
    pub(super) wallets: Vec<WalletConfig>,
}

impl ResolveSubjectsConfig {
    /// Creates validated subject-resolution config.
    pub fn new(wallets: Vec<WalletConfig>) -> Result<Self, ConfigError> {
        let config = Self { wallets };
        validate_resolve_subjects_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the wallets to resolve.
    pub fn wallets(&self) -> &[WalletConfig] {
        &self.wallets
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
}

impl SelectHoldingsConfig {
    /// Creates validated select-holdings config with the cutover policy id.
    pub fn new(
        portfolio: PortfolioConfig,
        store_scope: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            portfolio,
            store_scope: store_scope.into(),
            selection_policy_id: PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID.to_owned(),
        };
        validate_select_holdings_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Creates config using the default store scope.
    pub fn with_default_store_scope(portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        Self::new(portfolio, DEFAULT_PORTFOLIO_STORE_SCOPE)
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
}

/// Config for valuation resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.resolve_valuations",
    validate = "validate_resolve_valuations_config"
)]
pub struct ResolveValuationsConfig {
    /// Symbols whose valuation routes should be resolved.
    pub(super) symbol_configs: Vec<SymbolConfig>,
}

impl ResolveValuationsConfig {
    /// Creates validated valuation-resolution config.
    pub fn new(symbol_configs: Vec<SymbolConfig>) -> Result<Self, ConfigError> {
        let config = Self { symbol_configs };
        validate_resolve_valuations_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the symbols whose valuation routes should be resolved.
    pub fn symbol_configs(&self) -> &[SymbolConfig] {
        &self.symbol_configs
    }
}

/// Config for snapshot assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.assemble_snapshot",
    validate = "validate_assemble_snapshot_config"
)]
pub struct AssembleSnapshotConfig {
    /// Snapshot schema version to emit.
    pub(super) snapshot_version: NonZeroU64,
    /// Portfolio config carried into the public snapshot.
    pub(super) portfolio: PortfolioConfig,
}

impl AssembleSnapshotConfig {
    /// Creates validated snapshot-assembly config.
    pub fn new(snapshot_version: u64, portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let snapshot_version = NonZeroU64::new(snapshot_version)
            .ok_or_else(|| ConfigError::new("snapshot schema version must be non-zero"))?;
        let config = Self {
            snapshot_version,
            portfolio,
        };
        validate_assemble_snapshot_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the snapshot schema version to emit.
    pub const fn snapshot_version(&self) -> u64 {
        self.snapshot_version.get()
    }

    /// Returns the portfolio config carried into the public snapshot.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

/// Config for report projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.portfolio.config.project_report")]
pub struct ProjectReportConfig {
    /// Report schema version to emit.
    pub(super) report_version: NonZeroU64,
}

impl ProjectReportConfig {
    /// Creates validated report-projection config.
    pub fn new(report_version: u64) -> Result<Self, ConfigError> {
        let report_version = NonZeroU64::new(report_version)
            .ok_or_else(|| ConfigError::new("report schema version must be non-zero"))?;
        Ok(Self { report_version })
    }

    /// Returns the report schema version to emit.
    pub const fn report_version(&self) -> u64 {
        self.report_version.get()
    }
}

fn validate_portfolio_workflow_config(config: &PortfolioWorkflowConfig) -> Result<(), String> {
    ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_resolve_subjects_config(config: &ResolveSubjectsConfig) -> Result<(), String> {
    ValidatedWalletConfigs::new(config.wallets.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_select_holdings_config(config: &SelectHoldingsConfig) -> Result<(), String> {
    ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())?;
    StoreScopeRef::new(&config.store_scope).map_err(|error| error.to_string())?;
    if config.selection_policy_id != PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID {
        return Err(format!(
            "unsupported selection policy id `{}`",
            config.selection_policy_id
        ));
    }
    Ok(())
}

fn validate_resolve_valuations_config(config: &ResolveValuationsConfig) -> Result<(), String> {
    ValidatedSymbolConfigs::new(config.symbol_configs.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_assemble_snapshot_config(config: &AssembleSnapshotConfig) -> Result<(), String> {
    ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}
