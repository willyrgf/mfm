#![warn(missing_docs)]
//! Reusable Bitcoin capability contracts.
//!
//! This crate defines state/adapter-facing Bitcoin read authority contracts. It owns semantic
//! source identities, chain-head and balance requests, response evidence, provider traits, and
//! redacted errors. Concrete protocol clients and runtime routing live outside this crate.
//!
//! ```rust
//! use mfm_btc_capabilities::{
//!     BtcChain, BtcChainGuard, BtcChainHeadReadCapability, BtcHeadSelection, BtcNetworkId,
//!     BtcSourceIdentity,
//! };
//! use mfm_capabilities::CapabilitySpec;
//!
//! let guard = BtcChainGuard::new(
//!     BtcChain::Bitcoin,
//!     BtcNetworkId::new("bitcoin-mainnet")?,
//!     BtcSourceIdentity::new("public-bitcoin-core")?,
//! );
//! assert_eq!(guard.network_id().as_str(), "bitcoin-mainnet");
//! assert_eq!(BtcChainHeadReadCapability::name(), "mfm.bitcoin.chain_head.read");
//! assert_eq!(BtcHeadSelection::best().head_kind(), mfm_btc_capabilities::BtcHeadKind::Best);
//! # Ok::<(), mfm_btc_capabilities::BtcCapabilityError>(())
//! ```

use std::fmt;
use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::str::FromStr;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ProviderDiagnosticCode, ProviderDiagnosticValue,
    ReadExternalRole, RedactedProviderDiagnostic,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm, LocalPublicId};

/// Result type for Bitcoin capability contracts.
pub type Result<T> = std::result::Result<T, BtcCapabilityError>;

/// Boxed future returned by Bitcoin capability providers.
pub type BtcCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Bitcoin chain-head read authority.
pub struct BtcChainHeadReadCapability;

impl CapabilitySpec for BtcChainHeadReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.bitcoin",
            "chain_head.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.capability:chain_head.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.bitcoin.chain_head.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.chain_head.read"
    }
}

/// Provider interface for Bitcoin chain-head reads.
pub trait BtcChainHeadReadProvider: Send + Sync {
    /// Reads a Bitcoin chain head from the selected source.
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse>;
}

/// Bitcoin address balance read authority.
pub struct BtcBalanceReadCapability;

impl CapabilitySpec for BtcBalanceReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.bitcoin",
            "balance.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.capability:balance.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.bitcoin.balance.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.balance.read"
    }
}

/// Provider interface for Bitcoin address balance reads.
pub trait BtcBalanceReadProvider: Send + Sync {
    /// Reads a Bitcoin address balance from the selected source.
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse>;
}

/// Bitcoin-like chain family observed by this capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BtcChain {
    /// Bitcoin.
    Bitcoin,
}

impl BtcChain {
    /// Returns the stable chain tag used in fact subjects and evidence.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bitcoin => "bitcoin",
        }
    }
}

impl fmt::Display for BtcChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! checked_btc_public_id {
    (
        $(#[$meta:meta])*
        $name:ident,
        $new_doc:literal,
        $as_str_doc:literal
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(LocalPublicId);

        impl $name {
            #[doc = $new_doc]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = LocalPublicId::new(value).map_err(invalid_identifier)?;
                Ok(Self(value))
            }

            #[doc = $as_str_doc]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = BtcCapabilityError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = BtcCapabilityError;

            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0.into_string()
            }
        }
    };
}

checked_btc_public_id!(
    /// Semantic Bitcoin network id from authored workflow config.
    BtcNetworkId,
    "Creates a checked semantic Bitcoin network id.",
    "Returns the checked network id string."
);

checked_btc_public_id!(
    /// Non-secret semantic Bitcoin source identity.
    ///
    /// This identifies what was observed when source identity is part of the claim semantics. It is
    /// not a process-local route, URL, credential label, or deployment handle.
    BtcSourceIdentity,
    "Creates a checked semantic source identity.",
    "Returns the checked source identity string."
);

/// Semantic Bitcoin chain guard derived from workflow config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcChainGuard {
    chain: BtcChain,
    network_id: BtcNetworkId,
    source_identity: BtcSourceIdentity,
}

impl BtcChainGuard {
    /// Creates a semantic chain guard.
    pub fn new(
        chain: BtcChain,
        network_id: BtcNetworkId,
        source_identity: BtcSourceIdentity,
    ) -> Self {
        Self {
            chain,
            network_id,
            source_identity,
        }
    }

    /// Returns the chain family.
    pub const fn chain(&self) -> BtcChain {
        self.chain
    }

    /// Returns the semantic network id.
    pub const fn network_id(&self) -> &BtcNetworkId {
        &self.network_id
    }

    /// Returns the semantic source identity.
    pub const fn source_identity(&self) -> &BtcSourceIdentity {
        &self.source_identity
    }
}

