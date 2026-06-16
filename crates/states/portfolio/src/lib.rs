#![warn(missing_docs)]
//! Typed portfolio-domain state contracts.
//!
//! This crate owns the reusable typed state, value, input, and capability contracts for portfolio
//! snapshots. It intentionally exposes no legacy `DynContext`, `IoProvider`, `PlannedOp`,
//! `PortKey`, or hand-authored dynamic DAG surface.
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

use std::collections::{BTreeMap, BTreeSet};
use std::future::{self, Future};
use std::pin::Pin;

use alloy_primitives::U256;
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec, NoCaps, ReadExternalRole};
use mfm_effects::{Pure, ReadExternal};
use mfm_evm_core::encoding::format_u256_units;
use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, DigestAlgorithm, StateKind,
    StateVersion,
};
use mfm_portfolio_config::PortfolioSnapshotCanonicalConfig;
use mfm_portfolio_model::aave::AAVE_V3_PROTOCOL_ID;
use mfm_portfolio_model::portfolio::{
    validate_network_config, ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, NetworkPin,
    PortfolioConfig, PortfolioQuoteTotal, PortfolioReport, PortfolioSnapshot,
    PortfolioSnapshotError, ValidatedPortfolioBundle, ValidatedPortfolioConfig, WalletReport,
    WalletSnapshot,
};
use mfm_portfolio_model::symbol::{
    validate_symbol_config, validate_valuation_source_registry, BalanceReaderConfig, Observation,
    ObservationAnchor, ObservationQuantity, ObservationSource, ObservationValue,
    ObservationValueSourceRef, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolRole,
    ValuationReaderConfig, ValuationSourceRegistry,
};
use mfm_portfolio_model::wallet::{
    validate_wallet_config, WalletConfig, WalletImplementationConfig, WalletSubjectKind,
};
use mfm_program::{AdapterBindingSpec, PureState, ReadState, StateError, StateResult, StateSpec};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use mfm_values::{ConfigError, NonEmpty};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.portfolio";
const ADAPTER_NAME: &str = "typed-portfolio";
const ADAPTER_VERSION: &str = "mfm.portfolio.adapter.typed.v1";

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

fn capability_kind(name: &'static str) -> mfm_capabilities::Result<CapabilityKind> {
    CapabilityKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.portfolio.capability:{name}").as_bytes()),
    )
    .map_err(|error| CapabilityError::Identity(error.to_string()))
}

/// Read capability used by typed portfolio states that observe external chains.
pub struct PortfolioReadCapability;

impl CapabilitySpec for PortfolioReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("read")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.portfolio.capability.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.read"
    }
}

/// Future returned by portfolio read backends.
pub type PortfolioReadFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, PortfolioReadError>> + Send + 'a>>;

/// Redaction-safe external read error reported by portfolio read backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioReadError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Redacted human-readable message.
    pub message: String,
}

impl PortfolioReadError {
    /// Builds a redaction-safe portfolio read error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PortfolioReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PortfolioReadError {}

/// Runtime backend used by read portfolio states to observe external chain data.
pub trait PortfolioReadBackend: Send + Sync {
    /// Reads the current EVM block number for a network.
    fn evm_block_number<'a>(&'a self, network_id: &'a str) -> PortfolioReadFuture<'a, u64>;

    /// Reads a raw wallet/symbol balance and its decimals at a pinned block.
    fn observe_raw_balance<'a>(
        &'a self,
        config: &'a ObserveBatchConfig,
        block_number: u64,
    ) -> PortfolioReadFuture<'a, (U256, u8)>;
}

struct UnavailablePortfolioReadBackend;

impl PortfolioReadBackend for UnavailablePortfolioReadBackend {
    fn evm_block_number<'a>(&'a self, network_id: &'a str) -> PortfolioReadFuture<'a, u64> {
        Box::pin(async move {
            Err(PortfolioReadError::new(
                "external_read_required",
                format!("typed portfolio read backend unavailable for network `{network_id}`"),
            ))
        })
    }

    fn observe_raw_balance<'a>(
        &'a self,
        _config: &'a ObserveBatchConfig,
        _block_number: u64,
    ) -> PortfolioReadFuture<'a, (U256, u8)> {
        Box::pin(async {
            Err(PortfolioReadError::new(
                "external_read_required",
                "typed portfolio observation requires an external read backend",
            ))
        })
    }
}

/// Root typed portfolio workflow config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.workflow",
    validate = "validate_portfolio_workflow_config"
)]
pub struct PortfolioWorkflowConfig {
    /// Workflow config contract version.
    workflow_version: u64,
    /// Canonical portfolio config.
    portfolio: PortfolioConfig,
    /// Canonical valuation source registry.
    valuation_source_registry: ValuationSourceRegistry,
}

impl PortfolioWorkflowConfig {
    /// Creates a validated root portfolio workflow config.
    pub fn new(
        portfolio: PortfolioConfig,
        valuation_source_registry: ValuationSourceRegistry,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            workflow_version: 1,
            portfolio,
            valuation_source_registry,
        };
        validate_portfolio_workflow_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the workflow config contract version.
    pub const fn workflow_version(&self) -> u64 {
        self.workflow_version
    }

    /// Returns the canonical portfolio config.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns the canonical valuation source registry.
    pub const fn valuation_source_registry(&self) -> &ValuationSourceRegistry {
        &self.valuation_source_registry
    }
}

impl From<PortfolioSnapshotCanonicalConfig> for PortfolioWorkflowConfig {
    fn from(canonical: PortfolioSnapshotCanonicalConfig) -> Self {
        Self::new(canonical.portfolio, canonical.valuation_source_registry)
            .expect("canonical portfolio snapshot config must validate")
    }
}

/// Config for source preparation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.prepare_sources",
    validate = "validate_prepare_sources_config"
)]
pub struct PrepareSourcesConfig {
    /// Networks whose source pools are prepared.
    networks: Vec<NetworkConfig>,
}

