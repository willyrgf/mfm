use std::num::NonZeroU64;

use mfm_canonical::CanonicalBytes;
use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use serde::de;
use serde::{Deserialize, Serialize};

use crate::{
    EvmAddress, EvmBlockAnchor, EvmDomainError, EvmTransactionRoute, MAX_EVM_CALLDATA_BYTES,
};

/// Exact anchored contract-call Read capability identity.
pub const EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID: &str =
    "mfm.evm.capability.read-anchored-contract-call@1";
/// Maximum returned bytes retained by an anchored call.
pub const MAX_EVM_CALL_RETURN_BYTES: usize = 131_072;

/// Exact intent of one contract call fixed to an authored block anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-intent",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-intent"
)]
pub struct AnchoredContractCallIntent {
    chain_id: NonZeroU64,
    route_ref: ContentRef,
    anchor: EvmBlockAnchor,
    target: EvmAddress,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    calldata: CanonicalBytes,
}

impl AnchoredContractCallIntent {
    /// Constructs one checked anchored-call intent from public call bytes.
    pub fn new(
        chain_id: NonZeroU64,
        route_ref: ContentRef,
        anchor: EvmBlockAnchor,
        target: EvmAddress,
        calldata: Vec<u8>,
    ) -> Result<Self, EvmDomainError> {
        let intent = Self {
            chain_id,
            route_ref,
            anchor,
            target,
            calldata: CanonicalBytes::new(calldata),
        };
        intent.validate().map(|_| intent)
    }

    /// Returns the exact public chain target.
    pub const fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }

    /// Returns the exact planned transaction-route identity.
    pub const fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }

    /// Returns the exact authored block anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    /// Returns the exact contract target.
    pub const fn target(&self) -> &EvmAddress {
        &self.target
    }

    /// Returns the bounded call data.
    pub fn calldata(&self) -> &[u8] {
        self.calldata.as_bytes()
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.anchor.validate()?;
        crate::transaction::validate_input_bytes(self.calldata.as_bytes(), MAX_EVM_CALLDATA_BYTES)
    }
}

impl<'de> Deserialize<'de> for AnchoredContractCallIntent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            chain_id: NonZeroU64,
            route_ref: ContentRef,
            anchor: EvmBlockAnchor,
            target: EvmAddress,
            calldata: CanonicalBytes,
        }

        let wire = Wire::deserialize(deserializer)?;
        let intent = Self {
            chain_id: wire.chain_id,
            route_ref: wire.route_ref,
            anchor: wire.anchor,
            target: wire.target,
            calldata: wire.calldata,
        };
        intent.validate().map(|_| intent).map_err(de::Error::custom)
    }
}

/// Exact anchor and bounded bytes returned by one contract call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-result",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-result"
)]
pub struct AnchoredContractCallResult {
    anchor: EvmBlockAnchor,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    return_bytes: CanonicalBytes,
}

