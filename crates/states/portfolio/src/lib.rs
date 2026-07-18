#![warn(missing_docs)]
//! Typed portfolio-domain state contracts for certified portfolio snapshots.
//!
//! [`SelectHoldingsState`] consumes Bitcoin and generic EVM collection receipts directly, then
//! reads and reverifies every selected balance from the fact store before snapshot assembly.
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

mod collection_receipt;
mod holding_read;
mod selection;

#[path = "decimal.rs"]
mod decimal;
use self::decimal::{multiply_decimal_strings, DecimalValue};

pub use collection_receipt::{
    HoldingRequirementKey, SelectHoldingsInput, SelectHoldingsInputHandles,
};
pub use holding_read::{
    PortfolioHoldingFactResponse, SelectHoldingsReadEvidence, SelectHoldingsReadPlan,
};
pub use selection::{
    portfolio_holding_select_scope_decision_hash, portfolio_holding_selection_policy_digest,
    project_network_pins_from_observations, HoldingCandidate, PortfolioHoldingErrorCode,
    PortfolioHoldingSelectionError, SelectedHolding, SelectedHoldingMaterial,
    PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID,
};

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::NoCaps;
use mfm_effects::{Pure, ReadExternal};
use mfm_fact_capabilities::FactIndexReadCapability;
use mfm_facts::StoreScopeRef;
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_portfolio_model::portfolio::{
    PortfolioConfig, PortfolioQuoteTotal, PortfolioReport, PortfolioSnapshot,
    ValidatedPortfolioConfig, WalletReport, WalletSnapshot,
};
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, Observation, ObservationQuantity, ObservationValue, QuoteCode,
    SymbolConfig,
};
use mfm_program::{
    AdapterBindingSpec, ExternalReadEvidenceSet, NoContext, PureState, ReadState, StateError,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs, StateInput};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.portfolio";
const ADAPTER_NAME: &str = "typed-portfolio";
const ADAPTER_VERSION: &str = "mfm.portfolio.adapter.typed.v1";
const PORTFOLIO_STORE_SCOPE: &str = "mfm.store.default";

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

/// Selected receipt-authorized holdings emitted by fact selection (without valuation join).
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
    /// Selected Bitcoin and EVM holdings from Platform fact selection.
    pub holdings: SelectedHoldings,
}

/// Input consumed by report projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.project_report")]
pub struct ProjectReportInput {
    /// Assembled portfolio snapshot.
    pub snapshot: PortfolioSnapshot,
}

/// Public output contract for the complete portfolio snapshot root.
///
/// It exposes only the snapshot and report projections; collection receipts, fact identities, and
/// selection evidence remain internal certified graph values.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.portfolio.public_outputs")]
pub struct PortfolioPublicOutputs<'program, 'scope> {
    /// Public portfolio snapshot.
    pub snapshot: mfm_program::Handle<'program, 'scope, PortfolioSnapshot>,
    /// Projected portfolio report.
    pub report: mfm_program::Handle<'program, 'scope, PortfolioReport>,
}

#[cfg(test)]
mod tests;
