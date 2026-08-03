#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Pure portfolio model and structured snapshot operation.
//!
//! The sole production operation authors declaration-ordered EVM balance reads
//! and aggregates their typed results. Store-backed holding selection,
//! aggregate live readers, Bitcoin execution, and replay helpers are absent.
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

mod entry;
mod model;
mod state;
mod structured;

pub use entry::{PortfolioSnapshotSelector, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID};
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
pub use state::{
    EvmRoutingBinding, PortfolioPublicOutputs, PortfolioRoutingManifest, PortfolioSnapshotFailure,
    ValidatedEvmCollectionPosition, ValidatedPortfolioSnapshotSelection,
    PORTFOLIO_ROUTING_MANIFEST_VERSION, VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION,
};
pub use structured::{
    register_portfolio_process, structured_portfolio_lane_inputs,
    structured_portfolio_snapshot_program, PortfolioProcessQualification, PortfolioSnapshotInput,
    STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID,
};