/// Chain-head semantic kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BtcHeadKind {
    /// Best head known to the selected source.
    Best,
    /// Head behind the best tip by an explicit confirmation depth.
    Confirmed,
}

impl BtcHeadKind {
    /// Returns the stable diagnostic tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Confirmed => "confirmed",
        }
    }
}

/// Finality policy attached to a chain-head read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BtcFinality {
    /// No confirmation-depth finality claim; read the best available source head.
    BestAvailable,
    /// Read a head with at least the given confirmation depth relative to the source tip.
    Confirmations(NonZeroU64),
}

impl BtcFinality {
    /// Creates a confirmation-depth finality policy.
    pub fn confirmations(confirmations: u64) -> Result<Self> {
        let confirmations =
            NonZeroU64::new(confirmations).ok_or(BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::ZeroConfirmations,
            })?;
        Ok(Self::Confirmations(confirmations))
    }

    /// Returns the confirmation depth when this policy carries one.
    pub const fn confirmation_depth(self) -> Option<u64> {
        match self {
            Self::BestAvailable => None,
            Self::Confirmations(confirmations) => Some(confirmations.get()),
        }
    }

    /// Returns the stable diagnostic tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BestAvailable => "best_available",
            Self::Confirmations(_) => "confirmations",
        }
    }
}

/// Chain-head selector requested from a Bitcoin source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BtcHeadSelection {
    head_kind: BtcHeadKind,
    finality: BtcFinality,
}

impl BtcHeadSelection {
    /// Selects the best head known to the source.
    pub const fn best() -> Self {
        Self {
            head_kind: BtcHeadKind::Best,
            finality: BtcFinality::BestAvailable,
        }
    }

    /// Selects a confirmation-depth head relative to the source best tip.
    pub fn confirmed(confirmations: u64) -> Result<Self> {
        Ok(Self {
            head_kind: BtcHeadKind::Confirmed,
            finality: BtcFinality::confirmations(confirmations)?,
        })
    }

    /// Returns the selected head kind.
    pub const fn head_kind(self) -> BtcHeadKind {
        self.head_kind
    }

    /// Returns the selected finality policy.
    pub const fn finality(self) -> BtcFinality {
        self.finality
    }
}

/// Request for a Bitcoin chain-head read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcChainHeadRequest {
    /// Semantic chain guard.
    pub guard: BtcChainGuard,
    /// Requested head selection.
    pub selection: BtcHeadSelection,
}

/// Request for a Bitcoin address balance read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcBalanceReadRequest {
    /// Semantic chain guard.
    pub guard: BtcChainGuard,
    /// Public Bitcoin address to observe.
    pub address: BtcAddress,
    /// Requested UTXO-set head selection.
    pub selection: BtcHeadSelection,
}

/// Redacted Bitcoin source evidence attached to provider responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedBtcSourceEvidence {
    /// Chain family from the request guard.
    pub chain: BtcChain,
    /// Semantic network id from the request guard.
    pub network_id: BtcNetworkId,
    /// Semantic source identity observed by the provider.
    pub source_identity: BtcSourceIdentity,
    /// Observed source network tag, when the provider can report one.
    pub observed_network: Option<String>,
    /// Head kind represented by this evidence.
    pub head_kind: BtcHeadKind,
    /// Finality policy represented by this evidence.
    pub finality: BtcFinality,
    /// Source synchronization status represented by this evidence.
    pub source_status: BtcSourceStatus,
}

impl RedactedBtcSourceEvidence {
    /// Builds evidence from a request and observed source status.
    pub fn from_request(
        request: &BtcChainHeadRequest,
        observed_network: Option<String>,
        source_status: BtcSourceStatus,
    ) -> Self {
        Self {
            chain: request.guard.chain(),
            network_id: request.guard.network_id().clone(),
            source_identity: request.guard.source_identity().clone(),
            observed_network,
            head_kind: request.selection.head_kind(),
            finality: request.selection.finality(),
            source_status,
        }
    }

    /// Verifies that provider evidence matches the semantic request guard and head selection.
    pub fn verify_request(&self, request: &BtcChainHeadRequest) -> Result<()> {
        if self.chain == request.guard.chain()
            && &self.network_id == request.guard.network_id()
            && &self.source_identity == request.guard.source_identity()
            && self.head_kind == request.selection.head_kind()
            && self.finality == request.selection.finality()
        {
            Ok(())
        } else {
            Err(BtcCapabilityError::SourceMismatch {
                diagnostic: self.source_mismatch_diagnostic(),
            })
        }
    }