impl PrepareSourcesConfig {
    /// Creates validated source-preparation config.
    pub fn new(networks: Vec<NetworkConfig>) -> Result<Self, ConfigError> {
        let config = Self { networks };
        validate_prepare_sources_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the networks whose source pools are prepared.
    pub fn networks(&self) -> &[NetworkConfig] {
        &self.networks
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

/// Config for execution-view pinning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.pin_views",
    validate = "validate_pin_views_config"
)]
pub struct PinViewsConfig {
    /// Pinning config contract version.
    pin_version: u64,
    /// Networks to pin.
    networks: Vec<NetworkConfig>,
}

impl PinViewsConfig {
    /// Creates validated view-pinning config.
    pub fn new(networks: Vec<NetworkConfig>) -> Result<Self, ConfigError> {
        let config = Self {
            pin_version: 1,
            networks,
        };
        validate_pin_views_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the pinning config contract version.
    pub const fn pin_version(&self) -> u64 {
        self.pin_version
    }

    /// Returns the networks to pin.
    pub fn networks(&self) -> &[NetworkConfig] {
        &self.networks
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
    /// Typed valuation source registry.
    valuation_source_registry: ValuationSourceRegistry,
}

impl ResolveValuationsConfig {
    /// Creates validated valuation-resolution config.
    pub fn new(
        symbol_configs: Vec<SymbolConfig>,
        valuation_source_registry: ValuationSourceRegistry,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            symbol_configs,
            valuation_source_registry,
        };
        validate_resolve_valuations_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the symbols whose valuation routes should be resolved.
    pub fn symbol_configs(&self) -> &[SymbolConfig] {
        &self.symbol_configs
    }

    /// Returns the typed valuation source registry.
    pub const fn valuation_source_registry(&self) -> &ValuationSourceRegistry {
        &self.valuation_source_registry
    }
}

/// Config for one typed observation fanout state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.observe_batch",
    validate = "validate_observe_batch_config"
)]
pub struct ObserveBatchConfig {
    /// Wallet being observed.
    wallet: WalletConfig,
    /// Symbol being observed for the wallet.
    symbol: SymbolConfig,
    /// Network shared by the wallet and symbol.
    network: NetworkConfig,
}

impl ObserveBatchConfig {
    /// Creates validated observation-batch config.
    pub fn new(
        wallet: WalletConfig,
        symbol: SymbolConfig,
        network: NetworkConfig,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            wallet,
            symbol,
            network,
        };
        validate_observe_batch_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the wallet being observed.
    pub const fn wallet(&self) -> &WalletConfig {
        &self.wallet
    }

    /// Returns the symbol being observed.
    pub const fn symbol(&self) -> &SymbolConfig {
        &self.symbol
    }

    /// Returns the network shared by the wallet and symbol.
    pub const fn network(&self) -> &NetworkConfig {
        &self.network
    }
}

/// Config for observation fan-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.merge_observations",
    validate = "validate_merge_observations_config"
)]
pub struct MergeObservationsConfig {
    /// Merge config contract version.
    merge_version: u64,
}

impl MergeObservationsConfig {
    /// Creates validated observation-merge config.
    pub fn new() -> Self {
        Self { merge_version: 1 }
    }

    /// Returns the merge config contract version.
    pub const fn merge_version(&self) -> u64 {
        self.merge_version
    }
}

impl Default for MergeObservationsConfig {
    fn default() -> Self {
        Self::new()
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
    snapshot_version: u64,
    /// Portfolio config carried into the public snapshot.
    portfolio: PortfolioConfig,
}

impl AssembleSnapshotConfig {
    /// Creates validated snapshot-assembly config.
    pub fn new(snapshot_version: u64, portfolio: PortfolioConfig) -> Result<Self, ConfigError> {
        let config = Self {
            snapshot_version,
            portfolio,
        };
        validate_assemble_snapshot_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the snapshot schema version to emit.
    pub const fn snapshot_version(&self) -> u64 {
        self.snapshot_version
    }

    /// Returns the portfolio config carried into the public snapshot.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
}

/// Config for report projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.project_report",
    validate = "validate_project_report_config"
)]
pub struct ProjectReportConfig {
    /// Report schema version to emit.
    report_version: u64,
}

impl ProjectReportConfig {
    /// Creates validated report-projection config.
    pub fn new(report_version: u64) -> Result<Self, ConfigError> {
        let config = Self { report_version };
        validate_project_report_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the report schema version to emit.
    pub const fn report_version(&self) -> u64 {
        self.report_version
    }
}

fn validate_portfolio_workflow_config(config: &PortfolioWorkflowConfig) -> Result<(), String> {
    if config.workflow_version != 1 {
        return Err("unsupported portfolio workflow config version".to_owned());
    }
    ValidatedPortfolioBundle::new(
        config.portfolio.clone(),
        config.valuation_source_registry.clone(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn validate_prepare_sources_config(config: &PrepareSourcesConfig) -> Result<(), String> {
    validate_network_collection(&config.networks)
}

fn validate_resolve_subjects_config(config: &ResolveSubjectsConfig) -> Result<(), String> {
    for wallet in &config.wallets {
        validate_wallet_config(wallet).map_err(|error| error.to_string())?;
    }
    validate_wallet_keys(&config.wallets)
}

fn validate_pin_views_config(config: &PinViewsConfig) -> Result<(), String> {
    if config.pin_version != 1 {
        return Err("unsupported portfolio pin config version".to_owned());
    }
    validate_network_collection(&config.networks)
}

fn validate_resolve_valuations_config(config: &ResolveValuationsConfig) -> Result<(), String> {
    for symbol in &config.symbol_configs {
        validate_symbol_config(symbol).map_err(|error| error.to_string())?;
    }
    validate_symbol_keys(&config.symbol_configs)?;
    validate_valuation_source_registry(&config.valuation_source_registry)
        .map_err(|error| error.to_string())
}

fn validate_merge_observations_config(config: &MergeObservationsConfig) -> Result<(), String> {
    if config.merge_version == 1 {
        Ok(())
    } else {
        Err("unsupported portfolio merge config version".to_owned())
    }
}

fn validate_assemble_snapshot_config(config: &AssembleSnapshotConfig) -> Result<(), String> {
    if config.snapshot_version == 0 {
        return Err("snapshot schema version must be non-zero".to_owned());
    }
    ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_project_report_config(config: &ProjectReportConfig) -> Result<(), String> {
    if config.report_version == 0 {
        Err("report schema version must be non-zero".to_owned())
    } else {
        Ok(())
    }
}

fn validate_network_collection(networks: &[NetworkConfig]) -> Result<(), String> {
    for network in networks {
        validate_network_config(network).map_err(|error| error.to_string())?;
    }
    validate_network_keys(networks)
}

/// Prepared external source summary for one network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "prepared-source",
    schema = "mfm.portfolio.prepared_source"
)]
pub struct PreparedSource {
    /// Stable network identifier.
    pub network_id: String,
    /// Stable control scope used for reads.
    pub control_scope: String,
    /// Network family.
    pub family: NetworkFamilyConfig,
}

/// Prepared source collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "prepared-sources",
    schema = "mfm.portfolio.prepared_sources"
)]
pub struct PreparedSources {
    /// Prepared networks in canonical order.
    pub sources: Vec<PreparedSource>,
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

/// Pinned execution view for one network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "pinned-view",
    schema = "mfm.portfolio.pinned_view"
)]
pub struct PinnedView {
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned execution anchor.
    pub anchor: ExecutionAnchor,
}

