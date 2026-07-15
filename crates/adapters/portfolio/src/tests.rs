//! Adapter tests for fact-backed SelectHoldings (Platform fact-index + hydrate).
//!
//! These exercise the shipped `select_holdings` path with retained FactResponse
//! artifacts — not hand-built Observations.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

use mfm_canonical::sha256_digest_bytes;
use mfm_facts::{
    fact_descriptor_hash, DescriptorCatalogWatermark, FactAudience, FactClaimId,
    FactFieldValueType, FactProducerProvenance, FactQueryReceipt, FactQueryResultRow,
    FactQueryScope, FactResponseEvidence, FactSubjectRef, FactVisibility, FactVisibilityScope,
    InternalFactRef, InternalFactRefParts, StoreCommitOrder, StoreReadFrontier, StoreScopeRef,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig,
};
use mfm_portfolio_model::symbol::{
    HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use mfm_program::MfmFactType;
use mfm_program::ValidatedConfig;
use mfm_state_portfolio::{
    project_network_pins_from_observations, resolve_subjects_from_config, ResolveSubjectsConfig,
};
use mfm_states_btc::{BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact};
use mfm_states_evm::{EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact};
use mfm_store::v1::test_support::{
    fact_query_receipt_for_test, FactQueryReceiptFixtureInputForTest,
};

#[path = "tests/support.rs"]
mod support;
use self::support::*;
#[path = "tests/behavior.rs"]
mod behavior;
