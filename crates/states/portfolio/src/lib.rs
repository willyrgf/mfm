#![warn(missing_docs)]
//! Typed portfolio-domain state contracts for fact-backed report-only snapshots.
//!
//! After the collectors cutover, `portfolio_snapshot` is select-centric:
//! `ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_config::PortfolioSnapshotCanonicalConfig;
//! use mfm_state_portfolio::PortfolioWorkflowConfig;
//!
//! fn workflow_config(canonical: PortfolioSnapshotCanonicalConfig) -> PortfolioWorkflowConfig {
//!     PortfolioWorkflowConfig::from(canonical)
//! }
//! ```

mod selection;

pub use selection::{
    holding_candidate_from_normalized, is_filter_empty_holding_error,
    portfolio_holding_select_scope_decision_hash, portfolio_holding_selection_policy_digest,
    project_holding_fact_for_network, project_holding_fact_kind,
    project_network_pins_from_observations, select_network_coherent, HoldingAnchor,
    HoldingCandidate, HoldingFactProjection, NormalizedHoldingFields, PortfolioHoldingErrorCode,
    PortfolioHoldingSelectionError, RequiredHoldingKey, SelectedHolding, SelectedHoldingMaterial,
    PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID,
};

use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::num::NonZeroU64;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::NoCaps;
use mfm_effects::{Pure, ReadExternal};
use mfm_fact_capabilities::FactIndexReadCapability;
use mfm_facts::{FactSelectionEvidence, StoreScopeRef};
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_portfolio_config::PortfolioSnapshotCanonicalConfig;
use mfm_portfolio_model::aave::AAVE_V3_PROTOCOL_ID;
use mfm_portfolio_model::portfolio::{
    NetworkConfig, NetworkFamilyConfig, PortfolioConfig, PortfolioQuoteTotal, PortfolioReport,
    PortfolioSnapshot, ValidatedPortfolioConfig, ValidatedSymbolConfigs, ValidatedWalletConfigs,
    WalletReport, WalletSnapshot,
};
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, Observation, ObservationQuantity, ObservationSource, ObservationValue,
    QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
};
use mfm_portfolio_model::wallet::{WalletConfig, WalletImplementationConfig, WalletSubjectKind};
use mfm_program::{
    AdapterBindingSpec, NoContext, PureState, ReadState, StateError, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use mfm_values::ConfigError;
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.portfolio";
const ADAPTER_NAME: &str = "typed-portfolio";
const ADAPTER_VERSION: &str = "mfm.portfolio.adapter.typed.v1";
/// Default store scope used by portfolio Platform holding selection.
pub const DEFAULT_PORTFOLIO_STORE_SCOPE: &str = "mfm.store.default";

/// Returns the typed portfolio adapter kind.
pub fn portfolio_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.portfolio.adapter:typed-portfolio"),
    )
}

/// Returns the typed portfolio adapter version.
pub fn portfolio_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: portfolio_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("portfolio adapter kind invalid: {error}"))
        })?,
        adapter_version: portfolio_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("portfolio adapter version invalid: {error}"))
        })?,
    }])
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.portfolio.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.portfolio.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Root workflow config for the portfolio snapshot operation.
#[derive(Debug, Clone, Serialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.workflow",
    validate = "validate_portfolio_workflow_config"
)]
pub struct PortfolioWorkflowConfig {
    /// Canonical portfolio config.
    portfolio: PortfolioConfig,
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
    wallets: Vec<WalletConfig>,
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
    portfolio: PortfolioConfig,
    /// Store scope for Platform fact-index reads.
    store_scope: String,
    /// Fixed certified selection policy id.
    selection_policy_id: String,
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
    symbol_configs: Vec<SymbolConfig>,
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
    snapshot_version: NonZeroU64,
    /// Portfolio config carried into the public snapshot.
    portfolio: PortfolioConfig,
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
    report_version: NonZeroU64,
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

/// Resolved wallet subject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-subject",
    schema = "mfm.portfolio.resolved_subject"
)]
pub struct ResolvedSubject {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical resolved address.
    pub address: String,
    /// Subject kind.
    pub subject_kind: WalletSubjectKind,
    /// Stable network identifier.
    pub network_id: String,
    /// Stable wallet implementation kind.
    pub implementation_kind: String,
}