/// Pinned execution views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "pinned-views",
    schema = "mfm.portfolio.pinned_views"
)]
pub struct PinnedViews {
    /// Views in canonical network order.
    pub views: Vec<PinnedView>,
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
    /// Canonical valuation reader kind.
    pub valuation_reader_kind: String,
    /// Concrete source refs used for the valuation.
    pub source_refs: Vec<ObservationValueSourceRef>,
}

/// Resolved valuation collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-valuations",
    schema = "mfm.portfolio.resolved_valuations"
)]
pub struct ResolvedValuations {
    /// Valuations in canonical symbol/quote order.
    pub valuations: Vec<ResolvedValuation>,
    /// Non-fatal valuation resolution errors.
    pub errors: Vec<PortfolioSnapshotError>,
}

/// Observations emitted by one fanout state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-batch",
    schema = "mfm.portfolio.observation_batch"
)]
pub struct ObservationBatch {
    /// Stable batch key.
    pub batch_id: String,
    /// Observations in canonical order.
    pub observations: Vec<Observation>,
    /// Non-fatal observation errors.
    pub errors: Vec<PortfolioSnapshotError>,
}

/// Merged observation fan-in output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "merged-observations",
    schema = "mfm.portfolio.merged_observations"
)]
pub struct MergedObservations {
    /// Observations in stable domain-key fan-in order, then canonical observation order.
    pub observations: Vec<Observation>,
    /// Non-fatal observation errors.
    pub errors: Vec<PortfolioSnapshotError>,
}

/// Input consumed by typed observation fanout states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.observe_batch")]
pub struct ObserveBatchInput {
    /// Resolved wallet subjects.
    pub subjects: ResolvedSubjects,
    /// Pinned network views.
    pub views: PinnedViews,
    /// Resolved valuation routes.
    pub valuations: ResolvedValuations,
}

/// Input consumed by typed snapshot assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.assemble_snapshot")]
pub struct AssembleSnapshotInput {
    /// Resolved wallet subjects.
    pub subjects: ResolvedSubjects,
    /// Pinned network views.
    pub views: PinnedViews,
    /// Merged observation fan-in output.
    pub observations: MergedObservations,
}

/// Input consumed by report projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.project_report")]
pub struct ProjectReportInput {
    /// Assembled portfolio snapshot.
    pub snapshot: PortfolioSnapshot,
}

/// Fact request recorded by source-preparation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "source-preparation-request",
    schema = "mfm.portfolio.fact.source_preparation_request"
)]
pub struct SourcePreparationRequest {
    /// Networks requested by the state.
    pub network_ids: Vec<String>,
}

/// Fact response recorded by source-preparation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "source-preparation-response",
    schema = "mfm.portfolio.fact.source_preparation_response"
)]
pub struct SourcePreparationResponse {
    /// Prepared source payload.
    pub prepared: PreparedSources,
}

/// Fact request recorded by view-pinning states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "view-pin-request",
    schema = "mfm.portfolio.fact.view_pin_request"
)]
pub struct ViewPinRequest {
    /// Networks requested by the state.
    pub network_ids: Vec<String>,
}

/// Fact response recorded by view-pinning states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "view-pin-response",
    schema = "mfm.portfolio.fact.view_pin_response"
)]
pub struct ViewPinResponse {
    /// Pinned view payload.
    pub views: PinnedViews,
}

/// Fact request recorded by observation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-request",
    schema = "mfm.portfolio.fact.observation_request"
)]
pub struct ObservationRequest {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Stable symbol identifier.
    pub symbol_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Canonical balance reader kind.
    pub balance_reader_kind: String,
    /// Pinned EVM block number, when the observation targets an EVM network.
    pub block_number: Option<u64>,
}

/// Fact response recorded by observation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-response",
    schema = "mfm.portfolio.fact.observation_response"
)]
pub struct ObservationResponse {
    /// Observation batch payload.
    pub batch: ObservationBatch,
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

/// State that prepares external source metadata for configured networks.
pub struct PrepareSourcesState {
    config: PrepareSourcesConfig,
}

impl StateSpec for PrepareSourcesState {
    type Config = PrepareSourcesConfig;
    type Input = ();
    type Output = PreparedSources;
    type Effect = ReadExternal;
    type Caps = (PortfolioReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("prepare_sources")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("prepare_sources")
    }

