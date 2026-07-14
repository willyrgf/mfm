#![warn(missing_docs)]
//! Typed portfolio-domain state contracts for fact-backed report-only snapshots.
//!
//! After the collectors cutover, `portfolio_snapshot` is select-centric:
//! `ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_model::portfolio::PortfolioConfig;
//!
//! fn workflow_config(config: PortfolioConfig) -> PortfolioConfig {
//!     config.normalized()
//! }
//! ```

mod selection;

#[path = "decimal.rs"]
mod decimal;
use self::decimal::{multiply_decimal_strings, DecimalValue};

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
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.portfolio";
const ADAPTER_NAME: &str = "typed-portfolio";
const ADAPTER_VERSION: &str = "mfm.portfolio.adapter.typed.v1";
/// Default store scope used by portfolio Platform holding selection.
pub const DEFAULT_PORTFOLIO_STORE_SCOPE: &str = "mfm.store.default";

#[path = "config.rs"]
mod config;
pub use self::config::*;
#[path = "transforms.rs"]
mod transforms;
pub use self::transforms::*;
#[path = "states.rs"]
mod states;
pub use self::states::*;

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

/// Typed readiness evidence consumed before report fact selection.
///
/// Standalone reports use zero collector counts. Composed reports carry the number of
/// successfully summarized collector networks so the report graph cannot be detached from its
/// collector fan-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-inputs-ready",
    schema = "mfm.portfolio.portfolio_inputs_ready"
)]
pub struct PortfolioInputsReady {
    /// Number of Bitcoin collector summaries consumed by readiness.
    pub bitcoin_network_count: u32,
    /// Number of EVM collector summaries consumed by readiness.
    pub evm_network_count: u32,
}

impl PortfolioInputsReady {
    /// Creates readiness evidence from validated summary counts.
    pub const fn new(bitcoin_network_count: u32, evm_network_count: u32) -> Self {
        Self {
            bitcoin_network_count,
            evm_network_count,
        }
    }
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

#[cfg(test)]
mod tests;