/// Resolved subject collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-subjects",
    schema = "mfm.portfolio.resolved_subjects"
)]
pub struct ResolvedSubjects {
    /// Subjects in canonical wallet order.
    pub subjects: Vec<ResolvedSubject>,
}

/// Resolved unit-price valuation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-valuation",
    schema = "mfm.portfolio.resolved_valuation"
)]
pub struct ResolvedValuation {
    /// Symbol whose quote route was resolved.
    pub symbol_id: String,
    /// Quote unit.
    pub quote: QuoteCode,
    /// Symbol whose unit price applies to this route.
    pub priced_symbol_id: String,
    /// Decimal-string unit price.
    pub unit_price_dec: String,
}

/// Resolved valuation collection (hard-fail: empty only when no symbols; no soft errors).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-valuations",
    schema = "mfm.portfolio.resolved_valuations"
)]
pub struct ResolvedValuations {
    /// Valuations in canonical symbol/quote order.
    pub valuations: Vec<ResolvedValuation>,
}

/// Selected holdings material emitted by SelectHoldings (observations without valuation join).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "selected-holdings",
    schema = "mfm.portfolio.selected_holdings"
)]
pub struct SelectedHoldings {
    /// Selected observations in canonical wallet/symbol order. `values` may be empty until assemble.
    pub observations: Vec<Observation>,
}

/// Input consumed by typed snapshot assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.assemble_snapshot")]
pub struct AssembleSnapshotInput {
    /// Resolved wallet subjects.
    pub subjects: ResolvedSubjects,
    /// Selected holdings from Platform fact selection.
    pub holdings: SelectedHoldings,
    /// Resolved fixed unit-price valuations.
    pub valuations: ResolvedValuations,
}

/// Input consumed by report projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.project_report")]
pub struct ProjectReportInput {
    /// Assembled portfolio snapshot.
    pub snapshot: PortfolioSnapshot,
}

/// Public output contract for portfolio workflows.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.portfolio.public_outputs")]
pub struct PortfolioPublicOutputs<'program, 'scope> {
    /// Public portfolio snapshot.
    pub snapshot: mfm_program::Handle<'program, 'scope, PortfolioSnapshot>,
    /// Projected portfolio report.
    pub report: mfm_program::Handle<'program, 'scope, PortfolioReport>,
}

/// Operation output handles produced by the typed portfolio workflow.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.portfolio.operation_outputs")]
pub struct PortfolioOperationOutputs<'program, 'scope> {
    /// Public portfolio snapshot.
    pub snapshot: mfm_program::Handle<'program, 'scope, PortfolioSnapshot>,
    /// Projected portfolio report.
    pub report: mfm_program::Handle<'program, 'scope, PortfolioReport>,
}

/// State that resolves configured wallet subjects.
pub struct ResolveSubjectsState {
    config: ResolveSubjectsConfig,
}

