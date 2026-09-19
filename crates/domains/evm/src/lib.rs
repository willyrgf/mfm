#![warn(missing_docs)]
//! Secret-free EVM State values and capability contracts.
//!
//! The domain owns deterministic balance Reads, fixed EIP-1559 transaction Effects, and anchored
//! contract-call Reads. Provider clients, signing handles, nonce authority, and response ingress
//! belong downstream; no domain value contains a client, credential, raw response, or generic
//! context map.

use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::string_contains_secret_marker;
use serde::de;
use serde::{Deserialize, Serialize};

macro_rules! impl_checked_deserialize {
    ($type:ident { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    $($field: $field_type,)+
                }

                let wire = Wire::deserialize(deserializer)?;
                let value = Self {
                    $($field: wire.$field,)+
                };
                value.validate().map(|_| value).map_err(de::Error::custom)
            }
        }
    };
}

mod provider_failure;
pub use provider_failure::*;
mod recovery;
pub use recovery::EvmTransactionOperationalError;
mod anchored_call;
mod balance;
pub use balance::{
    project_balance_context, CheckEvmBalanceChain, ConfirmEvmBalanceAnchor, EvmBalanceBinding,
    EvmBalanceFailure, EvmBalanceLedger, EvmBalanceObservation, EvmBalanceRoute, EvmBalanceTarget,
    EvmChainChecked, EvmNativeBalance, EvmObservationRejection, EvmTokenAnchored, EvmTokenBalance,
    NativeBalanceStage, ReadEvmTokenDecimals, ReadInitialEvmBalanceAnchor,
};
pub mod custody;
mod transaction;
pub use anchored_call::{
    AnchoredContractCallEvidence, AnchoredContractCallFailureReason, AnchoredContractCallIntent,
    AnchoredContractCallResult, MAX_EVM_CALL_RETURN_BYTES,
};
pub use transaction::{
    Eip1559Options, Eip1559TransactionCommand, EvmAddress, EvmAuthorityEpoch, EvmBlockPoint,
    EvmChainInstance, EvmContractExecutionConfig, EvmContractReadImplementation, EvmHash,
    EvmNonceReservationEffect, EvmNonceReservationImplementation, EvmScalarContractArtifact,
    EvmTransactionBinding, EvmTransactionImplementation, EvmTransactionOutcome,
    EvmTransactionPreparationEffect, EvmTransactionPreparationImplementation,
    EvmTransactionReceipt, EvmTransactionRecipe, EvmTransactionRoute, EvmTransactionSettlement,
    EvmU256, NonceDomain, PrepareEvmTransaction, PreparedEvmTransaction,
    PreparedEvmTransactionEvidence, Reservation, ReservationBindingError, ReserveEvmNonce,
    ReservedEvmTransaction, ReservedRequest, ScalarArtifactError, MAX_EVM_CALLDATA_BYTES,
    MAX_EVM_INITCODE_BYTES,
};

/// Secret-free public identity of one live EVM route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "physical-target",
    version = "1",
    schema = "mfm.evm-physical-target"
)]
pub struct EvmPhysicalTarget {
    /// Chain id.
    pub chain_id: NonZeroU64,
    /// Endpoint ref.
    pub endpoint_ref: ContentRef,
}

impl EvmPhysicalTarget {
    /// Derives the exact canonical adapter binding identity.
    pub fn binding_ref(&self) -> Result<ContentRef, EvmDomainError> {
        mfm_values::canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }
}

/// Secret-free named identity of one physical EVM endpoint.
///
/// The name alone identifies the route: an RPC URL, credential, or client handle is never
/// endpoint material, so replacing one under the same name leaves the derived reference,
/// the `EvmPhysicalTarget`, and the planned Program byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "endpoint",
    version = "1",
    schema = "mfm.evm-endpoint"
)]
pub struct EvmEndpoint {
    endpoint_id: String,
}

impl_checked_deserialize!(EvmEndpoint {
    endpoint_id: String,
});

