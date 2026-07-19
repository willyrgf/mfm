use std::num::NonZeroU64;

use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcAddress, BtcBalanceReadRequest, BtcBalanceReadResponse, BtcBlockHash,
    BtcChainHeadRequest, BtcChainHeadResponse, BtcFinality, BtcHeadKind, BtcHeadSelection,
    BtcNetworkId, BtcSourceBinding, BtcSourceIdentity, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::{validate_bitcoin_network, BtcStateError};

/// Complete deterministic plan for one Bitcoin chain-head read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_read_plan",
    version = "1",
    schema = "mfm.bitcoin.external_read.chain_head.plan"
)]
pub struct BtcChainHeadReadPlan {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    head_kind: String,
    finality: String,
    confirmation_depth: Option<u64>,
}

impl BtcChainHeadReadPlan {
    /// Creates a checked plan for one source-bound chain-head read.
    pub fn new(
        network: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        selection: BtcHeadSelection,
    ) -> Result<Self, BtcStateError> {
        let plan = Self {
            network: network.into(),
            bitcoin_network: bitcoin_network.into(),
            semantic_source_identity: semantic_source_identity.into(),
            head_kind: selection.head_kind().as_str().to_owned(),
            finality: selection.finality().as_str().to_owned(),
            confirmation_depth: selection.finality().confirmation_depth(),
        };
        plan.binding()?;
        plan.request()?;
        Ok(plan)
    }

    /// Reconstructs the checked process-local source binding.
    pub fn binding(&self) -> Result<BtcSourceBinding, BtcStateError> {
        let network_id = BtcNetworkId::new(&self.network)?;
        let source_identity = BtcSourceIdentity::new(&self.semantic_source_identity)?;
        validate_bitcoin_network(&self.bitcoin_network)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let bitcoin_network = BitcoinNetworkTag::new(&self.bitcoin_network)?;
        Ok(BtcSourceBinding::new(
            network_id,
            source_identity,
            bitcoin_network,
        ))
    }

    /// Reconstructs the exact capability request declared by this plan.
    pub fn request(&self) -> Result<BtcChainHeadRequest, BtcStateError> {
        let selection = match (
            self.head_kind.as_str(),
            self.finality.as_str(),
            self.confirmation_depth,
        ) {
            ("best", "best_available", None) => BtcHeadSelection::best(),
            ("confirmed", "confirmations", Some(confirmations)) => {
                BtcHeadSelection::confirmed(confirmations)?
            }
            _ => {
                return Err(BtcStateError::InvalidInput {
                    reason: "chain-head plan contains an invalid selection".to_owned(),
                })
            }
        };
        Ok(BtcChainHeadRequest::new(selection))
    }
}

/// Canonical retained evidence for one Bitcoin chain-head read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_read_evidence",
    version = "1",
    schema = "mfm.bitcoin.external_read.chain_head.evidence"
)]
pub struct BtcChainHeadReadEvidence {
    head_kind: String,
    finality: String,
    confirmation_depth: Option<u64>,
    block_height: u64,
    block_hash: String,
    provider_time_unix_ms: Option<u64>,
    network: String,
    semantic_source_identity: String,
    bitcoin_network: String,
    observed_bitcoin_network: String,
    source_status: String,
}

impl BtcChainHeadReadEvidence {
    /// Captures canonical evidence from a capability response.
    pub fn from_response(response: &BtcChainHeadResponse) -> Self {
        Self {
            head_kind: response.head_kind.as_str().to_owned(),
            finality: response.finality.as_str().to_owned(),
            confirmation_depth: response.finality.confirmation_depth(),
            block_height: response.block_height,
            block_hash: response.block_hash.as_str().to_owned(),
            provider_time_unix_ms: response.provider_time_unix_ms,
            network: response.evidence.network_id.as_str().to_owned(),
            semantic_source_identity: response.evidence.source_identity.as_str().to_owned(),
            bitcoin_network: response.evidence.bitcoin_network.clone(),
            observed_bitcoin_network: response.evidence.observed_bitcoin_network.clone(),
            source_status: response.evidence.source_status.as_str().to_owned(),
        }
    }

    /// Reconstructs and checks the response against the state-authored plan.
    pub fn response(
        &self,
        plan: &BtcChainHeadReadPlan,
    ) -> Result<BtcChainHeadResponse, BtcStateError> {
        let binding = plan.binding()?;
        let request = plan.request()?;
        let selection = request.selection();
        let head_kind = parse_head_kind(&self.head_kind)?;
        let finality = parse_finality(&self.finality, self.confirmation_depth)?;
        if self.network != binding.network_id().as_str()
            || self.semantic_source_identity != binding.source_identity().as_str()
            || self.bitcoin_network != binding.bitcoin_network().as_str()
            || head_kind != selection.head_kind()
            || finality != selection.finality()
        {
            return Err(BtcStateError::SourceMismatch);
        }
        let evidence = RedactedBtcSourceEvidence::from_binding(
            &binding,
            self.observed_bitcoin_network.clone(),
            parse_source_status(&self.source_status)?,
        )?;
        Ok(BtcChainHeadResponse {
            evidence,
            head_kind,
            finality,
            block_height: self.block_height,
            block_hash: BtcBlockHash::new(&self.block_hash)?,
            provider_time_unix_ms: self.provider_time_unix_ms,
        })
    }
}

/// Complete deterministic plan for one pinned Bitcoin address-balance read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_read_plan",
    version = "1",
    schema = "mfm.bitcoin.external_read.address_balance.plan"
)]
pub struct BtcAddressBalanceReadPlan {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
    block_height: u64,
    block_hash: String,
}