impl StateSpec for ResolveSubjectsState {
    type Config = ResolveSubjectsConfig;
    type Context = NoContext;
    type Input = ();
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

/// One required holding expanded from portfolio config + resolved subjects.
#[derive(Debug, Clone, PartialEq)]
pub struct RequiredHoldingRequirement {
    /// Selection key.
    pub key: RequiredHoldingKey,
    /// Cutover fact projection kind.
    pub projection: HoldingFactProjection,
    /// Wallet address for subject predicates.
    pub address: String,
    /// Symbol config for observation join.
    pub symbol: SymbolConfig,
    /// Network config for subject predicates.
    pub network: NetworkConfig,
}

/// Resolves configured wallets into typed subjects.
pub fn resolve_subjects_from_config(config: &ResolveSubjectsConfig) -> ResolvedSubjects {
    let mut subjects = config
        .wallets
        .iter()
        .map(|wallet| ResolvedSubject {
            wallet_id: wallet.wallet_id.to_string(),
            address: wallet.subject.address_str().to_owned(),
            subject_kind: wallet.subject.kind(),
            network_id: wallet.network_id.to_string(),
            implementation_kind: wallet_implementation_kind(&wallet.implementation).to_owned(),
        })
        .collect::<Vec<_>>();
    subjects.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
    ResolvedSubjects { subjects }
}

/// Expands wallet×symbol requirements into cutover-supported holding requirements.
pub fn expand_required_holdings(
    config: &SelectHoldingsConfig,
    subjects: &ResolvedSubjects,
) -> Result<Vec<RequiredHoldingRequirement>, PortfolioHoldingSelectionError> {
    let networks = networks_by_id(&config.portfolio.networks)?;
    let symbols = symbols_by_id(&config.portfolio.symbol_configs)?;
    let subjects_by_wallet = subjects
        .subjects
        .iter()
        .map(|subject| (subject.wallet_id.as_str(), subject))
        .collect::<BTreeMap<_, _>>();

    let mut requirements = Vec::new();
    for wallet in &config.portfolio.wallets {
        let subject = subjects_by_wallet
            .get(wallet.wallet_id.as_str())
            .copied()
            .ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!("missing resolved subject for wallet {}", wallet.wallet_id),
                    Some(wallet.wallet_id.to_string()),
                    Some(wallet.network_id.to_string()),
                )
            })?;
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!(
                        "wallet `{}` referenced unknown symbol `{symbol_id}`",
                        wallet.wallet_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol_id)),
                    Some(wallet.network_id.to_string()),
                )
            })?;
            let network = networks
                .get(symbol.network_id.as_str())
                .copied()
                .ok_or_else(|| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        format!(
                            "missing network `{}` for symbol `{}`",
                            symbol.network_id, symbol.symbol_id
                        ),
                        Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id)),
                        Some(symbol.network_id.to_string()),
                    )
                })?;
            let family = match network.family() {
                NetworkFamilyConfig::Bitcoin => "bitcoin",
                NetworkFamilyConfig::Evm => "evm",
            };
            let is_native = matches!(
                (&symbol.kind, &symbol.balance_reader),
                (
                    SymbolKind::NativeBalance,
                    BalanceReaderConfig::NativeBalance {}
                )
            );
            if matches!(
                (&symbol.kind, &symbol.balance_reader),
                (
                    SymbolKind::Erc20Balance,
                    BalanceReaderConfig::Erc20Balance { .. }
                )
            ) {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!(
                        "ERC-20 symbol `{}` is not supported at cutover",
                        symbol.symbol_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id)),
                    Some(symbol.network_id.to_string()),
                ));
            }
            let projection =
                project_holding_fact_for_network(family, is_native).map_err(|mut err| {
                    err.holding_key = Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id));
                    err.network_id = Some(symbol.network_id.to_string());
                    err
                })?;
            requirements.push(RequiredHoldingRequirement {
                key: RequiredHoldingKey {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol.symbol_id.to_string(),
                    network_id: symbol.network_id.to_string(),
                },
                projection,
                address: subject.address.clone(),
                symbol: symbol.clone(),
                network: network.clone(),
            });
        }
    }
    requirements.sort_by(|left, right| {
        (
            left.key.network_id.as_str(),
            left.key.wallet_id.as_str(),
            left.key.symbol_id.as_str(),
        )
            .cmp(&(
                right.key.network_id.as_str(),
                right.key.wallet_id.as_str(),
                right.key.symbol_id.as_str(),
            ))
    });
    Ok(requirements)
}

