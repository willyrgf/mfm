//! Bounded operational causes and deterministic State-owned incident context.

use super::*;

/// Reviewed failure to obtain an EVM provider observation or settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "operational-error",
    version = "1",
    schema = "mfm.evm-operational-error"
)]
pub enum EvmOperationalError {
    /// The provider did not produce a usable response.
    Unavailable,
    /// The bounded provider deadline expired.
    Timeout,
    /// The provider explicitly limited request traffic.
    RateLimited,
}

/// Public context for one failed balance Read, excluding the caller continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-adapter-context",
    version = "1",
    schema = "mfm.evm-balance-adapter-context"
)]
pub struct EvmBalanceAdapterContext {
    collection_ordinal: u32,
    source_ordinal: u32,
    intent: EvmReadIntent,
}
impl_checked_deserialize!(EvmBalanceAdapterContext {
    collection_ordinal: u32,
    source_ordinal: u32,
    intent: EvmReadIntent,
});
impl EvmBalanceAdapterContext {
    /// Constructs context for one bounded source and its exact prepared intent.
    pub fn new(
        collection_ordinal: u32,
        source_ordinal: u32,
        intent: EvmReadIntent,
    ) -> Result<Self, EvmDomainError> {
        let context = Self {
            collection_ordinal,
            source_ordinal,
            intent,
        };
        context.validate()?;
        Ok(context)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.source_ordinal as usize >= EVM_BALANCE_SOURCE_LIMIT {
            return Err(EvmDomainError::InvalidValue);
        }
        self.intent.validate()
    }
    /// Returns the caller's checked collection ordinal.
    pub const fn collection_ordinal(&self) -> u32 {
        self.collection_ordinal
    }
    /// Returns the source ordinal within the collection.
    pub const fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
    /// Returns stage, source, route and any expected anchor through the exact intent.
    pub const fn intent(&self) -> &EvmReadIntent {
        &self.intent
    }
}

/// Public target facts for an operational anchored contract-call failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-call-adapter-context",
    version = "1",
    schema = "mfm.evm-anchored-call-adapter-context"
)]
pub struct AnchoredCallAdapterContext {
    chain_id: NonZeroU64,
    route_ref: ContentRef,
    anchor: EvmBlockAnchor,
    target: EvmAddress,
}
impl AnchoredCallAdapterContext {
    /// Derives bounded public facts without copying call bytes or the caller context.
    pub fn from_intent(intent: &AnchoredContractCallIntent) -> Self {
        Self {
            chain_id: intent.chain_id(),
            route_ref: intent.route_ref().clone(),
            anchor: intent.anchor().clone(),
            target: intent.target().clone(),
        }
    }
    /// Returns the expected chain identity.
    pub const fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }
    /// Returns the checked route reference.
    pub const fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Returns the exact expected anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }
    /// Returns the public contract target.
    pub const fn target(&self) -> &EvmAddress {
        &self.target
    }
}

/// Reviewed operational cause during transaction preparation or reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-operational-error",
    version = "1",
    schema = "mfm.evm-transaction-operational-error"
)]
pub enum EvmTransactionOperationalError {
    /// The EVM provider did not produce the required observation.
    Provider {
        /// Closed provider cause without response text.
        cause: EvmOperationalError,
    },
    /// Transaction authority did not acknowledge the requested operation.
    AuthorityUnavailable,
    /// The signer could not produce a signature.
    SignerUnavailable,
}

/// State-owned transaction facts; Runtime separately supplies authoritative execution phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-adapter-context",
    version = "1",
    schema = "mfm.evm-transaction-adapter-context"
)]
pub enum EvmTransactionAdapterContext {
    /// The nonce reservation State's public binding.
    Reservation {
        /// Checked route and authority identity.
        binding: EvmTransactionBinding,
    },
    /// The preparation State's retained reservation.
    Preparation {
        /// Exact reservation facts, without the complete command payload.
        reservation: Reservation,
    },
    /// The execution State's retained transaction identity.
    Execution {
        /// Exact reservation facts.
        reservation: Reservation,
        /// Hash of the prepared transaction.
        transaction_hash: EvmHash,
    },
}

impl mfm_program::ClassifyError for EvmOperationalError {
    fn classify(&self) -> mfm_program::Classification {
        // Executable Reads using this exact cause contract are duplicate-safe observations.
        match self {
            Self::Unavailable | Self::Timeout | Self::RateLimited => {
                mfm_program::Classification::Retryable
            }
        }
    }
}

impl mfm_program::ClassifyError for EvmBalanceFailure {
    fn classify(&self) -> mfm_program::Classification {
        match self {
            Self::AnchorChanged { .. } => mfm_program::Classification::InputInvalidated,
            Self::SourceUnavailable { .. } | Self::IntegrityBlocked { .. } => {
                mfm_program::Classification::Permanent
            }
        }
    }
}

impl mfm_program::ClassifyError for EvmTransactionOperationalError {
    fn classify(&self) -> mfm_program::Classification {
        match self {
            // These causes do not establish whether the external operation was acknowledged.
            Self::Provider { .. } | Self::AuthorityUnavailable => {
                mfm_program::Classification::OutcomeUnknown
            }
            // Signature acquisition precedes retaining and broadcasting prepared wire.
            Self::SignerUnavailable => mfm_program::Classification::Retryable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program::{Classification, ClassifyError};

    #[test]
    fn operational_semantics_distinguish_observations_from_unknown_transaction_outcomes() {
        for cause in [
            EvmOperationalError::Unavailable,
            EvmOperationalError::Timeout,
            EvmOperationalError::RateLimited,
        ] {
            assert_eq!(cause.classify(), Classification::Retryable);
            assert_eq!(
                EvmTransactionOperationalError::Provider { cause }.classify(),
                Classification::OutcomeUnknown
            );
        }
        assert_eq!(
            EvmTransactionOperationalError::AuthorityUnavailable.classify(),
            Classification::OutcomeUnknown
        );
        assert_eq!(
            EvmTransactionOperationalError::SignerUnavailable.classify(),
            Classification::Retryable
        );
    }
}
