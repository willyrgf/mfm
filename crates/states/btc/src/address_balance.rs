//! Bitcoin address balance snapshot fact (`bitcoin.address_balance_snapshot`).

use mfm_btc_capabilities::BitcoinBlockHash;
use mfm_facts::{FactAudience, FactVisibility};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

use crate::BtcStateError;

/// Returns Platform visibility for Bitcoin address balance snapshot facts.
pub fn address_balance_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

/// Subject identity for a Bitcoin address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance.subject"
)]
pub struct BtcAddressBalanceSubject {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
}

impl BtcAddressBalanceSubject {
    /// Creates subject material for an address balance snapshot.
    pub fn new(
        network: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        address: impl Into<String>,
    ) -> Result<Self, BtcStateError> {
        let network = network.into();
        let bitcoin_network = bitcoin_network.into();
        let semantic_source_identity = semantic_source_identity.into();
        let address = address.into();
        if network.trim().is_empty()
            || bitcoin_network.trim().is_empty()
            || semantic_source_identity.trim().is_empty()
            || address.trim().is_empty()
        {
            return Err(BtcStateError::InvalidInput {
                reason: "address balance subject fields must be non-empty".to_owned(),
            });
        }
        if semantic_source_identity.contains("://")
            || semantic_source_identity.contains('@')
            || semantic_source_identity.contains('/')
        {
            return Err(BtcStateError::InvalidInput {
                reason: "semantic_source_identity must not encode runtime routes".to_owned(),
            });
        }
        Ok(Self {
            network,
            bitcoin_network,
            semantic_source_identity,
            address,
        })
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the Bitcoin Core network tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the observed Bitcoin address.
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// Observed result for a Bitcoin address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_response",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance.response"
)]
pub struct BtcAddressBalanceResponse {
    anchor_height: u64,
    anchor_hash: String,
    balance_sats: u64,
    coverage: String,
    source_status: String,
}

impl BtcAddressBalanceResponse {
    /// Creates response material, failing closed on malformed hash or inadmissible coverage/status.
    pub fn new(
        anchor_height: u64,
        anchor_hash: impl Into<String>,
        balance_sats: u64,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, BtcStateError> {
        let anchor_hash = require_btc_anchor_hash(anchor_hash)?;
        if !coverage.is_admissible_for_write() {
            return Err(BtcStateError::InvalidInput {
                reason: format!(
                    "coverage {} is not admissible for Platform write",
                    coverage.as_str()
                ),
            });
        }
        if !source_status.is_admissible_for_write() {
            return Err(BtcStateError::InvalidInput {
                reason: format!(
                    "source_status {} is not admissible for Platform write",
                    source_status.as_str()
                ),
            });
        }
        Ok(Self {
            anchor_height,
            anchor_hash,
            balance_sats,
            coverage: coverage.as_str().to_owned(),
            source_status: source_status.as_str().to_owned(),
        })
    }

    /// Returns the mandatory anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the mandatory anchor block hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
    }

    /// Returns the total balance in satoshis.
    pub const fn balance_sats(&self) -> u64 {
        self.balance_sats
    }

    /// Returns the coverage tag.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the holding source status tag.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Parses coverage as a closed enum.
    pub fn coverage_status(&self) -> Result<CoverageStatus, BtcStateError> {
        self.coverage
            .parse()
            .map_err(|_| BtcStateError::InvalidInput {
                reason: format!("unknown coverage status {:?}", self.coverage),
            })
    }

    /// Parses holding source status as a closed enum.
    pub fn holding_source_status(&self) -> Result<HoldingSourceStatus, BtcStateError> {
        self.source_status
            .parse()
            .map_err(|_| BtcStateError::InvalidInput {
                reason: format!("unknown holding source status {:?}", self.source_status),
            })
    }
}