    fn name() -> &'static str {
        "mfm.portfolio.prepare_sources"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl ReadState for PrepareSourcesState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(&'a self, _input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        future::ready(Ok(prepare_sources_from_config(&self.config)))
    }
}

/// State that resolves configured wallet subjects.
pub struct ResolveSubjectsState {
    config: ResolveSubjectsConfig,
}

impl StateSpec for ResolveSubjectsState {
    type Config = ResolveSubjectsConfig;
    type Input = PreparedSources;
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for ResolveSubjectsState {
    fn run(&self, _input: Self::Input) -> StateResult<Self::Output> {
        Ok(resolve_subjects_from_config(&self.config))
    }
}

/// State that pins concrete execution views for configured networks.
pub struct PinViewsState {
    config: PinViewsConfig,
}

impl StateSpec for PinViewsState {
    type Config = PinViewsConfig;
    type Input = PreparedSources;
    type Output = PinnedViews;
    type Effect = ReadExternal;
    type Caps = (PortfolioReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("pin_views")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("pin_views")
    }

    fn name() -> &'static str {
        "mfm.portfolio.pin_views"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl ReadState for PinViewsState {
    type RunFuture<'a> = Pin<Box<dyn Future<Output = StateResult<Self::Output>> + Send + 'a>>;

    fn run<'a>(&'a self, _input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        Box::pin(async move {
            pin_views_with_backend(&self.config, &UnavailablePortfolioReadBackend)
                .await
                .map_err(state_error_from_portfolio_read)
        })
    }
}

/// State that resolves configured valuation routes.
pub struct ResolveValuationsState {
    config: ResolveValuationsConfig,
}

impl StateSpec for ResolveValuationsState {
    type Config = ResolveValuationsConfig;
    type Input = PinnedViews;
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for ResolveValuationsState {
    fn run(&self, views: Self::Input) -> StateResult<Self::Output> {
        Ok(resolve_valuations_from_config(&self.config, &views))
    }
}

/// State that observes one wallet/symbol batch.
pub struct ObserveBatchState {
    config: ObserveBatchConfig,
}

impl StateSpec for ObserveBatchState {
    type Config = ObserveBatchConfig;
    type Input = ObserveBatchInput;
    type Output = ObservationBatch;
    type Effect = ReadExternal;
    type Caps = (PortfolioReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("observe_batch")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("observe_batch")
    }

    fn name() -> &'static str {
        "mfm.portfolio.observe_batch"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl ReadState for ObserveBatchState {
    type RunFuture<'a> = Pin<Box<dyn Future<Output = StateResult<Self::Output>> + Send + 'a>>;

    fn run<'a>(&'a self, input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        Box::pin(async move {
            Ok(
                observe_batch_with_backend(&self.config, &input, &UnavailablePortfolioReadBackend)
                    .await,
            )
        })
    }
}

/// State that merges non-empty observation batches.
pub struct MergeObservationsState {
    config: MergeObservationsConfig,
}

impl StateSpec for MergeObservationsState {
    type Config = MergeObservationsConfig;
    type Input = NonEmpty<ObservationBatch>;
    type Output = MergedObservations;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("merge_observations")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("merge_observations")
    }

    fn name() -> &'static str {
        "mfm.portfolio.merge_observations"
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for MergeObservationsState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        let _ = self.config.merge_version();
        Ok(merge_observation_batches(input))
    }
}

/// State that assembles the canonical snapshot.
pub struct AssembleSnapshotState {
    config: AssembleSnapshotConfig,
}

impl StateSpec for AssembleSnapshotState {
    type Config = AssembleSnapshotConfig;
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for AssembleSnapshotState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(assemble_snapshot(&self.config, input, 0))
    }
}

/// State that projects the canonical public report from a snapshot.
pub struct ProjectReportState {
    config: ProjectReportConfig,
}

impl StateSpec for ProjectReportState {
    type Config = ProjectReportConfig;
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for ProjectReportState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        project_report_from_snapshot(input.snapshot, self.config.report_version())
    }
}