impl AnchoredContractCallResult {
    /// Constructs an exact anchored result from bounded public bytes.
    pub fn new(anchor: EvmBlockAnchor, return_bytes: Vec<u8>) -> Result<Self, EvmDomainError> {
        if return_bytes.len() > MAX_EVM_CALL_RETURN_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            anchor,
            return_bytes: CanonicalBytes::new(return_bytes),
        })
    }

    /// Returns the exact block anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    /// Returns the bounded call return bytes.
    pub fn return_bytes(&self) -> &[u8] {
        self.return_bytes.as_bytes()
    }

    pub(crate) fn validate(&self) -> Result<(), EvmDomainError> {
        (self.return_bytes.as_bytes().len() <= MAX_EVM_CALL_RETURN_BYTES)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

impl<'de> Deserialize<'de> for AnchoredContractCallResult {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            anchor: EvmBlockAnchor,
            return_bytes: CanonicalBytes,
        }

        let wire = Wire::deserialize(deserializer)?;
        let value = Self {
            anchor: wire.anchor,
            return_bytes: wire.return_bytes,
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

/// Closed evidence for one exact anchored contract call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-evidence",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-evidence"
)]
pub enum AnchoredContractCallEvidence {
    /// Provider returned an exact result for the committed intent.
    #[non_exhaustive]
    Returned {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
        /// Authenticated anchored call result.
        result: AnchoredContractCallResult,
    },
    /// The anchored target had no deployed code.
    #[non_exhaustive]
    Rejected {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
    /// The named anchor was absent in an authenticated observation.
    #[non_exhaustive]
    SafeFailure {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
    /// Authenticated observations disagreed with the authored anchor.
    #[non_exhaustive]
    IntegrityBlocked {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
}

impl AnchoredContractCallEvidence {
    /// Constructs successful evidence bound to one exact intent value.
    pub fn returned(intent_value_ref: ContentRef, result: AnchoredContractCallResult) -> Self {
        Self::Returned {
            intent_value_ref,
            result,
        }
    }

    /// Constructs codeless-target evidence bound to one exact intent value.
    pub fn rejected(intent_value_ref: ContentRef) -> Self {
        Self::Rejected { intent_value_ref }
    }

    /// Constructs named-anchor absence evidence bound to one exact intent value.
    pub fn safe_failure(intent_value_ref: ContentRef) -> Self {
        Self::SafeFailure { intent_value_ref }
    }

    /// Constructs authenticated replacement evidence bound to one exact intent value.
    pub fn integrity_blocked(intent_value_ref: ContentRef) -> Self {
        Self::IntegrityBlocked { intent_value_ref }
    }

    /// Returns the exact Runtime-owned intent value reference.
    pub const fn intent_value_ref(&self) -> &ContentRef {
        match self {
            Self::Returned {
                intent_value_ref, ..
            }
            | Self::Rejected { intent_value_ref }
            | Self::SafeFailure { intent_value_ref }
            | Self::IntegrityBlocked { intent_value_ref } => intent_value_ref,
        }
    }

    fn validate_for(&self, intent: &AnchoredContractCallIntent) -> Result<(), EvmDomainError> {
        intent.validate()?;
        match self {
            Self::Returned { result, .. }
                if result.validate().is_ok() && result.anchor() == intent.anchor() =>
            {
                Ok(())
            }
            Self::Rejected { .. } | Self::SafeFailure { .. } | Self::IntegrityBlocked { .. } => {
                Ok(())
            }
            Self::Returned { .. } => Err(EvmDomainError::EvidenceBinding),
        }
    }
}

/// Closed failure reason of one anchored contract-call Read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-failure-reason",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-failure-reason"
)]
pub enum AnchoredContractCallFailureReason {
    /// The anchored target had no deployed code.
    Rejected,
    /// The named anchor was absent in an authenticated observation.
    SafeFailure,
    /// Authenticated observations disagreed with the authored anchor.
    IntegrityBlocked,
}

impl<'de> Deserialize<'de> for AnchoredContractCallFailureReason {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Rejected,
            SafeFailure,
            IntegrityBlocked,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: Kind,
        }

        Ok(match Wire::deserialize(deserializer)?.kind {
            Kind::Rejected => Self::Rejected,
            Kind::SafeFailure => Self::SafeFailure,
            Kind::IntegrityBlocked => Self::IntegrityBlocked,
        })
    }
}

/// Duplicate-safe anchored contract-call Read capability.
pub struct EvmAnchoredContractCallRead;

impl ReadCapabilityContract for EvmAnchoredContractCallRead {
    type Intent = AnchoredContractCallIntent;
    type Evidence = AnchoredContractCallEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new(EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID)
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (evidence.intent_value_ref() == intent_value_ref)
            .then_some(())
            .ok_or(EvmDomainError::EvidenceBinding)
            .and_then(|_| evidence.validate_for(intent))
            .map_err(|_| CapabilityError::EvidenceBinding)
    }
}

mod context;
pub use context::*;
