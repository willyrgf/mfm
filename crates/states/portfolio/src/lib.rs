#![warn(missing_docs)]
//! Typed portfolio-domain state contracts.
//!
//! This crate owns the reusable typed state, value, input, and capability contracts for portfolio
//! snapshots.
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
use std::future;
use std::num::NonZeroU64;

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
    ExecutionAnchor, NetworkConfig, NetworkPin, PortfolioConfig, PortfolioQuoteTotal,
    PortfolioReport, PortfolioSnapshot, PortfolioSnapshotError, ValidatedNetworkConfigs,
    ValidatedPortfolioBundle, ValidatedPortfolioConfig, ValidatedSymbolConfigs,
    ValidatedWalletConfigs, WalletReport, WalletSnapshot,
};
use mfm_portfolio_model::symbol::{
    validate_symbol_config, validate_valuation_source_registry, BalanceReaderConfig, Observation,
    ObservationAnchor, ObservationQuantity, ObservationSource, ObservationValue,
    ObservationValueSourceRef, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolRole,
    ValuationReaderConfig, ValuationSourceRegistry,
};
use mfm_portfolio_model::wallet::{WalletConfig, WalletImplementationConfig, WalletSubjectKind};
use mfm_program::{
    AdapterBindingSpec, NoContext, PureState, ReadState, StateError, StateResult, StateSpec,
};
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

/// Raw balance read result returned by adapter-executed portfolio reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBalanceObservation {
    /// Raw balance integer in the symbol's native base unit.
    pub raw: U256,
    /// Decimal precision for rendering the raw amount.
    pub decimals: u8,
    /// Concrete observation anchor, when the executed read returns its own source anchor.
    pub anchor: Option<ExecutionAnchor>,
}

impl RawBalanceObservation {
    /// Creates a raw balance observation.
    pub fn new(raw: U256, decimals: u8, anchor: Option<ExecutionAnchor>) -> Self {
        Self {
            raw,
            decimals,
            anchor,
        }
    }
}

/// Classified network read intent derived from portfolio state config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioNetworkReadIntent {
    /// Read an Ethereum-compatible network.
    Evm {
        /// Stable portfolio network id.
        network_id: String,
        /// Expected EVM chain id.
        chain_id: u64,
    },
    /// Read a Bitcoin-family network.
    Bitcoin {
        /// Stable portfolio network id.
        network_id: String,
        /// Semantic source identity expected for the read.
        source_identity: String,
    },
}

/// Classified raw balance read intent derived from portfolio state config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioBalanceReadIntent {
    /// Read an EVM native account balance.
    EvmNativeBalance {
        /// Stable portfolio network id.
        network_id: String,
        /// Expected EVM chain id.
        chain_id: u64,
        /// Canonical account address.
        account: String,
        /// Pinned EVM block number.
        block_number: u64,
        /// Decimal precision used for rendering the raw amount.
        decimals: u8,
        /// Pinned execution anchor that must be preserved in the observation.
        anchor: ExecutionAnchor,
    },
    /// Read an ERC-20 balance.
    Erc20Balance {
        /// Stable portfolio network id.
        network_id: String,
        /// Expected EVM chain id.
        chain_id: u64,
        /// Canonical account address.
        account: String,
        /// Canonical token contract address.
        token_address: String,
        /// Pinned EVM block number.
        block_number: u64,
        /// Optional configured token decimals. When absent, the adapter must read token decimals.
        decimals: Option<u8>,
        /// Pinned execution anchor that must be preserved in the observation.
        anchor: ExecutionAnchor,
    },
    /// Read a Bitcoin native address balance.
    BitcoinNativeBalance {
        /// Stable portfolio network id.
        network_id: String,
        /// Semantic source identity expected for the read.
        source_identity: String,
        /// Canonical Bitcoin address.
        address: String,
        /// Pinned execution anchor that the returned balance must match.
        anchor: ExecutionAnchor,
        /// Decimal precision used for rendering the raw amount.
        decimals: u8,
    },
}

/// Redaction-safe external read error reported by portfolio read execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PortfolioReadError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Redacted human-readable message.
    pub message: String,
    /// Optional closed redacted diagnostic details for retained runtime evidence.
    pub redacted_details: Option<serde_json::Value>,
    /// Whether this read error must fail the runtime attempt instead of becoming domain output.
    pub fatal_attempt_failure: bool,
}

