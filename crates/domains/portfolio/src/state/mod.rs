//! Typed portfolio state contracts and deterministic reducers.

mod collection_receipt;
mod config;
mod decimal;
mod holding_read;
mod selection;
mod states;
mod transforms;

use collection_receipt::HoldingRequirementKey;
pub use collection_receipt::{SelectHoldingsInput, SelectHoldingsInputHandles};
pub use config::{
    AssembleSnapshotConfig, ProjectReportConfig, SelectHoldingsConfig,
    SelectHoldingsFactDescriptors,
};
use decimal::{multiply_decimal_strings, DecimalValue};
pub use holding_read::{
    PortfolioHoldingFactEvidence, SelectHoldingsReadEvidence, SelectHoldingsReadPlan,
};
use selection::{
    portfolio_holding_selection_policy_digest, project_network_pins_from_observations,
    HoldingCandidate, SelectedHolding, SelectedHoldingMaterial,
    PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID,
};
pub use selection::{PortfolioHoldingErrorCode, PortfolioHoldingSelectionError};
pub use states::{AssembleSnapshotState, ProjectReportState, SelectHoldingsState};
use transforms::{
    assemble_snapshot, observations_from_selected_holdings, project_report_from_snapshot,
    symbols_by_id_map,
};

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AnchoredHoldingSource, Observation, ObservationQuantity, ObservationValue, PortfolioConfig,
    PortfolioQuoteTotal, PortfolioReport, PortfolioSnapshot, QuoteCode, SymbolConfig,
    ValidatedPortfolioConfig, WalletReport, WalletSnapshot,
};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{NoCaps, Pure, ReadExternal};
use mfm_facts::FactQueryReadCapability;
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{
    AdapterBindingSpec, ExternalReadEvidenceSet, NoContext, PureState, ReadState, StateError,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs, StateInput};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.portfolio";
const ADAPTER_NAME: &str = "typed-portfolio";
const ADAPTER_VERSION: &str = "mfm.portfolio.adapter.typed.v2";

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
