//! Adapter tests for receipt-pinned fact selection and retained-evidence hydration.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::{
    derive_fact_content_identity_from_typed_values, typed_fact_subject_evidence, FactAudience,
    FactClaimId, FactFieldValueType, FactProducerProvenance, FactQueryReceipt, FactQueryResultRow,
    FactQueryScope, FactResponseEvidence, FactSubjectRef, FactVisibility, FactVisibilityScope,
    InternalFactRef, InternalFactRefParts, StoreCommitOrder, StoreReadFrontier, StoreScopeRef,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, NetworkPin, PortfolioConfig,
};
use mfm_portfolio_model::symbol::{
    HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use mfm_program::{MfmFactType, ValidatedConfig};
use mfm_state_portfolio::{
    CollectedHoldingReceipt, HoldingManifestEntry, HoldingRequirementKey, HoldingSourceKey,
    PortfolioCollectionReceipt, SelectHoldingsInput,
};
use mfm_states_btc::{
    BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact, BtcAddressBalanceSubject,
};
use mfm_states_evm::{
    EvmAddressErc20BalanceResponse, EvmAddressErc20BalanceSnapshotFact,
    EvmAddressErc20BalanceSubject, EvmAddressNativeBalanceResponse,
    EvmAddressNativeBalanceSnapshotFact, EvmAddressNativeBalanceSubject,
};
use mfm_store::v1::test_support::{
    fact_query_receipt_for_test, FactQueryReceiptFixtureInputForTest,
};

#[path = "tests/support.rs"]
mod support;
use self::support::*;
#[path = "tests/behavior.rs"]
mod behavior;
