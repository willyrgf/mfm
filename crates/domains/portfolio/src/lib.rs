#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Pure portfolio model, same-run state, and EVM-only graph contracts.
//!
//! The sole production operation consumes decomposed EVM collection outputs
//! through typed graph edges. Store-backed holding selection, aggregate live
//! readers, Bitcoin execution, and replay helpers are intentionally absent.
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
mod product;
mod qualification;
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
    portfolio_snapshot_entry_point_contract, portfolio_snapshot_planning_profile,
    PortfolioSnapshotSelector, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID, PORTFOLIO_SNAPSHOT_OPERATION_ID,
};
pub use product::{
    portfolio_snapshot_entry_point_registration, portfolio_snapshot_routing_manifest_member_path,
    portfolio_snapshot_unit_config_member_path, PortfolioSnapshotEntryPointArtifacts,
    PORTFOLIO_SNAPSHOT_ROUTING_MANIFEST_MEMBER_PATH, PORTFOLIO_SNAPSHOT_UNIT_CONFIG_MEMBER_PATH,
};
pub use qualification::{
    portfolio_snapshot_callback_surfaces, portfolio_snapshot_value_contracts,
    qualify_portfolio_snapshot_states, PortfolioSnapshotCallbackSurfaces,
    PortfolioSnapshotStateArtifacts, PortfolioSnapshotStateImplementations,
    PortfolioSnapshotValueContracts, PortfolioStateCallbackSurface,
    QualifiedPortfolioSnapshotStates, PORTFOLIO_STATE_CALLBACK_SURFACE_VERSION,
};
pub use state::{
    portfolio_snapshot_public_output_schema_id, AssemblePortfolioSnapshotState, EvmRoutingBinding,
    InvalidPortfolioSnapshotSelection, PortfolioPublicOutputs, PortfolioReportProjectionInput,
    PortfolioRoutingManifest, PortfolioSnapshotAssemblyInput, PortfolioSnapshotFailure,
    PortfolioSnapshotSelectionInput, ProjectPortfolioReportState,
    ValidatePortfolioSnapshotSelectionState, ValidatedEvmCollectionPosition,
    ValidatedPortfolioSnapshotSelection, PORTFOLIO_ROUTING_MANIFEST_VERSION,
    VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION,
};