impl EvmEndpoint {
    /// Constructs one checked public endpoint identity.
    pub fn new(endpoint_id: impl Into<String>) -> Result<Self, EvmDomainError> {
        let endpoint = Self {
            endpoint_id: endpoint_id.into(),
        };
        endpoint.validate().map(|_| endpoint)
    }

    /// Returns the checked public endpoint name.
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    /// Derives the exact canonical endpoint reference one `EvmPhysicalTarget` binds.
    pub fn endpoint_ref(&self) -> Result<ContentRef, EvmDomainError> {
        mfm_values::canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        valid_public_text(&self.endpoint_id, 256)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

/// Committed public block anchor of one bounded EVM observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBlockAnchor {
    /// Hash.
    pub hash: EvmHash,
    /// Number.
    pub number: EvmU256,
}

/// Redaction-safe EVM domain construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "domain-error",
    version = "3",
    schema = "mfm.evm-domain-error"
)]
pub enum EvmDomainError {
    /// Shared decimal-scale construction failed, retaining the actual rejected scale.
    #[error("EVM balance scale is invalid: {0}")]
    BalanceScale(#[from] mfm_chain::balance::DecimalScaleError),
    /// Shared scalar construction failed, retaining the exact checked cause.
    #[error("EVM unsigned scalar is invalid: {0}")]
    Unsigned256(#[from] mfm_values::Unsigned256Error),
    /// A bounded public value is invalid.
    #[error("EVM domain value is invalid")]
    InvalidValue,
    /// A capability intent/evidence pair is not bound.
    #[error("EVM capability evidence is not bound")]
    EvidenceBinding,
    /// Program authoring or binding identity is invalid.
    #[error("EVM Program contract is invalid")]
    Program,
}

/// Exact native operation; source qualification is retained once in its enclosing intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.derived",
    name = "evm_read_subject",
    version = "2",
    schema = "mfm.derived.evm_read_subject"
)]
pub enum EvmReadSubject {
    /// Observe the endpoint's chain identity.
    ChainIdentity,
    /// Observe the latest anchor before reading one source.
    InitialAnchor,
    /// Read native currency at the committed anchor.
    NativeBalance {
        /// Exact committed block number and hash.
        anchor: EvmBlockAnchor,
    },
    /// Read token decimals at the committed anchor.
    TokenDecimals {
        /// Exact committed block number and hash.
        anchor: EvmBlockAnchor,
    },
    /// Read token units at the committed anchor.
    TokenBalance {
        /// Exact committed block number and hash.
        anchor: EvmBlockAnchor,
    },
    /// Re-read the committed anchor by number, never latest.
    ConfirmAnchor {
        /// Exact committed block number and hash.
        anchor: EvmBlockAnchor,
    },
}

/// Fully qualified native balance intent, shared by supporting semantic and native protocols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.derived",
    name = "evm_read_intent",
    version = "2",
    schema = "mfm.derived.evm_read_intent"
)]
pub struct EvmReadIntent {
    source_ordinal: u32,
    collection_scale: mfm_chain::balance::DecimalScale,
    chain_id: NonZeroU64,
    target: EvmBalanceTarget,
    route_ref: ContentRef,
    subject: EvmReadSubject,
}
impl_checked_deserialize!(EvmReadIntent {
    source_ordinal: u32,
    collection_scale: mfm_chain::balance::DecimalScale,
    chain_id: NonZeroU64,
    target: EvmBalanceTarget,
    route_ref: ContentRef,
    subject: EvmReadSubject,
});
impl EvmReadIntent {
    /// Checks subject/asset agreement; selected implementation admission checks binding agreement.
    pub fn new(
        source_ordinal: u32,
        collection_scale: mfm_chain::balance::DecimalScale,
        chain_id: NonZeroU64,
        target: EvmBalanceTarget,
        route_ref: ContentRef,
        subject: EvmReadSubject,
    ) -> Result<Self, EvmDomainError> {
        let intent = Self {
            source_ordinal,
            collection_scale,
            chain_id,
            target,
            route_ref,
            subject,
        };
        intent.validate()?;
        Ok(intent)
    }
    /// Active source position supplied by retained execution input.
    pub fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
    /// Collection scale supplied by retained execution input.
    pub fn collection_scale(&self) -> mfm_chain::balance::DecimalScale {
        self.collection_scale
    }
    /// Exact nonzero chain target.
    pub fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }
    /// Exact account and asset, without duplicating shared source identity.
    pub fn target(&self) -> &EvmBalanceTarget {
        &self.target
    }
    /// Exact public physical route reference.
    pub fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Requested native operation and any committed anchor.
    pub fn subject(&self) -> &EvmReadSubject {
        &self.subject
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        match self.subject {
            EvmReadSubject::NativeBalance { .. } if self.target.token().is_some() => {
                Err(EvmDomainError::InvalidValue)
            }
            EvmReadSubject::TokenDecimals { .. } | EvmReadSubject::TokenBalance { .. }
                if self.target.token().is_none() =>
            {
                Err(EvmDomainError::InvalidValue)
            }
            _ => Ok(()),
        }
    }
}