/// Builds prepared source output from typed network config.
pub fn prepare_sources_from_config(config: &PrepareSourcesConfig) -> PreparedSources {
    let mut sources = config
        .networks
        .iter()
        .map(|network| PreparedSource {
            network_id: network.network_id.clone(),
            control_scope: network.control_scope.clone(),
            family: network.family,
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.network_id.cmp(&right.network_id));
    PreparedSources { sources }
}

/// Resolves configured wallets into typed subjects.
pub fn resolve_subjects_from_config(config: &ResolveSubjectsConfig) -> ResolvedSubjects {
    let mut subjects = config
        .wallets
        .iter()
        .map(|wallet| ResolvedSubject {
            wallet_id: wallet.wallet_id.clone(),
            address: wallet.address.clone(),
            subject_kind: wallet.subject_kind,
            network_id: wallet.network_id.clone(),
            implementation_kind: wallet_implementation_kind(&wallet.implementation).to_owned(),
        })
        .collect::<Vec<_>>();
    subjects.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
    ResolvedSubjects { subjects }
}

/// Builds a pinned view for `network` using the supplied EVM block number.
pub fn pinned_view_for_network(network: &NetworkConfig, evm_block_number: u64) -> PinnedView {
    let anchor = match network.family {
        NetworkFamilyConfig::Evm => ExecutionAnchor::Evm {
            chain_id: network.chain_id.unwrap_or_default(),
            block_number: evm_block_number,
        },
        NetworkFamilyConfig::Bitcoin => ExecutionAnchor::Bitcoin {
            height: 0,
            block_hash: String::new(),
        },
    };
    PinnedView {
        network_id: network.network_id.clone(),
        anchor,
    }
}

/// Pins execution views through the supplied portfolio read backend.
pub async fn pin_views_with_backend<B>(
    config: &PinViewsConfig,
    backend: &B,
) -> Result<PinnedViews, PortfolioReadError>
where
    B: PortfolioReadBackend + ?Sized,
{
    let mut views = Vec::new();
    for network in &config.networks {
        let block_number = match network.family {
            NetworkFamilyConfig::Evm => backend.evm_block_number(&network.network_id).await?,
            NetworkFamilyConfig::Bitcoin => 0,
        };
        views.push(pinned_view_for_network(network, block_number));
    }
    views.sort_by(|left, right| left.network_id.cmp(&right.network_id));
    Ok(PinnedViews { views })
}

/// Resolves configured valuation routes.
pub fn resolve_valuations_from_config(
    config: &ResolveValuationsConfig,
    views: &PinnedViews,
) -> ResolvedValuations {
    let mut valuations = Vec::new();
    let mut errors = Vec::new();
    for symbol in &config.symbol_configs {
        for quote in &symbol.valuation.quotes {
            match resolved_valuation_for_quote(symbol, quote, views) {
                Ok(valuation) => valuations.push(valuation),
                Err(error) => errors.push(*error),
            }
        }
    }
    valuations.sort_by(|left, right| {
        (left.symbol_id.as_str(), left.quote).cmp(&(right.symbol_id.as_str(), right.quote))
    });
    errors.sort_by(|left, right| error_sort_key(left).cmp(&error_sort_key(right)));
    ResolvedValuations { valuations, errors }
}

fn resolved_valuation_for_quote(
    symbol: &SymbolConfig,
    quote: &QuoteValuationConfig,
    views: &PinnedViews,
) -> Result<ResolvedValuation, Box<PortfolioSnapshotError>> {
    match &quote.reader {
        ValuationReaderConfig::FixedUnitPrice { unit_price_dec } => Ok(ResolvedValuation {
            symbol_id: symbol.symbol_id.clone(),
            quote: quote.quote,
            priced_symbol_id: quote.priced_symbol_id.clone(),
            unit_price_dec: unit_price_dec.clone(),
            valuation_reader_kind: "fixed_unit_price".to_owned(),
            source_refs: Vec::new(),
        }),
        ValuationReaderConfig::DirectPrice { source } => {
            let Some(view) = views
                .views
                .iter()
                .find(|view| view.network_id == source.network_id)
            else {
                return Err(Box::new(snapshot_error(
                    "missing_valuation_source_view",
                    format!(
                        "missing pinned view `{}` for direct price source `{}`",
                        source.network_id, source.source_id
                    ),
                    None,
                    Some(&symbol.symbol_id),
                    Some(&source.network_id),
                    Some("direct_price"),
                )));
            };
            Err(Box::new(snapshot_error(
                "unsupported_valuation_reader",
                "direct price valuation is not enabled in the typed portfolio runner",
                None,
                Some(&symbol.symbol_id),
                Some(&source.network_id),
                Some(observation_anchor_reader_kind(&view.anchor)),
            )))
        }
        ValuationReaderConfig::DerivedUnitPrice { .. } => Err(Box::new(snapshot_error(
            "unsupported_valuation_reader",
            "derived unit price valuation is not enabled in the typed portfolio runner",
            None,
            Some(&symbol.symbol_id),
            Some(&symbol.network_id),
            Some("derived_unit_price"),
        ))),
    }
}

/// Builds an observation batch from an externally read raw balance.
pub fn observation_batch_from_raw_balance(
    config: &ObserveBatchConfig,
    input: &ObserveBatchInput,
    raw: U256,
    decimals: u8,
    block_number: u64,
) -> ObservationBatch {
    let mut errors = input.valuations.errors.clone();
    let batch_id = observation_batch_id(&config.wallet.wallet_id, &config.symbol.symbol_id);
    let Some(subject) = input
        .subjects
        .subjects
        .iter()
        .find(|subject| subject.wallet_id == config.wallet.wallet_id)
    else {
        errors.push(snapshot_error(
            "missing_resolved_subject",
            "missing resolved subject for observation batch",
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(&config.network.network_id),
            None,
        ));
        return ObservationBatch {
            batch_id,
            observations: Vec::new(),
            errors,
        };
    };
    let Some(view) = input
        .views
        .views
        .iter()
        .find(|view| view.network_id == config.network.network_id)
    else {
        errors.push(snapshot_error(
            "missing_pinned_view",
            "missing pinned view for observation batch",
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(&config.network.network_id),
            None,
        ));
        return ObservationBatch {
            batch_id,
            observations: Vec::new(),
            errors,
        };
    };
    let amount_dec = format_u256_units(&raw, decimals);
    let values = observation_values(config, input, &amount_dec, &mut errors);
    if values.is_empty() {
        errors.push(snapshot_error(
            "missing_observation_values",
            "observation had no resolved valuation values",
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(&config.network.network_id),
            Some(balance_reader_kind(&config.symbol.balance_reader)),
        ));
        return ObservationBatch {
            batch_id,
            observations: Vec::new(),
            errors,
        };
    }

    let anchor = match &view.anchor {
        ExecutionAnchor::Evm { chain_id, .. } => ObservationAnchor::Evm {
            chain_id: *chain_id,
            block_number,
        },
        ExecutionAnchor::Bitcoin { height, block_hash } => ObservationAnchor::Bitcoin {
            height: *height,
            block_hash: block_hash.clone(),
        },
    };
    let mut observation = Observation {
        wallet_id: subject.wallet_id.clone(),
        symbol_id: config.symbol.symbol_id.clone(),
        display_symbol: config.symbol.display_symbol.clone(),
        kind: config.symbol.kind,
        role: config.symbol.role,
        network_id: config.network.network_id.clone(),
        protocol: config.symbol.protocol.clone(),
        quantity: ObservationQuantity {
            raw_dec: raw.to_string(),
            decimals,
            amount_dec,
        },
        values,
        source: ObservationSource {
            balance_reader_kind: balance_reader_kind(&config.symbol.balance_reader).to_owned(),
            network_id: config.network.network_id.clone(),
            anchor,
        },
        metadata: config.symbol.metadata.clone(),
    };
    observation.normalize();
    ObservationBatch {
        batch_id,
        observations: vec![observation],
        errors,
    }
}

/// Builds an observation batch containing a redaction-safe external-read error.
pub fn observation_batch_error(
    config: &ObserveBatchConfig,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ObservationBatch {
    ObservationBatch {
        batch_id: observation_batch_id(&config.wallet.wallet_id, &config.symbol.symbol_id),
        observations: Vec::new(),
        errors: vec![snapshot_error(
            code,
            message,
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(&config.network.network_id),
            Some(balance_reader_kind(&config.symbol.balance_reader)),
        )],
    }
}

/// Observes one wallet/symbol batch through the supplied portfolio read backend.
pub async fn observe_batch_with_backend<B>(
    config: &ObserveBatchConfig,
    input: &ObserveBatchInput,
    backend: &B,
) -> ObservationBatch
where
    B: PortfolioReadBackend + ?Sized,
{
    let block_number = evm_block_number_for(&input.views, &config.network.network_id);
    match backend.observe_raw_balance(config, block_number).await {
        Ok((raw, decimals)) => {
            observation_batch_from_raw_balance(config, input, raw, decimals, block_number)
        }
        Err(error) => observation_batch_error(config, error.code, error.message),
    }
}

/// Returns the pinned EVM block number for `network_id`, or zero when no EVM view exists.
pub fn evm_block_number_for(views: &PinnedViews, network_id: &str) -> u64 {
    views
        .views
        .iter()
        .find_map(|view| match &view.anchor {
            ExecutionAnchor::Evm { block_number, .. } if view.network_id == network_id => {
                Some(*block_number)
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn state_error_from_portfolio_read(error: PortfolioReadError) -> StateError {
    StateError::Message(error.to_string())
}

/// Merges non-empty observation batches.
pub fn merge_observation_batches(input: NonEmpty<ObservationBatch>) -> MergedObservations {
    let mut observations = Vec::new();
    let mut errors = Vec::new();
    for batch in input.values() {
        observations.extend(batch.observations.clone());
        errors.extend(batch.errors.clone());
    }
    for observation in &mut observations {
        observation.normalize();
    }
    observations.sort_by(|left, right| {
        (left.wallet_id.as_str(), left.symbol_id.as_str())
            .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
    });
    errors.sort_by(|left, right| error_sort_key(left).cmp(&error_sort_key(right)));
    MergedObservations {
        observations,
        errors,
    }
}

/// Assembles the canonical portfolio snapshot.
pub fn assemble_snapshot(
    config: &AssembleSnapshotConfig,
    input: AssembleSnapshotInput,
    generated_at_ms: u64,
) -> PortfolioSnapshot {
    let mut observations_by_wallet: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    for observation in input.observations.observations {
        observations_by_wallet
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    let mut subjects_by_wallet = BTreeMap::new();
    for subject in input.subjects.subjects {
        subjects_by_wallet.insert(subject.wallet_id.clone(), subject);
    }

    let mut wallets = config
        .portfolio
        .wallets
        .iter()
        .map(|wallet| {
            let subject = subjects_by_wallet
                .get(&wallet.wallet_id)
                .cloned()
                .unwrap_or_else(|| ResolvedSubject {
                    wallet_id: wallet.wallet_id.clone(),
                    address: wallet.address.clone(),
                    subject_kind: wallet.subject_kind,
                    network_id: wallet.network_id.clone(),
                    implementation_kind: wallet_implementation_kind(&wallet.implementation)
                        .to_owned(),
                });
            let mut wallet = WalletSnapshot {
                wallet_id: wallet.wallet_id.clone(),
                address: subject.address,
                subject_kind: subject.subject_kind,
                network_id: subject.network_id,
                observations: observations_by_wallet
                    .remove(&wallet.wallet_id)
                    .unwrap_or_default(),
            };
            wallet.normalize();
            wallet
        })
        .collect::<Vec<_>>();
    wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

    let mut network_pins = input
        .views
        .views
        .into_iter()
        .map(|view| NetworkPin {
            network_id: view.network_id,
            anchor: view.anchor,
        })
        .collect::<Vec<_>>();
    network_pins.sort_by(|left, right| left.network_id.cmp(&right.network_id));

    let mut symbol_configs = config.portfolio.symbol_configs.clone();
    for symbol in &mut symbol_configs {
        symbol.normalize();
    }
    symbol_configs.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));

    let mut snapshot = PortfolioSnapshot {
        schema_version: config.snapshot_version,
        portfolio_id: config.portfolio.portfolio_id.clone(),
        generated_at_ms,
        network_pins,
        wallets,
        symbol_configs,
        errors: input.observations.errors,
    };
    snapshot.normalize();
    snapshot
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
                wallet_id: wallet.wallet_id.clone(),
                network_id: wallet.network_id.clone(),
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
        error_count: snapshot.errors.len() as u64,
    };
    report.normalize();
    Ok(report)
}

/// Returns the canonical observation batch id for a wallet/symbol pair.
pub fn observation_batch_id(wallet_id: &str, symbol_id: &str) -> String {
    format!("wallet/{wallet_id}/symbol/{symbol_id}")
}

/// Returns the canonical balance reader kind string.
pub fn balance_reader_kind(reader: &BalanceReaderConfig) -> &'static str {
    match reader {
        BalanceReaderConfig::NativeBalance {} => "native_balance",
        BalanceReaderConfig::Erc20Balance { .. } => "erc20_balance",
        BalanceReaderConfig::ProtocolPosition {
            protocol, reader, ..
        } if protocol == AAVE_V3_PROTOCOL_ID => match reader.as_str() {
            "reserve_position" => "aave_v3/reserve_position",
            "debt_position" => "aave_v3/debt_position",
            _ => "protocol_position",
        },
        BalanceReaderConfig::ProtocolPosition { .. } => "protocol_position",
    }
}

fn observation_values(
    config: &ObserveBatchConfig,
    input: &ObserveBatchInput,
    amount_dec: &str,
    errors: &mut Vec<PortfolioSnapshotError>,
) -> Vec<ObservationValue> {
    let mut values = Vec::new();
    for quote in &config.symbol.valuation.quotes {
        let Some(resolved) = input.valuations.valuations.iter().find(|valuation| {
            valuation.symbol_id == config.symbol.symbol_id && valuation.quote == quote.quote
        }) else {
            errors.push(snapshot_error(
                "missing_resolved_valuation",
                format!("missing valuation for quote `{}`", quote.quote),
                Some(&config.wallet.wallet_id),
                Some(&config.symbol.symbol_id),
                Some(&config.network.network_id),
                Some("valuation"),
            ));
            continue;
        };
        match multiply_decimal_strings(amount_dec, &resolved.unit_price_dec) {
            Ok(value_dec) => values.push(ObservationValue {
                quote: resolved.quote,
                priced_symbol_id: resolved.priced_symbol_id.clone(),
                value_dec,
                unit_price_dec: resolved.unit_price_dec.clone(),
                valuation_reader_kind: resolved.valuation_reader_kind.clone(),
                source_refs: resolved.source_refs.clone(),
            }),
            Err(error) => errors.push(snapshot_error(
                "invalid_decimal_string",
                error.to_string(),
                Some(&config.wallet.wallet_id),
                Some(&config.symbol.symbol_id),
                Some(&config.network.network_id),
                Some("valuation"),
            )),
        }
    }
    values.sort_by_key(|value| value.quote);
    values
}

fn validate_network_keys(networks: &[NetworkConfig]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for network in networks {
        if !seen.insert(network.network_id.as_str()) {
            return Err(format!(
                "duplicate portfolio network domain key `{}`",
                network.network_id
            ));
        }
    }
    Ok(())
}

fn validate_wallet_keys(wallets: &[WalletConfig]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for wallet in wallets {
        if !seen.insert(wallet.wallet_id.as_str()) {
            return Err(format!(
                "duplicate portfolio wallet domain key `{}`",
                wallet.wallet_id
            ));
        }
    }
    Ok(())
}

fn validate_symbol_keys(symbols: &[SymbolConfig]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        if !seen.insert(symbol.symbol_id.as_str()) {
            return Err(format!(
                "duplicate portfolio symbol domain key `{}`",
                symbol.symbol_id
            ));
        }
    }
    Ok(())
}

fn validate_observe_batch_config(config: &ObserveBatchConfig) -> Result<(), String> {
    validate_wallet_config(&config.wallet).map_err(|error| error.to_string())?;
    validate_symbol_config(&config.symbol).map_err(|error| error.to_string())?;
    validate_network_config(&config.network).map_err(|error| error.to_string())?;
    if config.wallet.network_id != config.network.network_id {
        return Err(format!(
            "wallet `{}` network `{}` did not match observation network `{}`",
            config.wallet.wallet_id, config.wallet.network_id, config.network.network_id
        ));
    }
    if config.symbol.network_id != config.network.network_id {
        return Err(format!(
            "symbol `{}` network `{}` did not match observation network `{}`",
            config.symbol.symbol_id, config.symbol.network_id, config.network.network_id
        ));
    }
    if !config
        .wallet
        .symbol_ids
        .iter()
        .any(|symbol_id| symbol_id == &config.symbol.symbol_id)
    {
        return Err(format!(
            "wallet `{}` did not include observation symbol `{}`",
            config.wallet.wallet_id, config.symbol.symbol_id
        ));
    }
    if let BalanceReaderConfig::ProtocolPosition {
        protocol,
        reader,
        config: protocol_config,
    } = &config.symbol.balance_reader
    {
        if config.symbol.protocol.as_deref() != Some(protocol.as_str()) {
            return Err(format!(
                "symbol `{}` protocol did not match typed protocol reader `{}`",
                config.symbol.symbol_id, protocol
            ));
        }
        if protocol == AAVE_V3_PROTOCOL_ID && reader != protocol_config.reader_name() {
            return Err(format!(
                "symbol `{}` Aave reader `{}` did not match typed config reader `{}`",
                config.symbol.symbol_id,
                reader,
                protocol_config.reader_name()
            ));
        }
    }
    Ok(())
}

fn wallet_implementation_kind(implementation: &WalletImplementationConfig) -> &'static str {
    match implementation {
        WalletImplementationConfig::AddressOnly {} => "address_only",
        WalletImplementationConfig::KeystoreEntry { .. } => "keystore_entry",
        WalletImplementationConfig::NodeManagedAccount { .. } => "node_managed_account",
        WalletImplementationConfig::ExternalSigner { .. } => "external_signer",
    }
}

fn observation_anchor_reader_kind(anchor: &ExecutionAnchor) -> &'static str {
    match anchor {
        ExecutionAnchor::Evm { .. } => "evm_oracle",
        ExecutionAnchor::Bitcoin { .. } => "bitcoin",
    }
}