impl PortfolioReadError {
    /// Builds a redaction-safe portfolio read error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            redacted_details: None,
            fatal_attempt_failure: false,
        }
    }

    /// Attaches closed redacted diagnostic details.
    pub fn with_redacted_details(mut self, details: serde_json::Value) -> Self {
        self.redacted_details = Some(details);
        self
    }

    /// Marks this read error as a runtime attempt failure.
    pub fn with_fatal_attempt_failure(mut self) -> Self {
        self.fatal_attempt_failure = true;
        self
    }
}

/// Root typed portfolio workflow config.
#[derive(Debug, Clone, Serialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.workflow",
    validate = "validate_portfolio_workflow_config"
)]
pub struct PortfolioWorkflowConfig {
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
            portfolio,
            valuation_source_registry,
        };
        validate_portfolio_workflow_config(&config).map_err(ConfigError::new)?;
        Ok(config)
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

impl<'de> Deserialize<'de> for PortfolioWorkflowConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawPortfolioWorkflowConfig {
            portfolio: PortfolioConfig,
            valuation_source_registry: ValuationSourceRegistry,
        }

        let raw = RawPortfolioWorkflowConfig::deserialize(deserializer)?;
        Ok(Self {
            portfolio: raw.portfolio,
            valuation_source_registry: raw.valuation_source_registry,
        })
    }
}

impl From<PortfolioSnapshotCanonicalConfig> for PortfolioWorkflowConfig {
    fn from(canonical: PortfolioSnapshotCanonicalConfig) -> Self {
        Self::new(canonical.portfolio, canonical.valuation_source_registry)
            .expect("canonical portfolio snapshot config must validate")
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
#[derive(Debug, Clone, Serialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.portfolio.config.pin_views",
    validate = "validate_pin_views_config"
)]
pub struct PinViewsConfig {
    /// Networks to pin.
    pinned_networks: Vec<NetworkConfig>,
}

impl PinViewsConfig {
    /// Creates validated view-pinning config.
    pub fn new(networks: Vec<NetworkConfig>) -> Result<Self, ConfigError> {
        let config = Self {
            pinned_networks: networks,
        };
        validate_pin_views_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the networks to pin.
    pub fn networks(&self) -> &[NetworkConfig] {
        &self.pinned_networks
    }
}

impl<'de> Deserialize<'de> for PinViewsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawPinViewsConfig {
            pinned_networks: Vec<NetworkConfig>,
        }

        let raw = RawPinViewsConfig::deserialize(deserializer)?;
        Ok(Self {
            pinned_networks: raw.pinned_networks,
        })
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
#[derive(Debug, Clone, Serialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.portfolio.config.merge_observations")]
pub struct MergeObservationsConfig {}

impl MergeObservationsConfig {
    /// Creates validated observation-merge config.
    pub fn new() -> Self {
        Self {}
    }
}

impl<'de> Deserialize<'de> for MergeObservationsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawMergeObservationsConfig {}

        let _raw = RawMergeObservationsConfig::deserialize(deserializer)?;
        Ok(Self::new())
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
        let config = Self { report_version };
        Ok(config)
    }

    /// Returns the report schema version to emit.
    pub const fn report_version(&self) -> u64 {
        self.report_version.get()
    }
}