/// Supported ERC-20 decimal scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "token-decimals",
    version = "1",
    schema = "mfm.evm-token-decimals"
)]
pub struct EvmTokenDecimals(u8);

impl EvmTokenDecimals {
    /// Constructs a decimal scale supported by deterministic balance scaling.
    pub fn new(value: u8) -> Result<Self, EvmDomainError> {
        (value <= 30)
            .then_some(Self(value))
            .ok_or(EvmDomainError::InvalidValue)
    }

    /// Returns the checked decimal scale.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for EvmTokenDecimals {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Typed provider values admitted after raw EVM ingress is discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.derived",
    name = "evm_read_value",
    version = "1",
    schema = "mfm.derived.evm_read_value"
)]
pub enum EvmReadValue {
    /// Authenticated chain id.
    ChainId(NonZeroU64),
    /// Authenticated block anchor components.
    Anchor(EvmBlockAnchor),
    /// Canonical unsigned raw units.
    RawUnits(EvmU256),
    /// Token decimal scale.
    TokenDecimals(EvmTokenDecimals),
}

/// Closed EVM read evidence sum; raw provider material is discarded before construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.derived",
    name = "evm_read_evidence",
    version = "1",
    schema = "mfm.derived.evm_read_evidence"
)]
pub enum EvmReadEvidence {
    /// Provider returned an exact structured value for the committed intent.
    #[non_exhaustive]
    Returned {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
        /// Interpreted provider value.
        value: EvmReadValue,
    },
    /// Provider returned a reviewed rejection.
    #[non_exhaustive]
    Rejected {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
    /// Adapter accepted a safe failure.
    #[non_exhaustive]
    SafeFailure {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
    /// Authentication/integrity failed and grants no retry authority.
    #[non_exhaustive]
    IntegrityBlocked {
        /// Exact Runtime-owned canonical intent value reference.
        intent_value_ref: ContentRef,
    },
}

impl EvmReadEvidence {
    /// Constructs successful evidence bound to one exact intent value.
    pub fn returned(intent_value_ref: ContentRef, value: EvmReadValue) -> Self {
        Self::Returned {
            intent_value_ref,
            value,
        }
    }

    /// Constructs reviewed rejection evidence bound to one exact intent value.
    pub fn rejected(intent_value_ref: ContentRef) -> Self {
        Self::Rejected { intent_value_ref }
    }

    /// Constructs reviewed safe-failure evidence bound to one exact intent value.
    pub fn safe_failure(intent_value_ref: ContentRef) -> Self {
        Self::SafeFailure { intent_value_ref }
    }

    /// Constructs authenticated integrity-block evidence bound to one exact intent value.
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

    fn validate_for(&self, intent: &EvmReadIntent) -> Result<(), EvmDomainError> {
        let valid = match self {
            Self::Returned { value, .. } => read_value_valid_for_intent(intent, value),
            Self::Rejected { .. } | Self::SafeFailure { .. } | Self::IntegrityBlocked { .. } => {
                true
            }
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

/// Groups the subject variants one Read capability admits.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadCapabilityFamily {
    /// Chain identity observations.
    ChainIdentity,
    /// Initial anchor observes the head; confirmation re-observes the block its intent names.
    Anchor,
    /// Native and token balance observations.
    Balance,
}

impl ReadCapabilityFamily {
    /// Returns whether this capability admits the checked intent's subject.
    pub fn accepts(self, subject: &EvmReadSubject) -> bool {
        match self {
            ReadCapabilityFamily::ChainIdentity => {
                matches!(subject, EvmReadSubject::ChainIdentity)
            }
            ReadCapabilityFamily::Anchor => matches!(
                subject,
                EvmReadSubject::InitialAnchor | EvmReadSubject::ConfirmAnchor { .. }
            ),
            ReadCapabilityFamily::Balance => matches!(
                subject,
                EvmReadSubject::NativeBalance { .. }
                    | EvmReadSubject::TokenDecimals { .. }
                    | EvmReadSubject::TokenBalance { .. }
            ),
        }
    }
}

/// Observational EVM Read capability for the target chain identity.
pub struct EvmChainIdentityRead;

/// Observational EVM Read capability for chain anchors.
pub struct EvmAnchorRead;

/// Observational EVM Read capability for native and token balances.
pub struct EvmBalanceRead;

macro_rules! impl_read_capability {
    ($ty:ty, $name:literal, $family:ident) => {
        impl ReadCapabilityContract for $ty {
            type Intent = EvmReadIntent;
            type Evidence = EvmReadEvidence;
            fn contract_id() -> mfm_capabilities::Result<StableId> {
                Ok(StableId::new($name)?)
            }

            fn bind_evidence(
                intent_value_ref: &ContentRef,
                intent: &Self::Intent,
                _: &ContentRef,
                evidence: &Self::Evidence,
            ) -> Result<(), mfm_values::InvocationDiagnostic> {
                (evidence.intent_value_ref() == intent_value_ref)
                    .then_some(())
                    .ok_or(EvmDomainError::EvidenceBinding)
                    .and_then(|_| {
                        ReadCapabilityFamily::$family
                            .accepts(intent.subject())
                            .then_some(())
                            .ok_or(EvmDomainError::EvidenceBinding)
                    })
                    .and_then(|_| evidence.validate_for(intent))
                    .map_err(|error| {
                        mfm_values::InvocationDiagnostic::from_fields(
                            "state_internal",
                            "bind_evidence",
                            &error,
                            None,
                        )
                    })
            }
        }
    };
}

impl_read_capability!(
    EvmChainIdentityRead,
    "mfm.evm.capability.read-chain-identity@1",
    ChainIdentity
);
impl_read_capability!(EvmAnchorRead, "mfm.evm.capability.read-anchor@1", Anchor);
impl_read_capability!(EvmBalanceRead, "mfm.evm.capability.read-balance@1", Balance);

fn read_value_valid_for_intent(intent: &EvmReadIntent, value: &EvmReadValue) -> bool {
    match (&intent.subject, value) {
        // A returned different chain is authenticated domain evidence; the State owns rejection.
        (EvmReadSubject::ChainIdentity, EvmReadValue::ChainId(_))
        | (EvmReadSubject::InitialAnchor, EvmReadValue::Anchor(_))
        | (EvmReadSubject::NativeBalance { .. }, EvmReadValue::RawUnits(_))
        | (EvmReadSubject::TokenDecimals { .. }, EvmReadValue::TokenDecimals(_))
        | (EvmReadSubject::TokenBalance { .. }, EvmReadValue::RawUnits(_)) => true,
        (EvmReadSubject::ConfirmAnchor { .. }, EvmReadValue::Anchor(_)) => true,
        _ => false,
    }
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !string_contains_secret_marker(value)
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod balance_extremes;