fn snapshot_error(
    code: impl Into<String>,
    message: impl Into<String>,
    wallet_id: Option<&str>,
    symbol_id: Option<&str>,
    network_id: Option<&str>,
    reader_kind: Option<&str>,
) -> PortfolioSnapshotError {
    PortfolioSnapshotError {
        code: code.into(),
        message: message.into(),
        wallet_id: wallet_id.map(ToOwned::to_owned),
        symbol_id: symbol_id.map(ToOwned::to_owned),
        network_id: network_id.map(ToOwned::to_owned),
        reader_kind: reader_kind.map(ToOwned::to_owned),
    }
}

fn error_sort_key(error: &PortfolioSnapshotError) -> (&str, &str, &str, &str) {
    (
        error.network_id.as_deref().unwrap_or(""),
        error.wallet_id.as_deref().unwrap_or(""),
        error.symbol_id.as_deref().unwrap_or(""),
        error.code.as_str(),
    )
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum DecimalArithmeticError {
    InvalidDecimalString { value: String },
}

impl std::fmt::Display for DecimalArithmeticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDecimalString { value } => write!(f, "invalid decimal string `{value}`"),
        }
    }
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
mod tests {
    use super::*;
    use mfm_portfolio_model::symbol::{SymbolKind, SymbolValuationConfig};
    use mfm_values::MfmConfig as _;

