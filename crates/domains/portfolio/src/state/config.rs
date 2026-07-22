use super::*;

/// Certified family holding fact descriptors consumed by selection.
///
/// Descriptors are carried as canonical JSON so certified config fixes the exact query and
/// hydration authority. The portfolio operation supplies the concrete registered descriptors when
/// it authors the state config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "select_holdings_fact_descriptors",
    schema = "mfm.portfolio.config.select_holdings_fact_descriptors"
)]
pub struct SelectHoldingsFactDescriptors {
    bitcoin_native: String,
    evm_balance: String,
}

impl SelectHoldingsFactDescriptors {
    /// Canonicalizes and admits the registered descriptors used by the portfolio operation.
    pub fn new(
        bitcoin_native: &mfm_facts::FactDescriptor,
        evm_balance: &mfm_facts::FactDescriptor,
    ) -> Result<Self, ConfigError> {
        let descriptors = Self {
            bitcoin_native: canonical_descriptor_json(bitcoin_native)?,
            evm_balance: canonical_descriptor_json(evm_balance)?,
        };
        descriptors.validate().map_err(ConfigError::new)?;
        Ok(descriptors)
    }

    pub(crate) fn bitcoin_native(
        &self,
    ) -> Result<mfm_facts::FactDescriptor, PortfolioHoldingSelectionError> {
        self.descriptor(&self.bitcoin_native, "bitcoin.balance_snapshot", "Bitcoin")
    }

    pub(crate) fn evm_balance(
        &self,
    ) -> Result<mfm_facts::FactDescriptor, PortfolioHoldingSelectionError> {
        self.descriptor(&self.evm_balance, "evm.balance_snapshot", "EVM")
    }

    fn descriptor(
        &self,
        canonical: &str,
        expected_kind: &str,
        family: &str,
    ) -> Result<mfm_facts::FactDescriptor, PortfolioHoldingSelectionError> {
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(canonical.as_bytes())
            .map_err(|error| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::ReceiptMismatch,
                    error.to_string(),
                    None,
                    None,
                )
            })?;
        if descriptor.fact_kind().as_str() != expected_kind {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                format!("{family} fact descriptor kind did not match its source family"),
                None,
                None,
            ));
        }
        Ok(descriptor)
    }

    fn validate(&self) -> Result<(), String> {
        self.bitcoin_native().map_err(|error| error.to_string())?;
        self.evm_balance().map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn canonical_descriptor_json(
    descriptor: &mfm_facts::FactDescriptor,
) -> Result<String, ConfigError> {
    let bytes = mfm_facts::canonical_fact_descriptor_bytes(descriptor)
        .map_err(|error| ConfigError::new(error.to_string()))?;
    String::from_utf8(bytes.as_bytes().to_vec())
        .map_err(|error| ConfigError::new(error.to_string()))
}

/// Config for Platform fact-backed holding selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.config.select_holdings",
    validate = "validate_select_holdings_config"
)]
pub struct SelectHoldingsConfig {
    /// Portfolio requirements used for subject projection.
    pub(super) portfolio: PortfolioConfig,
    /// Fixed certified selection policy id.
    pub(super) selection_policy_id: String,
    /// Registered fact descriptors that define query and hydration authority.
    pub(super) fact_descriptors: SelectHoldingsFactDescriptors,
}

impl SelectHoldingsConfig {
    /// Creates validated receipt-pinned selection config with closed version-1 policy values.
    pub fn new(
        portfolio: PortfolioConfig,
        fact_descriptors: SelectHoldingsFactDescriptors,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            portfolio,
            selection_policy_id: PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID.to_owned(),
            fact_descriptors,
        };
        validate_select_holdings_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the portfolio requirements.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns the certified selection policy id.
    pub fn selection_policy_id(&self) -> &str {
        &self.selection_policy_id
    }

    /// Returns the certified holding fact descriptor set.
    pub const fn fact_descriptors(&self) -> &SelectHoldingsFactDescriptors {
        &self.fact_descriptors
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

fn validate_select_holdings_config(config: &SelectHoldingsConfig) -> Result<(), String> {
    validate_normalized_portfolio(&config.portfolio)?;
    if config.selection_policy_id != PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID {
        return Err(format!(
            "unsupported selection policy id `{}`",
            config.selection_policy_id
        ));
    }
    config.fact_descriptors.validate()?;
    Ok(())
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