impl BtcAddressBalanceReadPlan {
    /// Creates a checked source-bound plan for one pinned balance read.
    pub fn new(
        network: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        address: impl Into<String>,
        block_height: u64,
        block_hash: impl Into<String>,
    ) -> Result<Self, BtcStateError> {
        let plan = Self {
            network: network.into(),
            bitcoin_network: bitcoin_network.into(),
            semantic_source_identity: semantic_source_identity.into(),
            address: address.into(),
            block_height,
            block_hash: block_hash.into(),
        };
        plan.binding()?;
        plan.request()?;
        Ok(plan)
    }

    /// Reconstructs the checked process-local source binding.
    pub fn binding(&self) -> Result<BtcSourceBinding, BtcStateError> {
        let network_id = BtcNetworkId::new(&self.network)?;
        let source_identity = BtcSourceIdentity::new(&self.semantic_source_identity)?;
        validate_bitcoin_network(&self.bitcoin_network)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let bitcoin_network = BitcoinNetworkTag::new(&self.bitcoin_network)?;
        Ok(BtcSourceBinding::new(
            network_id,
            source_identity,
            bitcoin_network,
        ))
    }

    /// Reconstructs the exact capability request declared by this plan.
    pub fn request(&self) -> Result<BtcBalanceReadRequest, BtcStateError> {
        Ok(BtcBalanceReadRequest::new(
            BtcAddress::new(&self.address)?,
            self.block_height,
            BtcBlockHash::new(&self.block_hash)?,
        ))
    }
}

/// Canonical retained evidence for one pinned Bitcoin address-balance read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_read_evidence",
    version = "1",
    schema = "mfm.bitcoin.external_read.address_balance.evidence"
)]
pub struct BtcAddressBalanceReadEvidence {
    address: String,
    balance_sats: u64,
    block_height: u64,
    block_hash: String,
    network: String,
    semantic_source_identity: String,
    bitcoin_network: String,
    observed_bitcoin_network: String,
    source_status: String,
}

impl BtcAddressBalanceReadEvidence {
    /// Captures canonical evidence from a capability response.
    pub fn from_response(response: &BtcBalanceReadResponse) -> Self {
        Self {
            address: response.address.as_str().to_owned(),
            balance_sats: response.balance_sats,
            block_height: response.block_height,
            block_hash: response.block_hash.as_str().to_owned(),
            network: response.evidence.network_id.as_str().to_owned(),
            semantic_source_identity: response.evidence.source_identity.as_str().to_owned(),
            bitcoin_network: response.evidence.bitcoin_network.clone(),
            observed_bitcoin_network: response.evidence.observed_bitcoin_network.clone(),
            source_status: response.evidence.source_status.as_str().to_owned(),
        }
    }

    /// Reconstructs and checks the response against the state-authored plan.
    pub fn response(
        &self,
        plan: &BtcAddressBalanceReadPlan,
    ) -> Result<BtcBalanceReadResponse, BtcStateError> {
        let binding = plan.binding()?;
        let request = plan.request()?;
        if self.network != binding.network_id().as_str()
            || self.semantic_source_identity != binding.source_identity().as_str()
            || self.bitcoin_network != binding.bitcoin_network().as_str()
            || self.address != request.address().as_str()
            || self.block_height != request.block_height()
            || self.block_hash != request.block_hash().as_str()
        {
            return Err(BtcStateError::SourceMismatch);
        }
        let evidence = RedactedBtcSourceEvidence::from_binding(
            &binding,
            self.observed_bitcoin_network.clone(),
            parse_source_status(&self.source_status)?,
        )?;
        Ok(BtcBalanceReadResponse {
            evidence,
            address: BtcAddress::new(&self.address)?,
            balance_sats: self.balance_sats,
            block_height: self.block_height,
            block_hash: BtcBlockHash::new(&self.block_hash)?,
        })
    }
}

fn parse_head_kind(value: &str) -> Result<BtcHeadKind, BtcStateError> {
    match value {
        "best" => Ok(BtcHeadKind::Best),
        "confirmed" => Ok(BtcHeadKind::Confirmed),
        _ => invalid_evidence("unknown Bitcoin head kind"),
    }
}

fn parse_finality(
    value: &str,
    confirmation_depth: Option<u64>,
) -> Result<BtcFinality, BtcStateError> {
    match (value, confirmation_depth) {
        ("best_available", None) => Ok(BtcFinality::BestAvailable),
        ("confirmations", Some(confirmations)) => NonZeroU64::new(confirmations)
            .map(BtcFinality::Confirmations)
            .ok_or_else(|| BtcStateError::InvalidInput {
                reason: "Bitcoin evidence confirmation depth must be non-zero".to_owned(),
            }),
        _ => invalid_evidence("invalid Bitcoin finality evidence"),
    }
}

fn parse_source_status(value: &str) -> Result<BtcSourceStatus, BtcStateError> {
    match value {
        "synced" => Ok(BtcSourceStatus::Synced),
        "initial_block_download" => Ok(BtcSourceStatus::InitialBlockDownload),
        "unknown" => Ok(BtcSourceStatus::Unknown),
        _ => invalid_evidence("unknown Bitcoin source status"),
    }
}

fn invalid_evidence<T>(reason: &str) -> Result<T, BtcStateError> {
    Err(BtcStateError::InvalidInput {
        reason: reason.to_owned(),
    })
}