fn validate_portfolio_workflow_config(config: &PortfolioWorkflowConfig) -> Result<(), String> {
    ValidatedPortfolioBundle::new(
        config.portfolio.clone(),
        config.valuation_source_registry.clone(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn validate_resolve_subjects_config(config: &ResolveSubjectsConfig) -> Result<(), String> {
    ValidatedWalletConfigs::new(config.wallets.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_pin_views_config(config: &PinViewsConfig) -> Result<(), String> {
    ValidatedNetworkConfigs::new(config.pinned_networks.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_resolve_valuations_config(config: &ResolveValuationsConfig) -> Result<(), String> {
    ValidatedSymbolConfigs::new(config.symbol_configs.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())?;
    validate_valuation_source_registry(&config.valuation_source_registry)
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

/// State that pins concrete execution views for configured networks.
pub struct PinViewsState;

impl StateSpec for PinViewsState {
    type Config = PinViewsConfig;
    type Context = NoContext;
    type Input = ();
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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        let _config = config.into_inner();
        Ok(Self)
    }
}

impl ReadState for PinViewsState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(adapter_bound_read_state_error(Self::name())))
    }
}

/// State that resolves configured valuation routes.
pub struct ResolveValuationsState {
    config: ResolveValuationsConfig,
}

impl StateSpec for ResolveValuationsState {
    type Config = ResolveValuationsConfig;
    type Context = NoContext;
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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ResolveValuationsState {
    fn run(
        &self,
        views: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(resolve_valuations_from_config(&self.config, &views))
    }
}

/// State that observes one wallet/symbol batch.
pub struct ObserveBatchState;

impl StateSpec for ObserveBatchState {
    type Config = ObserveBatchConfig;
    type Context = NoContext;
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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        let _config = config.into_inner();
        Ok(Self)
    }
}

impl ReadState for ObserveBatchState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        let _input = input;
        future::ready(Err(adapter_bound_read_state_error(Self::name())))
    }
}

/// State that merges non-empty observation batches.
pub struct MergeObservationsState;

impl StateSpec for MergeObservationsState {
    type Config = MergeObservationsConfig;
    type Context = NoContext;
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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        let _config = config.into_inner();
        Ok(Self)
    }
}

impl PureState for MergeObservationsState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(merge_observation_batches(input))
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
        Ok(assemble_snapshot(&self.config, input, 0))
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

/// Builds a pinned view for `network` using the supplied execution anchor.
pub fn pinned_view_for_network(network: &NetworkConfig, anchor: ExecutionAnchor) -> PinnedView {
    PinnedView {
        network_id: network.network_id().to_string(),
        anchor,
    }
}

/// Builds a canonical pinned-view collection.
pub fn pinned_views_from_views(mut views: Vec<PinnedView>) -> PinnedViews {
    views.sort_by(|left, right| left.network_id.cmp(&right.network_id));
    PinnedViews { views }
}

/// Classifies the network read intent for a configured portfolio network.
pub fn network_read_intent_for_network(network: &NetworkConfig) -> PortfolioNetworkReadIntent {
    match network {
        NetworkConfig::Evm {
            network_id,
            chain_id,
            ..
        } => PortfolioNetworkReadIntent::Evm {
            network_id: network_id.to_string(),
            chain_id: chain_id.get(),
        },
        NetworkConfig::Bitcoin { network_id, .. } => PortfolioNetworkReadIntent::Bitcoin {
            network_id: network_id.to_string(),
            source_identity: network_id.to_string(),
        },
    }
}

/// Classifies the network reads needed to pin all configured execution views.
pub fn pin_view_read_intents(config: &PinViewsConfig) -> Vec<PortfolioNetworkReadIntent> {
    config
        .networks()
        .iter()
        .map(network_read_intent_for_network)
        .collect()
}

/// Classifies the network read required by a supported observation batch.
pub fn observe_batch_network_read_intent(
    config: &ObserveBatchConfig,
) -> Option<PortfolioNetworkReadIntent> {
    match (&config.network, &config.symbol.balance_reader) {
        (NetworkConfig::Evm { .. }, BalanceReaderConfig::NativeBalance {})
        | (NetworkConfig::Evm { .. }, BalanceReaderConfig::Erc20Balance { .. })
        | (NetworkConfig::Bitcoin { .. }, BalanceReaderConfig::NativeBalance {}) => {
            Some(network_read_intent_for_network(&config.network))
        }
        (NetworkConfig::Bitcoin { .. }, BalanceReaderConfig::Erc20Balance { .. })
        | (_, BalanceReaderConfig::ProtocolPosition { .. }) => None,
    }
}

/// Classifies the raw balance read required by one observation batch at a pinned anchor.
pub fn observe_batch_read_intent(
    config: &ObserveBatchConfig,
    anchor: &ExecutionAnchor,
) -> Result<PortfolioBalanceReadIntent, PortfolioReadError> {
    match &config.symbol.balance_reader {
        BalanceReaderConfig::NativeBalance {} => match &config.network {
            NetworkConfig::Evm {
                network_id,
                chain_id,
                ..
            } => Ok(PortfolioBalanceReadIntent::EvmNativeBalance {
                network_id: network_id.to_string(),
                chain_id: chain_id.get(),
                account: wallet_evm_address(config)?,
                block_number: evm_block_number_from_anchor(chain_id.get(), anchor)?,
                decimals: config.symbol.decimals.unwrap_or(18),
                anchor: anchor.clone(),
            }),
            NetworkConfig::Bitcoin { network_id, .. } => {
                ensure_bitcoin_anchor(anchor)?;
                Ok(PortfolioBalanceReadIntent::BitcoinNativeBalance {
                    network_id: network_id.to_string(),
                    source_identity: network_id.to_string(),
                    address: wallet_btc_address(config)?,
                    anchor: anchor.clone(),
                    decimals: config.symbol.decimals.unwrap_or(8),
                })
            }
        },
        BalanceReaderConfig::Erc20Balance { token_address } => match &config.network {
            NetworkConfig::Evm {
                network_id,
                chain_id,
                ..
            } => Ok(PortfolioBalanceReadIntent::Erc20Balance {
                network_id: network_id.to_string(),
                chain_id: chain_id.get(),
                account: wallet_evm_address(config)?,
                token_address: token_address.to_string(),
                block_number: evm_block_number_from_anchor(chain_id.get(), anchor)?,
                decimals: config.symbol.decimals,
                anchor: anchor.clone(),
            }),
            NetworkConfig::Bitcoin { .. } => Err(unsupported_balance_reader_error()),
        },
        BalanceReaderConfig::ProtocolPosition { .. } => Err(unsupported_balance_reader_error()),
    }
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
            symbol_id: symbol.symbol_id.to_string(),
            quote: quote.quote,
            priced_symbol_id: quote.priced_symbol_id.to_string(),
            unit_price_dec: unit_price_dec.to_string(),
            valuation_reader_kind: "fixed_unit_price".to_owned(),
            source_refs: Vec::new(),
        }),
        ValuationReaderConfig::DirectPrice { source } => {
            let Some(view) = views
                .views
                .iter()
                .find(|view| view.network_id == source.network_id.as_str())
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
    balance: RawBalanceObservation,
) -> ObservationBatch {
    let mut errors = input.valuations.errors.clone();
    let batch_id = observation_batch_id(&config.wallet.wallet_id, &config.symbol.symbol_id);
    let Some(subject) = input
        .subjects
        .subjects
        .iter()
        .find(|subject| subject.wallet_id == config.wallet.wallet_id.as_str())
    else {
        errors.push(snapshot_error(
            "missing_resolved_subject",
            "missing resolved subject for observation batch",
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(config.network.network_id()),
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
        .find(|view| view.network_id == config.network.network_id().as_str())
    else {
        return observation_batch_missing_pinned_view(config, errors);
    };
    let amount_dec = format_u256_units(&balance.raw, balance.decimals);
    let values = observation_values(config, input, &amount_dec, &mut errors);
    if values.is_empty() {
        errors.push(snapshot_error(
            "missing_observation_values",
            "observation had no resolved valuation values",
            Some(&config.wallet.wallet_id),
            Some(&config.symbol.symbol_id),
            Some(config.network.network_id()),
            Some(balance_reader_kind(&config.symbol.balance_reader)),
        ));
        return ObservationBatch {
            batch_id,
            observations: Vec::new(),
            errors,
        };
    }

    let anchor_source = balance.anchor.as_ref().unwrap_or(&view.anchor);
    let anchor = match anchor_source {
        ExecutionAnchor::Evm {
            chain_id,
            block_number,
        } => ObservationAnchor::Evm {
            chain_id: *chain_id,
            block_number: *block_number,
        },
        ExecutionAnchor::Bitcoin { height, block_hash } => ObservationAnchor::Bitcoin {
            height: *height,
            block_hash: block_hash.clone(),
        },
    };
    let mut observation = Observation {
        wallet_id: subject.wallet_id.clone(),
        symbol_id: config.symbol.symbol_id.to_string(),
        display_symbol: config.symbol.display_symbol.clone(),
        kind: config.symbol.kind,
        role: config.symbol.role,
        network_id: config.network.network_id().to_string(),
        protocol: config.symbol.protocol.as_ref().map(ToString::to_string),
        quantity: ObservationQuantity {
            raw_dec: balance.raw.to_string(),
            decimals: balance.decimals,
            amount_dec,
        },
        values,
        source: ObservationSource {
            balance_reader_kind: balance_reader_kind(&config.symbol.balance_reader).to_owned(),
            network_id: config.network.network_id().to_string(),
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
            Some(config.network.network_id()),
            Some(balance_reader_kind(&config.symbol.balance_reader)),
        )],
    }
}

/// Returns the pinned execution anchor for `network_id`.
pub fn pinned_anchor_for<'a>(
    views: &'a PinnedViews,
    network_id: &str,
) -> Option<&'a ExecutionAnchor> {
    views
        .views
        .iter()
        .find(|view| view.network_id == network_id)
        .map(|view| &view.anchor)
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

/// Builds an observation batch containing a missing pinned-view domain error.
pub fn observation_batch_missing_pinned_view(
    config: &ObserveBatchConfig,
    mut errors: Vec<PortfolioSnapshotError>,
) -> ObservationBatch {
    errors.push(snapshot_error(
        "missing_pinned_view",
        "missing pinned view for observation batch",
        Some(&config.wallet.wallet_id),
        Some(&config.symbol.symbol_id),
        Some(config.network.network_id()),
        None,
    ));
    ObservationBatch {
        batch_id: observation_batch_id(&config.wallet.wallet_id, &config.symbol.symbol_id),
        observations: Vec::new(),
        errors,
    }
}

fn evm_block_number_from_anchor(
    expected_chain_id: u64,
    anchor: &ExecutionAnchor,
) -> Result<u64, PortfolioReadError> {
    match anchor {
        ExecutionAnchor::Evm {
            chain_id,
            block_number,
        } if *chain_id == expected_chain_id => Ok(*block_number),
        ExecutionAnchor::Evm { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio EVM read anchor did not match the configured network",
        )),
        ExecutionAnchor::Bitcoin { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio EVM read received a non-EVM execution anchor",
        )),
    }
}