    /// Returns closed redacted source-mismatch diagnostic details.
    pub fn source_mismatch_diagnostic(&self) -> RedactedProviderDiagnostic {
        let mut diagnostic = btc_diagnostic(ProviderDiagnosticCode::SourceMismatch)
            .with_field(
                btc_public_id("chain"),
                ProviderDiagnosticValue::Id(btc_public_id(self.chain.as_str())),
            )
            .with_field(
                btc_public_id("network_id"),
                ProviderDiagnosticValue::Id(btc_public_id(self.network_id.as_str())),
            )
            .with_field(
                btc_public_id("source_identity"),
                ProviderDiagnosticValue::Id(btc_public_id(self.source_identity.as_str())),
            )
            .with_field(
                btc_public_id("head_kind"),
                ProviderDiagnosticValue::Id(btc_public_id(self.head_kind.as_str())),
            )
            .with_field(
                btc_public_id("finality"),
                ProviderDiagnosticValue::Id(btc_public_id(self.finality.as_str())),
            )
            .with_field(
                btc_public_id("source_status"),
                ProviderDiagnosticValue::Id(btc_public_id(self.source_status.as_str())),
            );
        if let Some(confirmations) = self.finality.confirmation_depth() {
            diagnostic = diagnostic.with_field(
                btc_public_id("confirmations"),
                ProviderDiagnosticValue::U64(confirmations),
            );
        }
        diagnostic
    }
}

/// Source synchronization status represented by a chain-head read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BtcSourceStatus {
    /// Source reports an in-sync usable view.
    Synced,
    /// Source reports it is still catching up.
    InitialBlockDownload,
    /// Source status could not be classified by the provider.
    Unknown,
}

impl BtcSourceStatus {
    /// Returns the stable diagnostic tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::InitialBlockDownload => "initial_block_download",
            Self::Unknown => "unknown",
        }
    }
}

/// Canonical Bitcoin block hash string.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BtcBlockHash(String);

impl BtcBlockHash {
    /// Creates a checked lowercase hex block hash.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::InvalidBlockHash,
            });
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Returns the canonical lowercase hex block hash.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BtcBlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BtcBlockHash").field(&self.0).finish()
    }
}

impl fmt::Display for BtcBlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BtcBlockHash {
    type Err = BtcCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for BtcBlockHash {
    type Error = BtcCapabilityError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<BtcBlockHash> for String {
    fn from(value: BtcBlockHash) -> Self {
        value.0
    }
}

/// Canonical public Bitcoin address string.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BtcAddress(String);

impl BtcAddress {
    /// Creates a checked public Bitcoin address.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        if !is_supported_bitcoin_address_envelope(value) {
            return Err(BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::InvalidAddress,
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the canonical address string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BtcAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BtcAddress").field(&self.0).finish()
    }
}

impl fmt::Display for BtcAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BtcAddress {
    type Err = BtcCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for BtcAddress {
    type Error = BtcCapabilityError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<BtcAddress> for String {
    fn from(value: BtcAddress) -> Self {
        value.0
    }
}

/// Response for a Bitcoin chain-head read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcChainHeadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedBtcSourceEvidence,
    /// Observed block height.
    pub block_height: u64,
    /// Observed block hash.
    pub block_hash: BtcBlockHash,
    /// Provider-reported block time as Unix milliseconds, when available.
    pub provider_time_unix_ms: Option<u64>,
}

impl BtcChainHeadResponse {
    /// Verifies that this response matches the request guard and head selection.
    pub fn verify_request(&self, request: &BtcChainHeadRequest) -> Result<()> {
        self.evidence.verify_request(request)
    }
}

/// Response for a Bitcoin address balance read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcBalanceReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedBtcSourceEvidence,
    /// Public Bitcoin address that was observed.
    pub address: BtcAddress,
    /// Total confirmed UTXO amount in satoshis.
    pub balance_sats: u64,
    /// UTXO set height used by the provider.
    pub block_height: u64,
    /// UTXO set block hash used by the provider.
    pub block_hash: BtcBlockHash,
}

impl BtcBalanceReadResponse {
    /// Verifies that this response matches the request guard, address, and head selection.
    pub fn verify_request(&self, request: &BtcBalanceReadRequest) -> Result<()> {
        self.evidence.verify_request(&BtcChainHeadRequest {
            guard: request.guard.clone(),
            selection: request.selection,
        })?;
        if self.address == request.address {
            Ok(())
        } else {
            Err(BtcCapabilityError::SourceMismatch {
                diagnostic: self.evidence.source_mismatch_diagnostic(),
            })
        }
    }
}

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtcInvalidRequest {
    /// Identifier was invalid.
    InvalidIdentifier,
    /// Confirmation depth was zero.
    ZeroConfirmations,
    /// Block hash was not 32 bytes of hex.
    InvalidBlockHash,
    /// Address was not in a supported public Bitcoin address envelope.
    InvalidAddress,
}

/// Redaction-safe Bitcoin capability error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcCapabilityError {
    /// Request failed contract validation.
    #[error("Bitcoin capability request was invalid")]
    InvalidRequest {
        /// Closed invalid-request reason.
        reason: BtcInvalidRequest,
    },
    /// Provider failed without exposing concrete source details.
    #[error("Bitcoin capability provider failed: {diagnostic}")]
    Provider {
        /// Closed redacted provider diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// Provider evidence did not match the semantic request.
    #[error("Bitcoin source evidence did not match request: {diagnostic}")]
    SourceMismatch {
        /// Closed redacted source-mismatch diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
}

impl BtcCapabilityError {
    /// Builds a provider failure from a closed redacted diagnostic.
    pub fn provider_failure(diagnostic: RedactedProviderDiagnostic) -> Self {
        Self::Provider { diagnostic }
    }

    /// Returns the closed redacted provider diagnostic carried by this error.
    pub const fn redacted_diagnostic(&self) -> Option<&RedactedProviderDiagnostic> {
        match self {
            Self::Provider { diagnostic } | Self::SourceMismatch { diagnostic } => Some(diagnostic),
            Self::InvalidRequest { .. } => None,
        }
    }
}

/// Builds a closed redacted Bitcoin provider diagnostic.
pub fn btc_diagnostic(code: ProviderDiagnosticCode) -> RedactedProviderDiagnostic {
    RedactedProviderDiagnostic::new(btc_public_id("bitcoin"), code)
}

fn btc_public_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("Bitcoin diagnostic id must be checked public text")
}

fn invalid_identifier(_source: mfm_ids::CheckedStringError) -> BtcCapabilityError {
    BtcCapabilityError::InvalidRequest {
        reason: BtcInvalidRequest::InvalidIdentifier,
    }
}

fn is_supported_bitcoin_address_envelope(value: &str) -> bool {
    if value.trim() != value || value.len() < 14 || value.len() > 90 || !value.is_ascii() {
        return false;
    }
    if !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return false;
    }
    if value.starts_with("bc1") || value.starts_with("tb1") || value.starts_with("bcrt1") {
        return value == value.to_ascii_lowercase();
    }
    matches!(
        value.as_bytes().first().copied(),
        Some(b'1' | b'3' | b'2' | b'm' | b'n')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> BtcChainGuard {
        BtcChainGuard::new(
            BtcChain::Bitcoin,
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        )
    }

    fn request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
        BtcChainHeadRequest {
            guard: guard(),
            selection,
        }
    }

    #[test]
    fn confirmation_depth_rejects_zero() {
        let error = BtcHeadSelection::confirmed(0).expect_err("zero confirmations");

        assert_eq!(
            error,
            BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::ZeroConfirmations,
            }
        );
    }