/// Builds quantity-only observations from selected holdings (valuation join deferred).
pub fn observations_from_selected_holdings(
    selected: &[SelectedHolding],
    symbols_by_id: &BTreeMap<&str, &SymbolConfig>,
) -> Result<Vec<Observation>, PortfolioHoldingSelectionError> {
    let mut observations = Vec::with_capacity(selected.len());
    for item in selected {
        let symbol = symbols_by_id
            .get(item.key.symbol_id.as_str())
            .copied()
            .ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!("missing symbol config for {}", item.key.symbol_id),
                    Some(item.key.as_key_str()),
                    Some(item.key.network_id.clone()),
                )
            })?;
        let amount_dec = amount_dec_from_raw(&item.material.raw_dec, item.material.decimals)?;
        let mut observation = Observation {
            wallet_id: item.material.wallet_id.clone(),
            symbol_id: item.material.symbol_id.clone(),
            display_symbol: symbol.display_symbol.clone(),
            kind: symbol.kind,
            role: symbol.role,
            network_id: item.material.network_id.clone(),
            protocol: symbol.protocol.as_ref().map(ToString::to_string),
            quantity: ObservationQuantity {
                raw_dec: item.material.raw_dec.clone(),
                decimals: item.material.decimals,
                amount_dec,
            },
            values: Vec::new(),
            source: ObservationSource {
                balance_reader_kind: item.material.balance_reader_kind.clone(),
                network_id: item.material.network_id.clone(),
                anchor: item.material.observation_anchor.clone(),
            },
            coverage: item.material.coverage.clone(),
            metadata: symbol.metadata.clone(),
        };
        observation.normalize();
        observations.push(observation);
    }
    Ok(observations)
}

/// Joins fixed unit-price valuations onto observations (hard-fail on missing route).
pub fn apply_valuations_to_observations(
    mut observations: Vec<Observation>,
    valuations: &ResolvedValuations,
    symbols_by_id: &BTreeMap<&str, &SymbolConfig>,
) -> StateResult<Vec<Observation>> {
    for observation in &mut observations {
        let symbol = symbols_by_id
            .get(observation.symbol_id.as_str())
            .copied()
            .ok_or_else(|| {
                StateError::Message(format!(
                    "missing symbol config for observation {}",
                    observation.symbol_id
                ))
            })?;
        let mut values = Vec::new();
        for quote in &symbol.valuation.quotes {
            let resolved = valuations
                .valuations
                .iter()
                .find(|valuation| {
                    valuation.symbol_id == observation.symbol_id.as_str()
                        && valuation.quote == quote.quote
                })
                .ok_or_else(|| {
                    StateError::Message(format!(
                        "missing_fixed_unit_price: missing valuation for symbol `{}` quote `{}`",
                        observation.symbol_id, quote.quote
                    ))
                })?;
            let value_dec = multiply_decimal_strings(
                &observation.quantity.amount_dec,
                &resolved.unit_price_dec,
            )
            .map_err(|error| StateError::Message(error.to_string()))?;
            values.push(ObservationValue {
                quote: resolved.quote,
                priced_symbol_id: resolved.priced_symbol_id.clone(),
                value_dec,
                unit_price_dec: resolved.unit_price_dec.clone(),
            });
        }
        values.sort_by_key(|value| value.quote);
        observation.values = values;
        observation.normalize();
    }
    Ok(observations)
}

/// Resolves configured fixed unit-price valuation routes (hard-fail).
pub fn resolve_valuations_from_config(
    config: &ResolveValuationsConfig,
) -> StateResult<ResolvedValuations> {
    let mut valuations = Vec::new();
    for symbol in &config.symbol_configs {
        for quote in &symbol.valuation.quotes {
            valuations.push(resolved_valuation_for_quote(symbol, quote)?);
        }
    }
    valuations.sort_by(|left, right| {
        (left.symbol_id.as_str(), left.quote).cmp(&(right.symbol_id.as_str(), right.quote))
    });
    Ok(ResolvedValuations { valuations })
}

