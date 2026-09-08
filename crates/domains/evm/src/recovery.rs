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

/// Selectable balance recovery assessment; scheduling remains the caller's handler policy.
pub struct EvmBalanceClassifier;
impl
    mfm_program::Classifier<
        mfm_program::Incident<EvmBalanceFailure, EvmOperationalError, EvmBalanceAdapterContext>,
    > for EvmBalanceClassifier
{
    type Params = mfm_program::NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.evm.classifier.balance@1")
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
    fn classify(
        _: &Self::Params,
        incident: &mfm_program::Incident<
            EvmBalanceFailure,
            EvmOperationalError,
            EvmBalanceAdapterContext,
        >,
        _: &mfm_program::RecoveryContext<'_>,
    ) -> Result<mfm_program::Assessment, mfm_program::StateExecutionError> {
        use mfm_program::{Assessment, Incident};
        Ok(match incident {
            Incident::Adapter {
                original:
                    EvmOperationalError::Unavailable
                    | EvmOperationalError::Timeout
                    | EvmOperationalError::RateLimited,
                ..
            }
            | Incident::Domain(EvmBalanceFailure::AnchorChanged { .. }) => Assessment::Recoverable,
            Incident::Domain(
                EvmBalanceFailure::SourceUnavailable { .. }
                | EvmBalanceFailure::IntegrityBlocked { .. },
            ) => Assessment::Nonrecoverable,
        })
    }
}