fn ensure_bitcoin_anchor(anchor: &ExecutionAnchor) -> Result<(), PortfolioReadError> {
    match anchor {
        ExecutionAnchor::Bitcoin { .. } => Ok(()),
        ExecutionAnchor::Evm { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio Bitcoin read received a non-Bitcoin execution anchor",
        )),
    }
}

fn wallet_evm_address(config: &ObserveBatchConfig) -> Result<String, PortfolioReadError> {
    let Some(address) = config.wallet.subject.evm_address() else {
        return Err(PortfolioReadError::new(
            "invalid_wallet_subject",
            "wallet subject was not an EVM address for portfolio EVM read",
        ));
    };
    Ok(address.to_string())
}

fn wallet_btc_address(config: &ObserveBatchConfig) -> Result<String, PortfolioReadError> {
    if config.wallet.subject.kind() != WalletSubjectKind::BitcoinAddress {
        return Err(PortfolioReadError::new(
            "invalid_wallet_subject",
            "wallet subject was not a Bitcoin address for portfolio Bitcoin read",
        ));
    }
    Ok(config.wallet.subject.address_str().to_owned())
}

fn unsupported_balance_reader_error() -> PortfolioReadError {
    PortfolioReadError::new(
        "unsupported_balance_reader",
        "portfolio balance reader is not enabled in the typed portfolio runner",
    )
}