fn resolved_valuation_for_quote(
    symbol: &SymbolConfig,
    quote: &QuoteValuationConfig,
) -> StateResult<ResolvedValuation> {
    Ok(ResolvedValuation {
        symbol_id: symbol.symbol_id.to_string(),
        quote: quote.quote,
        priced_symbol_id: quote.priced_symbol_id.to_string(),
        unit_price_dec: quote.unit_price_dec.to_string(),
    })
}

/// Hard-fail when any configured wallet×symbol required holding lacks an observation.
///
/// Defense-in-depth for the pure assemble path: SelectHoldings is the graph authority, but
/// assemble must not emit a successful snapshot with empty/partial required holdings.
fn require_required_holdings_present(
    portfolio: &PortfolioConfig,
    observations: &[Observation],
) -> Result<(), PortfolioHoldingSelectionError> {
    let present: BTreeSet<(String, String)> = observations
        .iter()
        .map(|observation| (observation.wallet_id.clone(), observation.symbol_id.clone()))
        .collect();
    for wallet in &portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let key = (wallet.wallet_id.to_string(), symbol_id.to_string());
            if !present.contains(&key) {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!(
                        "required holding missing observation for wallet `{}` symbol `{symbol_id}`",
                        wallet.wallet_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol_id)),
                    Some(wallet.network_id.to_string()),
                ));
            }
        }
    }
    Ok(())
}

/// Assembles the canonical portfolio snapshot (hard-fail; pins from selected observations).
pub fn assemble_snapshot(
    config: &AssembleSnapshotConfig,
    input: AssembleSnapshotInput,
    generated_at_ms: u64,
) -> StateResult<PortfolioSnapshot> {
    let symbols = symbols_by_id(&config.portfolio.symbol_configs).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let observations =
        apply_valuations_to_observations(input.holdings.observations, &input.valuations, &symbols)?;
    require_required_holdings_present(&config.portfolio, &observations).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let network_pins = project_network_pins_from_observations(&observations).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;

    let mut observations_by_wallet: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    for observation in observations {
        observations_by_wallet
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    let mut subjects_by_wallet = BTreeMap::new();
    for subject in input.subjects.subjects {
        subjects_by_wallet.insert(subject.wallet_id.clone(), subject);
    }

    let mut wallets = Vec::with_capacity(config.portfolio.wallets.len());
    for wallet_cfg in &config.portfolio.wallets {
        let subject = subjects_by_wallet
            .get(wallet_cfg.wallet_id.as_str())
            .cloned()
            .unwrap_or_else(|| ResolvedSubject {
                wallet_id: wallet_cfg.wallet_id.to_string(),
                address: wallet_cfg.subject.address_str().to_owned(),
                subject_kind: wallet_cfg.subject.kind(),
                network_id: wallet_cfg.network_id.to_string(),
                implementation_kind: wallet_implementation_kind(&wallet_cfg.implementation)
                    .to_owned(),
            });
        // Required holdings already verified; absent wallet key means zero symbols configured.
        let observations = observations_by_wallet
            .remove(wallet_cfg.wallet_id.as_str())
            .unwrap_or_default();
        let mut wallet = WalletSnapshot {
            wallet_id: wallet_cfg.wallet_id.to_string(),
            address: subject.address,
            subject_kind: subject.subject_kind,
            network_id: subject.network_id,
            observations,
        };
        wallet.normalize();
        wallets.push(wallet);
    }
    wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

    let mut symbol_configs = config.portfolio.symbol_configs.clone();
    for symbol in &mut symbol_configs {
        symbol.normalize();
    }
    symbol_configs.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));

    let mut snapshot = PortfolioSnapshot {
        schema_version: config.snapshot_version(),
        portfolio_id: config.portfolio.portfolio_id.to_string(),
        generated_at_ms,
        network_pins,
        wallets,
        symbol_configs,
    };
    snapshot.normalize();
    Ok(snapshot)
}