    #[test]
    fn observe_batch_mfm_config_validation_rejects_network_mismatch() {
        let wallet = WalletConfig {
            wallet_id: "wallet_main".to_owned(),
            address: "0x000000000000000000000000000000000000dead".to_owned(),
            subject_kind: WalletSubjectKind::EvmAddress,
            network_id: "ethereum-mainnet".to_owned(),
            implementation: WalletImplementationConfig::AddressOnly {},
            symbol_ids: vec!["eth.native.ethereum-mainnet".to_owned()],
            metadata: BTreeMap::new(),
        };
        let symbol = SymbolConfig {
            symbol_id: "eth.native.ethereum-mainnet".to_owned(),
            display_symbol: Some("ETH".to_owned()),
            kind: SymbolKind::NativeBalance,
            role: SymbolRole::Native,
            network_id: "ethereum-mainnet".to_owned(),
            protocol: None,
            balance_reader: BalanceReaderConfig::NativeBalance {},
            valuation: SymbolValuationConfig {
                quotes: vec![QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet".to_owned(),
                    reader: ValuationReaderConfig::FixedUnitPrice {
                        unit_price_dec: "2.5".to_owned(),
                    },
                }],
            },
            decimals: Some(18),
            underlying_symbol_id: None,
            metadata: BTreeMap::new(),
        };
        let config = ObserveBatchConfig {
            wallet,
            symbol,
            network: NetworkConfig {
                network_id: "ethereum-goerli".to_owned(),
                family: NetworkFamilyConfig::Evm,
                chain_id: Some(5),
                control_scope: "shared".to_owned(),
                metadata: BTreeMap::new(),
            },
        };

