//! Shared balance requests and checked collection semantics, independent of native protocols.

use mfm_program_derive::MfmValue;
use mfm_values::{string_contains_secret_marker, SizeLimitExceeded};
use serde::{Deserialize, Serialize};

use crate::BalanceTarget;

mod planning;
pub use planning::{BalanceExecutionConfig, BalanceSourceDefinition};
mod read;
pub use read::{BalanceEvidence, BalanceOutcome, BalanceRead, ReadBalanceAt, ReadBalanceAtError};
mod arithmetic;
pub use arithmetic::{BalanceArithmetic, BalanceCollectionFailure};
mod metadata;
pub use metadata::BalanceCollectionMetadata;
mod context;
pub use context::{
    BalanceContext, BalanceContextError, CandidateBalance, ConfirmedBalance, PreparedBalance,
};
mod observe;
pub use observe::{ObserveBalance, ObserveBalanceFailure};
mod completion;
pub use completion::{BalanceCollectionCompletion, ConsolidateBalanceCollection};

/// Rejected decimal scale, retaining the actual supplied scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue, thiserror::Error)]
#[error("balance decimal scale {actual} exceeds 30")]
pub struct DecimalScaleError {
    actual: u8,
}
impl<'de> Deserialize<'de> for DecimalScaleError {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            actual: u8,
        }
        let wire = Wire::deserialize(deserializer)?;
        DecimalScale::new(wire.actual)
            .err()
            .ok_or_else(|| serde::de::Error::custom("rejected decimal scale must exceed 30"))
    }
}

/// Supported decimal scale, from zero through thirty inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.chain",
    name = "decimal-scale",
    version = "1",
    schema = "mfm.chain-decimal-scale"
)]
pub struct DecimalScale(u8);

impl DecimalScale {
    /// Checks the supported decimal range.
    pub fn new(value: u8) -> Result<Self, DecimalScaleError> {
        if value > 30 {
            return Err(DecimalScaleError { actual: value });
        }
        Ok(Self(value))
    }

    /// Number of decimal places.
    pub fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for DecimalScale {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u8::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Rejected public balance identifier or correlation; rejected text is never retained.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum BalanceTextError {
    /// Public identifiers must contain text.
    #[error("balance public text is empty")]
    Empty,
    /// Public identifiers have an inclusive 256-byte bound.
    #[error("balance public text size: {0}")]
    Size(#[from] SizeLimitExceeded),
    /// Control characters and secret markers are forbidden.
    #[error("balance public text violates its character rules; input withheld")]
    InvalidText,
}

/// One declared balance source with a checked public identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "balance-source",
    version = "1",
    schema = "mfm.chain-balance-source"
)]
pub struct BalanceSource {
    source_id: String,
    target: BalanceTarget,
}

fn check_public_text(value: &str) -> Result<(), BalanceTextError> {
    if value.is_empty() {
        return Err(BalanceTextError::Empty);
    }
    SizeLimitExceeded::check(value.len() as u64, 256)?;
    if value.chars().any(char::is_control) || string_contains_secret_marker(value) {
        return Err(BalanceTextError::InvalidText);
    }
    Ok(())
}

impl BalanceSource {
    /// Checks the source identity; native target qualification remains with its owner.
    pub fn new(source_id: String, target: BalanceTarget) -> Result<Self, BalanceTextError> {
        check_public_text(&source_id)?;
        Ok(Self { source_id, target })
    }

    /// Stable public source identity in declaration order.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Exact ledger and native target envelope.
    pub fn target(&self) -> &BalanceTarget {
        &self.target
    }
}

impl<'de> Deserialize<'de> for BalanceSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source_id: String,
            target: BalanceTarget,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.source_id, wire.target).map_err(serde::de::Error::custom)
    }
}

/// Rejected collection admission, with indices rather than copied source identifiers.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum BalanceRequestError {
    /// A collection must declare at least one source.
    #[error("balance collection has no sources")]
    Empty,
    /// A collection exceeds the fixed source count.
    #[error("balance collection source count: {0}")]
    Size(#[from] SizeLimitExceeded),
    /// A later source reuses a previously declared identity.
    #[error("balance source {duplicate} duplicates source {first}")]
    Duplicate {
        /// Index of the first declaration.
        first: usize,
        /// Index of the repeated declaration.
        duplicate: usize,
    },
    /// A source belongs to a different ledger than the first declaration.
    #[error("balance source {index} has a different ledger")]
    Ledger {
        /// Index of the mismatching source.
        index: usize,
    },
}

/// Nonempty, ordered collection of at most 64 unique sources on one ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "balance-request",
    version = "1",
    schema = "mfm.chain-balance-request"
)]
pub struct BalanceRequest {
    sources: Vec<BalanceSource>,
    decimals: DecimalScale,
}

impl BalanceRequest {
    /// Checks count, unique identities and ledger agreement without reordering sources.
    pub fn new(
        sources: Vec<BalanceSource>,
        decimals: DecimalScale,
    ) -> Result<Self, BalanceRequestError> {
        let Some(first) = sources.first() else {
            return Err(BalanceRequestError::Empty);
        };
        SizeLimitExceeded::check(sources.len() as u64, 64)?;
        for (index, source) in sources.iter().enumerate() {
            if source.target().ledger() != first.target().ledger() {
                return Err(BalanceRequestError::Ledger { index });
            }
            if let Some(first) = sources[..index]
                .iter()
                .position(|previous| previous.source_id() == source.source_id())
            {
                return Err(BalanceRequestError::Duplicate {
                    first,
                    duplicate: index,
                });
            }
        }
        Ok(Self { sources, decimals })
    }

    /// Exact declaration order, retained through collection execution.
    pub fn sources(&self) -> &[BalanceSource] {
        &self.sources
    }

    /// Target decimal scale of collected amounts.
    pub fn decimals(&self) -> DecimalScale {
        self.decimals
    }
}

impl<'de> Deserialize<'de> for BalanceRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            sources: Vec<BalanceSource>,
            decimals: DecimalScale,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.sources, wire.decimals).map_err(serde::de::Error::custom)
    }
}

/// Closed public balance failure projection, independent of native wire or recovery classification.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    mfm_program_derive::MfmValue,
)]
#[serde(rename_all = "snake_case")]
pub enum BalanceFailureCode {
    /// Authenticated collection anchors changed.
    AnchorChanged,
    /// Chain identity could not be established from the accepted observation.
    ChainIdentityUnavailable,
    /// The accepted observation or confirmed amount could not complete the collection.
    ObservationUnavailable,
    /// Authenticated external evidence blocked integrity.
    IntegrityBlocked,
}
impl BalanceFailureCode {
    /// Stable public spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnchorChanged => "anchor_changed",
            Self::ChainIdentityUnavailable => "chain_identity_unavailable",
            Self::ObservationUnavailable => "observation_unavailable",
            Self::IntegrityBlocked => "integrity_blocked",
        }
    }
}