/// Projects a canonical portfolio report from a snapshot.
pub fn project_report_from_snapshot(
    snapshot: PortfolioSnapshot,
    report_version: u64,
) -> StateResult<PortfolioReport> {
    let report_quotes = collect_report_quotes(&snapshot);
    let mut portfolio_totals = initialized_quote_totals(&report_quotes);
    let wallet_summaries = snapshot
        .wallets
        .iter()
        .map(|wallet| {
            let wallet_totals = derive_quote_totals(&report_quotes, &wallet.observations)?;
            merge_quote_totals(&mut portfolio_totals, &wallet_totals);
            Ok(WalletReport {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
                totals_by_quote: quote_totals_to_vec(wallet_totals),
            })
        })
        .collect::<StateResult<Vec<_>>>()?;

    let mut report = PortfolioReport {
        schema_version: report_version,
        portfolio_id: snapshot.portfolio_id,
        generated_at_ms: snapshot.generated_at_ms,
        network_pins: snapshot.network_pins,
        wallet_summaries,
        totals_by_quote: quote_totals_to_vec(portfolio_totals),
    };
    report.normalize();
    Ok(report)
}

/// Returns the canonical balance reader kind string.
pub fn balance_reader_kind(reader: &BalanceReaderConfig) -> &'static str {
    match reader {
        BalanceReaderConfig::NativeBalance {} => "native_balance",
        BalanceReaderConfig::Erc20Balance { .. } => "erc20_balance",
        BalanceReaderConfig::ProtocolPosition {
            protocol, reader, ..
        } if protocol.as_str() == AAVE_V3_PROTOCOL_ID => match reader.as_str() {
            "reserve_position" => "aave_v3/reserve_position",
            "debt_position" => "aave_v3/debt_position",
            _ => "protocol_position",
        },
        BalanceReaderConfig::ProtocolPosition { .. } => "protocol_position",
    }
}

/// Builds a symbols-by-id index for assemble / observation join.
pub fn symbols_by_id_map(
    symbols: &[SymbolConfig],
) -> Result<BTreeMap<&str, &SymbolConfig>, PortfolioHoldingSelectionError> {
    symbols_by_id(symbols)
}

fn networks_by_id(
    networks: &[NetworkConfig],
) -> Result<BTreeMap<&str, &NetworkConfig>, PortfolioHoldingSelectionError> {
    let mut by_id = BTreeMap::new();
    for network in networks {
        if by_id
            .insert(network.network_id().as_str(), network)
            .is_some()
        {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::UnsupportedRequirement,
                format!("duplicate network id `{}`", network.network_id()),
                None,
                Some(network.network_id().to_string()),
            ));
        }
    }
    Ok(by_id)
}

fn symbols_by_id(
    symbols: &[SymbolConfig],
) -> Result<BTreeMap<&str, &SymbolConfig>, PortfolioHoldingSelectionError> {
    let mut by_id = BTreeMap::new();
    for symbol in symbols {
        if by_id.insert(symbol.symbol_id.as_str(), symbol).is_some() {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::UnsupportedRequirement,
                format!("duplicate symbol id `{}`", symbol.symbol_id),
                None,
                Some(symbol.network_id.to_string()),
            ));
        }
    }
    Ok(by_id)
}

fn amount_dec_from_raw(
    raw_dec: &str,
    decimals: u8,
) -> Result<String, PortfolioHoldingSelectionError> {
    if raw_dec.is_empty() || !raw_dec.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            format!("invalid raw balance decimal `{raw_dec}`"),
            None,
            None,
        ));
    }
    Ok(format_decimal_amount(raw_dec, decimals))
}