fn adapter_bound_read_state_error(state_name: &str) -> StateError {
    StateError::Message(format!(
        "{state_name} requires adapter-bound external read execution"
    ))
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
                .get(wallet.wallet_id.as_str())
                .cloned()
                .unwrap_or_else(|| ResolvedSubject {
                    wallet_id: wallet.wallet_id.to_string(),
                    address: wallet.subject.address_str().to_owned(),
                    subject_kind: wallet.subject.kind(),
                    network_id: wallet.network_id.to_string(),
                    implementation_kind: wallet_implementation_kind(&wallet.implementation)
                        .to_owned(),
                });
            let mut wallet = WalletSnapshot {
                wallet_id: wallet.wallet_id.to_string(),
                address: subject.address,
                subject_kind: subject.subject_kind,
                network_id: subject.network_id,
                observations: observations_by_wallet
                    .remove(wallet.wallet_id.as_str())
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
        schema_version: config.snapshot_version(),
        portfolio_id: config.portfolio.portfolio_id.to_string(),
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
        } if protocol.as_str() == AAVE_V3_PROTOCOL_ID => match reader.as_str() {
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
            valuation.symbol_id == config.symbol.symbol_id.as_str()
                && valuation.quote == quote.quote
        }) else {
            errors.push(snapshot_error(
                "missing_resolved_valuation",
                format!("missing valuation for quote `{}`", quote.quote),
                Some(&config.wallet.wallet_id),
                Some(&config.symbol.symbol_id),
                Some(config.network.network_id()),
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
                Some(config.network.network_id()),
                Some("valuation"),
            )),
        }
    }
    values.sort_by_key(|value| value.quote);
    values
}

fn validate_observe_batch_config(config: &ObserveBatchConfig) -> Result<(), String> {
    validate_symbol_config(&config.symbol).map_err(|error| error.to_string())?;
    if &config.wallet.network_id != config.network.network_id() {
        return Err(format!(
            "wallet `{}` network `{}` did not match observation network `{}`",
            config.wallet.wallet_id,
            config.wallet.network_id,
            config.network.network_id()
        ));
    }
    if &config.symbol.network_id != config.network.network_id() {
        return Err(format!(
            "symbol `{}` network `{}` did not match observation network `{}`",
            config.symbol.symbol_id,
            config.symbol.network_id,
            config.network.network_id()
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
        if protocol.as_str() == AAVE_V3_PROTOCOL_ID
            && reader.as_str() != protocol_config.reader_name()
        {
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