    #[test]
    fn block_hash_validation_rejects_non_hash_values() {
        let error = BtcBlockHash::new("not-a-block-hash").expect_err("invalid hash");

        assert_eq!(
            error,
            BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::InvalidBlockHash,
            }
        );
    }

    #[test]
    fn block_hash_validation_normalizes_to_lowercase() {
        let hash =
            BtcBlockHash::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("hash");

        assert_eq!(
            hash.as_str(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn response_evidence_verifies_request() {
        let request = request(BtcHeadSelection::best());
        let evidence = RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_string()),
            BtcSourceStatus::Synced,
        );

        evidence
            .verify_request(&request)
            .expect("matching evidence");
    }

    #[test]
    fn response_evidence_rejects_mismatch_without_concrete_source_details() {
        let request = request(BtcHeadSelection::best());
        let mismatched = RedactedBtcSourceEvidence {
            source_identity: BtcSourceIdentity::new("different-semantic-source").expect("source"),
            ..RedactedBtcSourceEvidence::from_request(&request, None, BtcSourceStatus::Unknown)
        };

        let error = mismatched
            .verify_request(&request)
            .expect_err("mismatch should fail");
        let rendered = format!("{error:?} {error}");

        assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
        assert!(rendered.contains("different-semantic-source"));
        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains(concat!("Bear", "er")));
        assert!(!rendered.contains("secret"));
    }

    #[test]
    fn provider_failure_carries_only_closed_diagnostics() {
        let diagnostic = btc_diagnostic(ProviderDiagnosticCode::RpcHttpStatus)
            .with_operation(btc_public_id("scantxoutset"))
            .with_field(
                btc_public_id("http_status"),
                ProviderDiagnosticValue::U64(403),
            );
        let error = BtcCapabilityError::provider_failure(diagnostic.clone());
        let rendered = format!("{error:?} {error}");

        assert_eq!(error, BtcCapabilityError::Provider { diagnostic });
        assert!(!rendered.contains("node.invalid"));
        assert!(!rendered.contains(concat!("Author", "ization")));
        assert!(!rendered.contains("secret"));
    }
}