fn format_decimal_amount(raw_dec: &str, decimals: u8) -> String {
    let d = decimals as usize;
    if d == 0 {
        return raw_dec.to_owned();
    }
    if raw_dec.len() <= d {
        format!("0.{}{}", "0".repeat(d - raw_dec.len()), raw_dec)
    } else {
        let split = raw_dec.len() - d;
        format!("{}.{}", &raw_dec[..split], &raw_dec[split..])
    }
}

fn wallet_implementation_kind(implementation: &WalletImplementationConfig) -> &'static str {
    match implementation {
        WalletImplementationConfig::AddressOnly {} => "address_only",
        WalletImplementationConfig::KeystoreEntry { .. } => "keystore_entry",
        WalletImplementationConfig::NodeManagedAccount { .. } => "node_managed_account",
        WalletImplementationConfig::ExternalSigner { .. } => "external_signer",
    }
}

fn collect_report_quotes(snapshot: &PortfolioSnapshot) -> Vec<QuoteCode> {
    let mut quotes = BTreeSet::new();
    for symbol in &snapshot.symbol_configs {
        for quote in &symbol.valuation.quotes {
            quotes.insert(quote.quote);
        }
    }
    for wallet in &snapshot.wallets {
        for observation in &wallet.observations {
            for value in &observation.values {
                quotes.insert(value.quote);
            }
        }
    }
    quotes.into_iter().collect()
}

fn derive_quote_totals(
    report_quotes: &[QuoteCode],
    observations: &[Observation],
) -> StateResult<BTreeMap<QuoteCode, QuoteTotalsAccumulator>> {
    let mut totals = initialized_quote_totals(report_quotes);
    for observation in observations {
        for value in &observation.values {
            let entry = totals.entry(value.quote).or_default();
            let value_dec = DecimalValue::parse_signed(&value.value_dec)
                .map_err(|error| StateError::Message(error.to_string()))?;
            match observation.role {
                SymbolRole::Native | SymbolRole::Asset => {
                    entry.assets_value = entry.assets_value.add(&value_dec);
                }
                SymbolRole::Collateral => {
                    entry.collateral_value = entry.collateral_value.add(&value_dec);
                }
                SymbolRole::Debt => {
                    entry.debt_value = entry.debt_value.add(&value_dec);
                }
                SymbolRole::Staked => {
                    entry.staked_value = entry.staked_value.add(&value_dec);
                }
            }
        }
    }
    Ok(totals)
}

fn initialized_quote_totals(
    report_quotes: &[QuoteCode],
) -> BTreeMap<QuoteCode, QuoteTotalsAccumulator> {
    report_quotes
        .iter()
        .copied()
        .map(|quote| (quote, QuoteTotalsAccumulator::default()))
        .collect()
}

fn merge_quote_totals(
    target: &mut BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
    source: &BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) {
    for (quote, totals) in source {
        target
            .entry(*quote)
            .and_modify(|acc| acc.merge(totals))
            .or_insert_with(|| totals.clone());
    }
}

