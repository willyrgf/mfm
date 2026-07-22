#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Pure portfolio model, state, and operation contracts.
//!
//! Internal role modules are private and flow in one direction: model, state, then operation.
//! Live fact selection and runtime binding belong outside this crate.
//!
//! ```rust
//! use mfm_portfolio::{decode_portfolio_config, PortfolioConfig, QuoteCode};
//!
//! # fn checked(raw: &serde_json::Value) -> Result<PortfolioConfig, Box<dyn std::error::Error>> {
//! let config = decode_portfolio_config(raw)?;
//! assert!(config.quote_codes.contains(&QuoteCode::Usd));
//! # Ok(config)
//! # }
//! ```
//!
//! Role modules are not public compatibility namespaces.
//!
//! ```compile_fail
//! use mfm_portfolio::model::PortfolioConfig;
//! ```

mod model;
mod operation;
mod state;

pub use model::ids::{
    BitcoinSourceIdentityId, ExternalSignerId, KeystoreEntryId, NetworkId, NormalizedEvmAddress,
    PortfolioId, PortfolioScalarError, SymbolId, UnitPriceDecimal, WalletId,
};
pub use model::metadata::{PublicMetadata, PublicMetadataError};
pub use model::portfolio::{
    decode_portfolio_config, ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, NetworkPin,
    PortfolioConfig, PortfolioConfigError, PortfolioQuoteTotal, PortfolioReport, PortfolioSnapshot,
    ValidatedPortfolioConfig, WalletReport, WalletSnapshot, EVM_NETWORK_HOLDING_SOURCE_LIMIT,
    PORTFOLIO_HOLDING_RELATION_LIMIT, PORTFOLIO_NETWORK_LIMIT, PORTFOLIO_SYMBOL_LIMIT,
    PORTFOLIO_WALLET_LIMIT,
};
pub use model::symbol::{
    validate_symbol_config, AnchoredHoldingSource, HoldingSourceConfig, Observation,
    ObservationQuantity, ObservationValue, QuoteCode, QuoteValuationConfig, SymbolConfig,
    SymbolConfigError, SymbolValuationConfig,
};
pub use model::wallet::{
    BitcoinAddress, WalletConfig, WalletConfigError, WalletImplementationConfig, WalletSubject,
    WalletSubjectKind,
};
pub use operation::{
    portfolio_snapshot_authoring_catalog, portfolio_snapshot_operation_registry,
    portfolio_snapshot_program_draft, portfolio_snapshot_program_launch_plan,
    portfolio_snapshot_public_output_schema_id, portfolio_snapshot_state_registry,
    register_portfolio_snapshot_certification_descriptors, PortfolioReportOperation,
    PortfolioReportOperationOutputs, PortfolioSnapshotOperation, PortfolioSnapshotOperationOutputs,
};
pub use state::{
    portfolio_adapter_kind, portfolio_adapter_version, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotInputHandles, AssembleSnapshotState,
    PortfolioHoldingErrorCode, PortfolioHoldingFactEvidence, PortfolioHoldingSelectionError,
    PortfolioPublicOutputs, ProjectReportConfig, ProjectReportInput, ProjectReportInputHandles,
    ProjectReportState, SelectHoldingsConfig, SelectHoldingsFactDescriptors, SelectHoldingsInput,
    SelectHoldingsInputHandles, SelectHoldingsReadEvidence, SelectHoldingsReadPlan,
    SelectHoldingsState, SelectedHoldings,
};