/// Platform fact for a Bitcoin address balance snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_snapshot_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance_snapshot"
)]
#[mfm_fact(kind = "bitcoin.address_balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.bitcoin_network",
    source = "subject",
    path = "bitcoin_network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.address",
    source = "subject",
    path = "address",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.anchor_height",
    source = "result",
    path = "anchor_height",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.anchor_hash",
    source = "result",
    path = "anchor_hash",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.balance_sats",
    source = "result",
    path = "balance_sats",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.coverage",
    source = "result",
    path = "coverage",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.source_status",
    source = "result",
    path = "source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "metadata.store_commit_order",
    source = "metadata",
    metadata = "store_commit_order",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.anchor_height.desc",
    term(
        field = "result.anchor_height",
        direction = "descending",
        nulls = "last"
    )
))]
#[mfm_fact(ordering(
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct BtcAddressBalanceSnapshotFact {
    subject: BtcAddressBalanceSubject,
    response: BtcAddressBalanceResponse,
}

impl BtcAddressBalanceSnapshotFact {
    /// Creates a balance snapshot fact from validated subject and response material.
    pub const fn new(
        subject: BtcAddressBalanceSubject,
        response: BtcAddressBalanceResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Creates a fact, re-validating response admission rules for fail-closed writers.
    pub fn try_new(
        subject: BtcAddressBalanceSubject,
        anchor_height: u64,
        anchor_hash: impl Into<String>,
        balance_sats: u64,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, BtcStateError> {
        let response = BtcAddressBalanceResponse::new(
            anchor_height,
            anchor_hash,
            balance_sats,
            coverage,
            source_status,
        )?;
        Ok(Self::new(subject, response))
    }

    /// Returns the subject material.
    pub const fn subject(&self) -> &BtcAddressBalanceSubject {
        &self.subject
    }

    /// Returns the response material.
    pub const fn response(&self) -> &BtcAddressBalanceResponse {
        &self.response
    }
}

/// Source-near holding material normalized from a Bitcoin address balance fact.
///
/// Contains no portfolio wallet_id/symbol_id; report join happens later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedBtcAddressHolding {
    /// Semantic network id.
    pub network: String,
    /// Bitcoin Core network tag.
    pub bitcoin_network: String,
    /// Observed address.
    pub address: String,
    /// Total balance in satoshis.
    pub balance_sats: u64,
    /// Anchor height.
    pub anchor_height: u64,
    /// Anchor block hash.
    pub anchor_hash: String,
    /// Coverage claim.
    pub coverage: CoverageStatus,
    /// Holding source status.
    pub source_status: HoldingSourceStatus,
}

/// Pure normalize: fact → source-near holding fields. Fail closed on missing hash or
/// inadmissible coverage/status tags.
pub fn normalize_btc_address_balance_fact(
    fact: &BtcAddressBalanceSnapshotFact,
) -> Result<NormalizedBtcAddressHolding, BtcStateError> {
    normalize_btc_address_balance(fact.subject(), fact.response())
}

/// Pure normalize from subject + response material.
pub fn normalize_btc_address_balance(
    subject: &BtcAddressBalanceSubject,
    response: &BtcAddressBalanceResponse,
) -> Result<NormalizedBtcAddressHolding, BtcStateError> {
    let anchor_hash = require_btc_anchor_hash(response.anchor_hash())?;
    let coverage = response.coverage_status()?;
    let source_status = response.holding_source_status()?;
    Ok(NormalizedBtcAddressHolding {
        network: subject.network().to_owned(),
        bitcoin_network: subject.bitcoin_network().to_owned(),
        address: subject.address().to_owned(),
        balance_sats: response.balance_sats(),
        anchor_height: response.anchor_height(),
        anchor_hash,
        coverage,
        source_status,
    })
}

/// Requires a 32-byte lowercase hex Bitcoin block hash (empty and malformed fail closed).
fn require_btc_anchor_hash(anchor_hash: impl Into<String>) -> Result<String, BtcStateError> {
    let anchor_hash = anchor_hash.into();
    BitcoinBlockHash::new(anchor_hash.trim())
        .map(|hash| hash.to_string())
        .map_err(|_| BtcStateError::InvalidInput {
            reason: "address balance anchor_hash must be a 32-byte hex hash".to_owned(),
        })
}

#[cfg(test)]
#[path = "address_balance_tests.rs"]
mod tests;