fn quote_totals_to_vec(
    totals: BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) -> Vec<PortfolioQuoteTotal> {
    totals
        .into_iter()
        .map(|(quote, totals)| totals.into_report_total(quote))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct QuoteTotalsAccumulator {
    assets_value: DecimalValue,
    collateral_value: DecimalValue,
    debt_value: DecimalValue,
    staked_value: DecimalValue,
}

impl QuoteTotalsAccumulator {
    fn merge(&mut self, other: &Self) {
        self.assets_value = self.assets_value.add(&other.assets_value);
        self.collateral_value = self.collateral_value.add(&other.collateral_value);
        self.debt_value = self.debt_value.add(&other.debt_value);
        self.staked_value = self.staked_value.add(&other.staked_value);
    }

    fn into_report_total(self, quote: QuoteCode) -> PortfolioQuoteTotal {
        let positive_value = self
            .assets_value
            .add(&self.collateral_value)
            .add(&self.staked_value);
        let net_value = positive_value.sub(&self.debt_value);
        PortfolioQuoteTotal {
            quote,
            assets_value_dec: self.assets_value.to_canonical_string(),
            collateral_value_dec: self.collateral_value.to_canonical_string(),
            debt_value_dec: self.debt_value.to_canonical_string(),
            staked_value_dec: self.staked_value.to_canonical_string(),
            net_value_dec: net_value.to_canonical_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum DecimalArithmeticError {
    #[error("invalid decimal string `{value}`")]
    InvalidDecimalString { value: String },
}

fn multiply_decimal_strings(left: &str, right: &str) -> Result<String, DecimalArithmeticError> {
    let left = DecimalValue::parse_non_negative(left)?;
    let right = DecimalValue::parse_non_negative(right)?;
    let product = DecimalValue {
        digits: left.digits * right.digits,
        scale: left.scale + right.scale,
    };
    Ok(product.to_string_with_min_scale(left.scale.max(right.scale)))
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl DecimalValue {
    fn parse_non_negative(input: &str) -> Result<Self, DecimalArithmeticError> {
        Self::parse(input, false)
    }

    fn parse_signed(input: &str) -> Result<Self, DecimalArithmeticError> {
        Self::parse(input, true)
    }

    fn parse(input: &str, allow_negative: bool) -> Result<Self, DecimalArithmeticError> {
        let trimmed = input.trim();
        let (negative, digits_part) = match trimmed.strip_prefix('-') {
            Some(rest) => {
                if !allow_negative {
                    return Err(DecimalArithmeticError::InvalidDecimalString {
                        value: input.to_owned(),
                    });
                }
                (true, rest)
            }
            None => (false, trimmed),
        };
        if digits_part.is_empty() {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_owned(),
            });
        }
        let (whole, frac) = digits_part.split_once('.').unwrap_or((digits_part, ""));
        if frac.contains('.')
            || (!whole.is_empty() && !whole.chars().all(|ch| ch.is_ascii_digit()))
            || (!frac.is_empty() && !frac.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_owned(),
            });
        }
        let digits = format!("{whole}{frac}");
        let parsed: BigInt = if digits.is_empty() {
            BigInt::from(0u8)
        } else {
            digits
                .parse()
                .map_err(|_| DecimalArithmeticError::InvalidDecimalString {
                    value: input.to_owned(),
                })?
        };
        Ok(Self {
            digits: if negative && !parsed.is_zero() {
                -parsed
            } else {
                parsed
            },
            scale: frac.len() as u32,
        })
    }

    fn add(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) + other.scaled_digits(scale),
            scale,
        }
    }

    fn sub(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) - other.scaled_digits(scale),
            scale,
        }
    }

    fn scaled_digits(&self, scale: u32) -> BigInt {
        if self.scale == scale {
            self.digits.clone()
        } else {
            &self.digits * ten_pow(scale - self.scale)
        }
    }

    fn to_canonical_string(&self) -> String {
        self.to_string_with_min_scale(self.scale)
    }

    fn to_string_with_min_scale(&self, min_scale: u32) -> String {
        let negative = self.digits.is_negative();
        let digits = self.digits.abs().to_string();
        let scale = self.scale as usize;
        let mut out = if scale == 0 {
            digits
        } else if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            let split = digits.len() - scale;
            format!("{}.{}", &digits[..split], &digits[split..])
        };

        if let Some((whole, frac)) = out.split_once('.') {
            let mut frac = frac.to_owned();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = format!("{whole}.{frac}");
        } else if min_scale > 0 {
            out.push('.');
            out.push_str(&"0".repeat(min_scale as usize));
        }

        if negative && out != "0" {
            format!("-{out}")
        } else {
            out
        }
    }
}

fn ten_pow(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}

#[cfg(test)]
mod tests;