        let error = config.validate().expect_err("network mismatch must fail");
        assert!(
            error
                .message()
                .contains("did not match observation network"),
            "{error}"
        );
    }

    #[test]
    fn fixed_price_observation_projects_report_totals() {
        let network = NetworkConfig {
            network_id: "ethereum-mainnet".to_owned(),
            family: NetworkFamilyConfig::Evm,
            chain_id: Some(1),
            control_scope: "shared".to_owned(),
            metadata: BTreeMap::new(),
        };
        let wallet = WalletConfig {
            wallet_id: "wallet_main".to_owned(),
            address: "0x000000000000000000000000000000000000dead".to_owned(),
            subject_kind: WalletSubjectKind::EvmAddress,
            network_id: network.network_id.clone(),
            implementation: WalletImplementationConfig::AddressOnly {},
            symbol_ids: vec!["eth.native.ethereum-mainnet".to_owned()],
            metadata: BTreeMap::new(),
        };
        let symbol = SymbolConfig {
            symbol_id: "eth.native.ethereum-mainnet".to_owned(),
            display_symbol: Some("ETH".to_owned()),
            kind: SymbolKind::NativeBalance,
            role: SymbolRole::Native,
            network_id: network.network_id.clone(),
            protocol: None,
            balance_reader: BalanceReaderConfig::NativeBalance {},
            valuation: SymbolValuationConfig {
                quotes: vec![QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet".to_owned(),
                    reader: ValuationReaderConfig::FixedUnitPrice {
                        unit_price_dec: "2.5".to_owned(),
                    },
                }],
            },
            decimals: Some(18),
            underlying_symbol_id: None,
            metadata: BTreeMap::new(),
        };
        let input = ObserveBatchInput {
            subjects: ResolvedSubjects {
                subjects: vec![ResolvedSubject {
                    wallet_id: wallet.wallet_id.clone(),
                    address: wallet.address.clone(),
                    subject_kind: wallet.subject_kind,
                    network_id: wallet.network_id.clone(),
                    implementation_kind: "address_only".to_owned(),
                }],
            },
            views: PinnedViews {
                views: vec![PinnedView {
                    network_id: network.network_id.clone(),
                    anchor: ExecutionAnchor::Evm {
                        chain_id: 1,
                        block_number: 10,
                    },
                }],
            },
            valuations: ResolvedValuations {
                valuations: vec![ResolvedValuation {
                    symbol_id: symbol.symbol_id.clone(),
                    quote: QuoteCode::Usd,
                    priced_symbol_id: symbol.symbol_id.clone(),
                    unit_price_dec: "2.5".to_owned(),
                    valuation_reader_kind: "fixed_unit_price".to_owned(),
                    source_refs: Vec::new(),
                }],
                errors: Vec::new(),
            },
        };
        let batch = observation_batch_from_raw_balance(
            &ObserveBatchConfig::new(wallet.clone(), symbol.clone(), network.clone())
                .expect("observe config"),
            &input,
            U256::from(1_000_000_000_000_000_000u128),
            18,
            10,
        );
        assert_eq!(
            batch.observations[0].values[0].value_dec,
            "2.500000000000000000"
        );

        let snapshot = assemble_snapshot(
            &AssembleSnapshotConfig::new(
                2,
                PortfolioConfig {
                    portfolio_id: "portfolio_main".to_owned(),
                    quote_codes: vec![QuoteCode::Usd],
                    networks: vec![network],
                    wallets: vec![wallet],
                    symbol_configs: vec![symbol],
                    metadata: BTreeMap::new(),
                },
            )
            .expect("assemble config"),
            AssembleSnapshotInput {
                subjects: input.subjects,
                views: input.views,
                observations: MergedObservations {
                    observations: batch.observations,
                    errors: Vec::new(),
                },
            },
            42,
        );
        let report = project_report_from_snapshot(snapshot, 2).expect("report");
        assert_eq!(
            report.totals_by_quote[0].assets_value_dec,
            "2.500000000000000000"
        );
        assert_eq!(
            report.totals_by_quote[0].net_value_dec,
            "2.500000000000000000"
        );
    }
}
