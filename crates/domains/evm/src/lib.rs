#![warn(missing_docs)]
//! Secret-free EVM State values and capability contracts.
//!
//! The domain owns complete cumulative contexts for the two current operations.  Provider
//! clients, signing authority, nonce storage, and response ingress belong to the live adapter
//! crate; no domain value contains a client, credential, raw response, or generic context map.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::num::NonZeroU16;

use mfm_capabilities::{
    AccessCapabilityContract, EffectMode, NoPriorFacts, ProposedStateOutcome, ReadMode,
};
use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    capability_contract_ref, nominal_contract_ref, state_implementation_ref, BindingDescriptor,
    Declaration, ExecutionMode, FailureValue, MatchDeclaration, MatchVariant, ProgramDocument,
    SequentialControlAddress, State, StateDeclaration,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_values::{string_contains_secret_marker, MfmValue as MfmValueTrait};
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

/// Stable entry-point identity for EVM submission.
pub const EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID: &str = "mfm.evm/submit-transaction@1";
/// Maximum admitted EVM balance sources.
pub const EVM_BALANCE_SOURCE_LIMIT: usize = 64;
/// Maximum EVM transaction payload bytes.
const EVM_TRANSACTION_DATA_LIMIT: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmSubmissionRoute {
    pub target: EvmTransactionTarget,
    pub public_signer_key_instance_ref: ContentRef,
}

/// Secret-free EVM configuration selected by trusted composition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(validate = "validate_evm_config")]
pub struct EvmConfig {
    /// Finite deployment-selected submission routes.
    submission_routes: Vec<EvmSubmissionRoute>,
}

fn validate_evm_config(config: &EvmConfig) -> Result<(), EvmDomainError> {
    if config.submission_routes.is_empty()
        || config.submission_routes.len() > EVM_BALANCE_SOURCE_LIMIT
        || config
            .submission_routes
            .iter()
            .any(|route| route.target.validate().is_err())
        || config
            .submission_routes
            .iter()
            .enumerate()
            .any(|(index, route)| {
                config.submission_routes[..index]
                    .iter()
                    .any(|prior| prior.target == route.target)
            })
    {
        return Err(EvmDomainError::InvalidValue);
    }
    Ok(())
}

/// A checked public EVM target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmTransactionTarget {
    /// Chain identity selected by the trusted configuration.
    pub chain_id: u64,
    /// Public sender address in lowercase hexadecimal form.
    pub sender: String,
    /// Wallet nonce domain.
    pub nonce_domain: String,
}

impl_checked_deserialize!(EvmTransactionTarget {
    chain_id: u64,
    sender: String,
    nonce_domain: String,
});

impl EvmTransactionTarget {
    /// Creates one bounded public target.
    pub fn new(
        chain_id: u64,
        sender: String,
        nonce_domain: String,
    ) -> Result<Self, EvmDomainError> {
        if chain_id == 0
            || !valid_public_text(&sender, 128)
            || sender != sender.to_ascii_lowercase()
            || !valid_public_text(&nonce_domain, 256)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            chain_id,
            sender,
            nonce_domain,
        })
    }

    /// Validates a decoded target without changing ownership.
    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(
            self.chain_id,
            self.sender.clone(),
            self.nonce_domain.clone(),
        )
        .map(|_| ())
    }
}

/// Public EVM submission selector accepted from a transport.
///
/// It contains no signer, route identity, binding, or planned context. Trusted configuration and
/// live bindings select those fields during [`plan_submission`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionSelector {
    /// Requested deployment-selected target.
    pub target: EvmTransactionTarget,
    /// Bounded non-secret idempotency key.
    pub idempotency_key: String,
    /// Canonical unsigned transaction data.
    pub data: Vec<u8>,
    /// Gas limit.
    pub gas_limit: u64,
    /// Fee cap in integer smallest units.
    pub max_fee: String,
}

impl_checked_deserialize!(EvmSubmissionSelector {
    target: EvmTransactionTarget,
    idempotency_key: String,
    data: Vec<u8>,
    gas_limit: u64,
    max_fee: String,
});

impl EvmSubmissionSelector {
    /// Constructs one bounded transport selector.
    pub fn new(
        target: EvmTransactionTarget,
        idempotency_key: String,
        data: Vec<u8>,
        gas_limit: u64,
        max_fee: String,
    ) -> Result<Self, EvmDomainError> {
        validate_submission_fields(&target, &idempotency_key, &data, gas_limit, &max_fee)?;
        Ok(Self {
            target,
            idempotency_key,
            data,
            gas_limit,
            max_fee,
        })
    }

    /// Validates a decoded selector without changing ownership.
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_submission_fields(
            &self.target,
            &self.idempotency_key,
            &self.data,
            self.gas_limit,
            &self.max_fee,
        )
    }
}

/// A singular domain-planned admission value for EVM submission.
///
/// Its fields are private so only the domain planner can bind a selected public signer identity.
#[derive(Debug, Clone, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionRequest {
    target: EvmTransactionTarget,
    idempotency_key: String,
    data: Vec<u8>,
    gas_limit: u64,
    max_fee: String,
    public_signer_key_instance_ref: ContentRef,
}

impl_checked_deserialize!(EvmSubmissionRequest {
    target: EvmTransactionTarget,
    idempotency_key: String,
    data: Vec<u8>,
    gas_limit: u64,
    max_fee: String,
    public_signer_key_instance_ref: ContentRef,
});

impl EvmSubmissionRequest {
    fn planned(
        selector: EvmSubmissionSelector,
        public_signer_key_instance_ref: ContentRef,
    ) -> Result<Self, EvmDomainError> {
        selector.validate()?;
        Ok(Self {
            target: selector.target,
            idempotency_key: selector.idempotency_key,
            data: selector.data,
            gas_limit: selector.gas_limit,
            max_fee: selector.max_fee,
            public_signer_key_instance_ref,
        })
    }

    /// Validates a decoded submission request by representation and domain bounds.
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_submission_fields(
            &self.target,
            &self.idempotency_key,
            &self.data,
            self.gas_limit,
            &self.max_fee,
        )
    }
}

fn validate_submission_fields(
    target: &EvmTransactionTarget,
    idempotency_key: &str,
    data: &[u8],
    gas_limit: u64,
    max_fee: &str,
) -> Result<(), EvmDomainError> {
    target.validate()?;
    if idempotency_key.is_empty()
        || idempotency_key.len() > 256
        || string_contains_secret_marker(idempotency_key)
        || data.len() > EVM_TRANSACTION_DATA_LIMIT
        || gas_limit == 0
        || !is_decimal_integer(max_fee)
        || max_fee.len() > 80
    {
        return Err(EvmDomainError::InvalidValue);
    }
    Ok(())
}

/// Opaque cumulative submission progress passed between the fixed submission States.
///
/// The private closed phase sum preserves the exact nonce, candidate, broadcast, receipt,
/// finality, and canonical-inclusion proof required by the next semantic State.  Callers cannot
/// construct an intermediate phase or bypass one of those States.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionProgress {
    request: EvmSubmissionRequest,
    phase: SubmissionPhase,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum SubmissionPhase {
    Reserved {
        nonce: u64,
    },
    Candidate {
        nonce: u64,
        candidate_id: String,
    },
    Broadcast {
        nonce: u64,
        candidate_id: String,
        transaction_hash: String,
    },
    Receipt {
        nonce: u64,
        candidate_id: String,
        transaction_hash: String,
        execution_disposition: EvmExecutionDisposition,
        inclusion_block: EvmBlockAnchor,
    },
    Finalized {
        nonce: u64,
        candidate_id: String,
        transaction_hash: String,
        execution_disposition: EvmExecutionDisposition,
        inclusion_block: EvmBlockAnchor,
        finalized_head_number: String,
    },
    Canonical {
        nonce: u64,
        candidate_id: String,
        transaction_hash: String,
        execution_disposition: EvmExecutionDisposition,
        inclusion_block: EvmBlockAnchor,
        finalized_head_number: String,
        canonical_inclusion_block: EvmBlockAnchor,
    },
}

impl<'de> Deserialize<'de> for EvmSubmissionProgress {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: EvmSubmissionRequest,
            phase: SubmissionPhase,
        }

        let wire = Wire::deserialize(deserializer)?;
        let value = Self {
            request: wire.request,
            phase: wire.phase,
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

impl EvmSubmissionProgress {
    fn new(request: EvmSubmissionRequest, phase: SubmissionPhase) -> Result<Self, EvmDomainError> {
        let value = Self { request, phase };
        value.validate().map(|_| value)
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.request.validate()?;
        match &self.phase {
            SubmissionPhase::Reserved { .. } => Ok(()),
            SubmissionPhase::Candidate {
                nonce,
                candidate_id,
            } => valid_submission_candidate(&self.request.target, *nonce, candidate_id),
            SubmissionPhase::Broadcast {
                nonce,
                candidate_id,
                transaction_hash,
                ..
            } => {
                valid_submission_candidate(&self.request.target, *nonce, candidate_id)?;
                valid_submission_hash(transaction_hash)
            }
            SubmissionPhase::Receipt {
                nonce,
                candidate_id,
                transaction_hash,
                inclusion_block,
                ..
            } => valid_submission_receipt(
                &self.request.target,
                *nonce,
                candidate_id,
                transaction_hash,
                inclusion_block,
            ),
            SubmissionPhase::Finalized {
                nonce,
                candidate_id,
                transaction_hash,
                inclusion_block,
                finalized_head_number,
                ..
            } => valid_submission_finality(
                &self.request.target,
                *nonce,
                candidate_id,
                transaction_hash,
                inclusion_block,
                finalized_head_number,
            ),
            SubmissionPhase::Canonical {
                nonce,
                candidate_id,
                transaction_hash,
                inclusion_block,
                finalized_head_number,
                canonical_inclusion_block,
                ..
            } => {
                valid_submission_finality(
                    &self.request.target,
                    *nonce,
                    candidate_id,
                    transaction_hash,
                    inclusion_block,
                    finalized_head_number,
                )?;
                canonical_inclusion_block.validate()?;
                (canonical_inclusion_block == inclusion_block)
                    .then_some(())
                    .ok_or(EvmDomainError::InvalidValue)
            }
        }
    }
}

fn submission_candidate_id(target: &EvmTransactionTarget, nonce: u64) -> String {
    format!(
        "mfm.evm.candidate/{}/{}/{}/{}",
        target.chain_id, target.sender, target.nonce_domain, nonce
    )
}

fn valid_submission_candidate(
    target: &EvmTransactionTarget,
    nonce: u64,
    candidate_id: &str,
) -> Result<(), EvmDomainError> {
    (candidate_id == submission_candidate_id(target, nonce) && valid_public_text(candidate_id, 256))
        .then_some(())
        .ok_or(EvmDomainError::InvalidValue)
}

fn valid_submission_hash(transaction_hash: &str) -> Result<(), EvmDomainError> {
    valid_public_text(transaction_hash, 256)
        .then_some(())
        .ok_or(EvmDomainError::InvalidValue)
}

fn valid_submission_receipt(
    target: &EvmTransactionTarget,
    nonce: u64,
    candidate_id: &str,
    transaction_hash: &str,
    inclusion_block: &EvmBlockAnchor,
) -> Result<(), EvmDomainError> {
    valid_submission_candidate(target, nonce, candidate_id)?;
    valid_submission_hash(transaction_hash)?;
    inclusion_block.validate()
}

fn valid_submission_finality(
    target: &EvmTransactionTarget,
    nonce: u64,
    candidate_id: &str,
    transaction_hash: &str,
    inclusion_block: &EvmBlockAnchor,
    finalized_head_number: &str,
) -> Result<(), EvmDomainError> {
    valid_submission_receipt(
        target,
        nonce,
        candidate_id,
        transaction_hash,
        inclusion_block,
    )?;
    (is_decimal_integer(finalized_head_number)
        && decimal_at_least(finalized_head_number, &inclusion_block.number))
    .then_some(())
    .ok_or(EvmDomainError::InvalidValue)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum EvmExecutionDisposition {
    Succeeded,
    Reverted,
}

impl EvmExecutionDisposition {
    const fn code(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Reverted => "reverted",
        }
    }

    fn from_code(value: &str) -> Option<Self> {
        match value {
            "succeeded" => Some(Self::Succeeded),
            "reverted" => Some(Self::Reverted),
            _ => None,
        }
    }
}

/// Public terminal EVM submission result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionOutput {
    /// Authenticated receipt/finality disposition, with all operational detail retained privately.
    execution_disposition: String,
}

impl_checked_deserialize!(EvmSubmissionOutput {
    execution_disposition: String,
});

impl EvmSubmissionOutput {
    /// Constructs the exact frozen public success projection.
    fn new(execution_disposition: EvmExecutionDisposition) -> Self {
        Self {
            execution_disposition: execution_disposition.code().to_owned(),
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        EvmExecutionDisposition::from_code(&self.execution_disposition)
            .map(|_| ())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

/// Explicit fail-fast EVM submission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvmSubmissionFailure {
    /// The wallet nonce authority could not reserve the operation safely.
    NonceAuthorityUnavailable,
    /// The authenticated destination rejected the committed candidate before entry.
    DestinationRejected,
    /// The provider could not supply a safe, usable observation.
    ProviderUnavailable,
    /// Retained nonce/candidate lineage was internally impossible.
    NonceLineageDiverged,
}

impl<'de> Deserialize<'de> for EvmSubmissionFailure {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields)]
        struct Wire {
            kind: String,
        }

        match Wire::deserialize(deserializer)?.kind.as_str() {
            "nonce_authority_unavailable" => Ok(Self::NonceAuthorityUnavailable),
            "destination_rejected" => Ok(Self::DestinationRejected),
            "provider_unavailable" => Ok(Self::ProviderUnavailable),
            "nonce_lineage_diverged" => Ok(Self::NonceLineageDiverged),
            _ => Err(de::Error::custom(EvmDomainError::InvalidValue)),
        }
    }
}

impl FailureValue for EvmSubmissionFailure {
    fn integrity_blocked() -> Self {
        Self::ProviderUnavailable
    }
}

/// One EVM balance source admitted into a cumulative collection context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceSource {
    /// Public source identity.
    pub source_id: String,
    /// Public chain id.
    pub chain_id: u64,
    /// Public account address.
    pub address: String,
    /// Optional token contract address.
    pub token: Option<String>,
}

impl_checked_deserialize!(EvmBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    token: Option<String>,
});

impl EvmBalanceSource {
    /// Validates one public source identity without changing ownership.
    fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.source_id, 256)
            || !valid_public_text(&self.address, 128)
            || self.address != self.address.to_ascii_lowercase()
            || self.chain_id == 0
            || self.token.as_ref().is_some_and(|token| {
                !valid_public_text(token, 128) || token != &token.to_ascii_lowercase()
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Singular domain-planned EVM balance fragment admission value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceRequest {
    /// Ordered sources; the sequential Program expands one State per source.
    pub sources: Vec<EvmBalanceSource>,
    /// Integer decimal scale for public amounts.
    pub decimals: u8,
}

impl_checked_deserialize!(EvmBalanceRequest {
    sources: Vec<EvmBalanceSource>,
    decimals: u8,
});

impl EvmBalanceRequest {
    /// Creates one bounded declaration-ordered source list.
    pub fn new(sources: Vec<EvmBalanceSource>, decimals: u8) -> Result<Self, EvmDomainError> {
        if sources.is_empty()
            || sources.len() > EVM_BALANCE_SOURCE_LIMIT
            || decimals > 30
            || sources.iter().any(|source| source.validate().is_err())
            || duplicate_source_ids(&sources)
            || sources.first().is_some_and(|first| {
                sources
                    .iter()
                    .any(|source| source.chain_id != first.chain_id)
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { sources, decimals })
    }

    /// Validates a decoded balance request and every admitted source.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if self.sources.is_empty()
            || self.sources.len() > EVM_BALANCE_SOURCE_LIMIT
            || self.decimals > 30
            || self.sources.iter().any(|source| source.validate().is_err())
            || duplicate_source_ids(&self.sources)
            || self.sources.first().is_some_and(|first| {
                self.sources
                    .iter()
                    .any(|source| source.chain_id != first.chain_id)
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }

    /// Scales one exact observed raw amount to this request's declared decimal scale.
    pub fn scale_units(&self, raw_units: &str, source_decimals: u8) -> Option<String> {
        scale_units(raw_units, source_decimals, self.decimals)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceResultMetadata {
    collection_ordinal: u32,
    correlation: String,
}

impl_checked_deserialize!(EvmBalanceResultMetadata {
    collection_ordinal: u32,
    correlation: String,
});

impl EvmBalanceResultMetadata {
    fn new(collection_ordinal: u32, correlation: String) -> Result<Self, EvmDomainError> {
        if !valid_public_text(&correlation, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            collection_ordinal,
            correlation,
        })
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(self.collection_ordinal, self.correlation.clone()).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBlockAnchor {
    number: String,
    hash: String,
}

impl_checked_deserialize!(EvmBlockAnchor {
    number: String,
    hash: String,
});

impl EvmBlockAnchor {
    fn new(number: String, hash: String) -> Result<Self, EvmDomainError> {
        if !is_decimal_integer(&number) || !valid_public_text(&hash, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { number, hash })
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(self.number.clone(), self.hash.clone()).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvmBalanceWork {
    CheckChainIdentity {
        source: EvmBalanceSource,
    },
    ReadInitialAnchor {
        source: EvmBalanceSource,
        checked_chain_id: u64,
    },
    SelectAsset {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadNativeBalance {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadTokenDecimals {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadTokenBalance {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
        token_decimals: u8,
    },
    ConfirmAnchor {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
        source_decimals: u8,
        raw_balance: String,
    },
    Complete,
}

/// Complete cumulative balance context consumed by each source State and consolidation State.
///
/// `K` is the caller-owned continuation.  The EVM domain stores and returns it without
/// interpreting its fields; the consuming handoff is what keeps Portfolio semantics outside the
/// reusable EVM fragment.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub struct EvmBalanceContext<K: MfmValueTrait> {
    request: EvmBalanceRequest,
    caller_continuation: K,
    metadata: EvmBalanceResultMetadata,
    completed: Vec<EvmBalanceResult>,
    work: EvmBalanceWork,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceContext<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K> {
            request: EvmBalanceRequest,
            caller_continuation: K,
            metadata: EvmBalanceResultMetadata,
            completed: Vec<EvmBalanceResult>,
            work: EvmBalanceWork,
        }

        let wire = Wire::deserialize(deserializer)?;
        let context = Self {
            request: wire.request,
            caller_continuation: wire.caller_continuation,
            metadata: wire.metadata,
            completed: wire.completed,
            work: wire.work,
        };
        context
            .validate()
            .map(|_| context)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceContext<K> {
    /// Creates the first work item for one caller-owned collection continuation.
    pub fn new(
        request: EvmBalanceRequest,
        caller_continuation: K,
        collection_ordinal: u32,
        correlation: String,
    ) -> Result<Self, EvmDomainError> {
        let first = request
            .sources
            .first()
            .cloned()
            .ok_or(EvmDomainError::InvalidValue)?;
        let metadata = EvmBalanceResultMetadata::new(collection_ordinal, correlation)?;
        let context = Self {
            request,
            caller_continuation,
            metadata,
            completed: Vec::new(),
            work: EvmBalanceWork::CheckChainIdentity { source: first },
        };
        context.validate()?;
        Ok(context)
    }

    fn active_source(&self) -> Option<&EvmBalanceSource> {
        match &self.work {
            EvmBalanceWork::CheckChainIdentity { source }
            | EvmBalanceWork::ReadInitialAnchor { source, .. }
            | EvmBalanceWork::SelectAsset { source, .. }
            | EvmBalanceWork::ReadNativeBalance { source, .. }
            | EvmBalanceWork::ReadTokenDecimals { source, .. }
            | EvmBalanceWork::ReadTokenBalance { source, .. }
            | EvmBalanceWork::ConfirmAnchor { source, .. } => Some(source),
            EvmBalanceWork::Complete => None,
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.request.validate()?;
        self.metadata.validate()?;
        if self.completed.len() > self.request.sources.len()
            || self
                .completed
                .iter()
                .zip(&self.request.sources)
                .any(|(result, source)| {
                    result.validate().is_err()
                        || result.source != *source
                        || (result.source.token.is_none()
                            && result.decimals != self.request.decimals)
                        || self
                            .request
                            .scale_units(&result.raw_units, result.decimals)
                            .is_none()
                })
            || self
                .completed
                .windows(2)
                .any(|pair| pair[0].anchor != pair[1].anchor)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        if let Some(anchor) = work_initial_anchor(&self.work) {
            if anchor.validate().is_err()
                || self
                    .completed
                    .first()
                    .is_some_and(|result| result.anchor != *anchor)
            {
                return Err(EvmDomainError::InvalidValue);
            }
        }
        let expected = self.request.sources.get(self.completed.len());
        match (&self.work, expected) {
            (EvmBalanceWork::Complete, None) => {}
            (EvmBalanceWork::Complete, Some(_)) => return Err(EvmDomainError::InvalidValue),
            (work, Some(expected_source)) => {
                let source = self.active_source().ok_or(EvmDomainError::InvalidValue)?;
                if source != expected_source || source.validate().is_err() {
                    return Err(EvmDomainError::InvalidValue);
                }
                match work {
                    EvmBalanceWork::ReadNativeBalance { source, .. } if source.token.is_some() => {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadTokenDecimals { source, .. }
                    | EvmBalanceWork::ReadTokenBalance { source, .. }
                        if source.token.is_none() =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadTokenBalance { token_decimals, .. }
                        if *token_decimals > 30 =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ConfirmAnchor {
                        source_decimals,
                        raw_balance,
                        initial_anchor,
                        checked_chain_id,
                        ..
                    } if *source_decimals > 30
                        || (source.token.is_none()
                            && *source_decimals != self.request.decimals)
                        || !is_decimal_integer(raw_balance)
                        || initial_anchor.validate().is_err()
                        || *checked_chain_id != source.chain_id =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadInitialAnchor {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::SelectAsset {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadNativeBalance {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadTokenDecimals {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadTokenBalance {
                        checked_chain_id, ..
                    } if *checked_chain_id != source.chain_id => {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    _ => {}
                }
            }
            (_, None) => return Err(EvmDomainError::InvalidValue),
        }
        Ok(())
    }
}

/// Closed Match selector retaining the entire EVM context through either asset arm.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub enum EvmBalanceAsset<K: MfmValueTrait> {
    /// Native arm with the complete cumulative context.
    Native(EvmBalanceContext<K>),
    /// Token arm with the complete cumulative context.
    Token(EvmBalanceContext<K>),
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceAsset<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        enum Wire<K: MfmValueTrait> {
            Native(EvmBalanceContext<K>),
            Token(EvmBalanceContext<K>),
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::Native(context) => Self::Native(context),
            Wire::Token(context) => Self::Token(context),
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceAsset<K> {
    fn validate(&self) -> Result<(), EvmDomainError> {
        let context = match self {
            Self::Native(context) | Self::Token(context) => context,
        };
        context.validate()?;
        match (self, context.active_source()) {
            (Self::Native(_), Some(source)) if source.token.is_none() => Ok(()),
            (Self::Token(_), Some(source)) if source.token.is_some() => Ok(()),
            _ => Err(EvmDomainError::InvalidValue),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceResult {
    source: EvmBalanceSource,
    decimals: u8,
    raw_units: String,
    anchor: EvmBlockAnchor,
}

impl_checked_deserialize!(EvmBalanceResult {
    source: EvmBalanceSource,
    decimals: u8,
    raw_units: String,
    anchor: EvmBlockAnchor,
});

impl EvmBalanceResult {
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.source.validate().is_err()
            || self.decimals > 30
            || !is_decimal_integer(&self.raw_units)
            || self.anchor.validate().is_err()
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvmCollectedAsset {
    Native,
    Token { contract: String },
}

impl EvmCollectedAsset {
    fn validate(&self) -> Result<(), EvmDomainError> {
        match self {
            Self::Native => Ok(()),
            Self::Token { contract }
                if valid_public_text(contract, 128)
                    && contract == &contract.to_ascii_lowercase() =>
            {
                Ok(())
            }
            Self::Token { .. } => Err(EvmDomainError::InvalidValue),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmCollectedBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    asset: EvmCollectedAsset,
}

impl_checked_deserialize!(EvmCollectedBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    asset: EvmCollectedAsset,
});

impl EvmCollectedBalanceSource {
    fn from_source(source: &EvmBalanceSource) -> Self {
        Self {
            source_id: source.source_id.clone(),
            chain_id: source.chain_id,
            address: source.address.clone(),
            asset: match &source.token {
                Some(contract) => EvmCollectedAsset::Token {
                    contract: contract.clone(),
                },
                None => EvmCollectedAsset::Native,
            },
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.source_id, 256)
            || self.chain_id == 0
            || !valid_public_text(&self.address, 128)
            || self.address != self.address.to_ascii_lowercase()
        {
            return Err(EvmDomainError::InvalidValue);
        }
        self.asset.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmCollectedBalance {
    source: EvmCollectedBalanceSource,
    decimals: u8,
    raw_units: String,
}

impl_checked_deserialize!(EvmCollectedBalance {
    source: EvmCollectedBalanceSource,
    decimals: u8,
    raw_units: String,
});

impl EvmCollectedBalance {
    fn from_result(result: EvmBalanceResult) -> Self {
        Self {
            source: EvmCollectedBalanceSource::from_source(&result.source),
            decimals: result.decimals,
            raw_units: result.raw_units,
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.source.validate().is_err()
            || self.decimals > 30
            || !is_decimal_integer(&self.raw_units)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

type EvmCollectedBalanceParts = (String, String, Option<String>, u8, String);
type EvmBalanceCollectionParts = (u64, EvmBlockAnchor, Vec<EvmCollectedBalanceParts>, String);
type EvmBalanceCollectionCompletionParts<K> = (
    K,
    u32,
    u64,
    String,
    String,
    Vec<EvmCollectedBalanceParts>,
    String,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceCollectionResult {
    chain_id: u64,
    anchor: EvmBlockAnchor,
    balances: Vec<EvmCollectedBalance>,
    total_scaled: String,
}

impl_checked_deserialize!(EvmBalanceCollectionResult {
    chain_id: u64,
    anchor: EvmBlockAnchor,
    balances: Vec<EvmCollectedBalance>,
    total_scaled: String,
});

impl EvmBalanceCollectionResult {
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.chain_id == 0
            || self.anchor.validate().is_err()
            || self.balances.is_empty()
            || self.balances.len() > EVM_BALANCE_SOURCE_LIMIT
            || self
                .balances
                .iter()
                .any(|balance| balance.validate().is_err())
            || duplicate_collected_source_ids(&self.balances)
            || self
                .balances
                .iter()
                .any(|balance| balance.source.chain_id != self.chain_id)
            || !is_decimal_integer(&self.total_scaled)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }

    fn into_parts(self) -> EvmBalanceCollectionParts {
        let balances = self
            .balances
            .into_iter()
            .map(|balance| {
                let token = match balance.source.asset {
                    EvmCollectedAsset::Native => None,
                    EvmCollectedAsset::Token { contract } => Some(contract),
                };
                (
                    balance.source.source_id,
                    balance.source.address,
                    token,
                    balance.decimals,
                    balance.raw_units,
                )
            })
            .collect();
        (self.chain_id, self.anchor, balances, self.total_scaled)
    }
}

/// One consuming frozen EVM collection handoff returned to the caller context.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub struct EvmBalanceCollectionCompletion<K: MfmValueTrait> {
    caller_context: K,
    collection_ordinal: u32,
    collection: EvmBalanceCollectionResult,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceCollectionCompletion<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K> {
            caller_context: K,
            collection_ordinal: u32,
            collection: EvmBalanceCollectionResult,
        }
        let wire = Wire::deserialize(deserializer)?;
        let completion = Self {
            caller_context: wire.caller_context,
            collection_ordinal: wire.collection_ordinal,
            collection: wire.collection,
        };
        completion
            .validate()
            .map(|_| completion)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceCollectionCompletion<K> {
    fn new(
        caller_context: K,
        collection_ordinal: u32,
        collection: EvmBalanceCollectionResult,
    ) -> Result<Self, EvmDomainError> {
        collection.validate()?;
        Ok(Self {
            caller_context,
            collection_ordinal,
            collection,
        })
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.collection.validate()
    }

    /// Consumes the EVM completion into its unchanged caller context and checked observations.
    pub fn into_parts(self) -> EvmBalanceCollectionCompletionParts<K> {
        let (chain_id, anchor, balances, total_scaled) = self.collection.into_parts();
        (
            self.caller_context,
            self.collection_ordinal,
            chain_id,
            anchor.number,
            anchor.hash,
            balances,
            total_scaled,
        )
    }
}

/// Redaction-safe EVM domain construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmDomainError {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum EvmReadSubject {
    ChainIdentity,
    InitialAnchor,
    NativeBalance {
        source: EvmBalanceSource,
        anchor: EvmBlockAnchor,
    },
    TokenDecimals {
        source: EvmBalanceSource,
        anchor: EvmBlockAnchor,
    },
    TokenBalance {
        source: EvmBalanceSource,
        anchor: EvmBlockAnchor,
    },
    ConfirmAnchor {
        source: EvmBalanceSource,
        anchor: EvmBlockAnchor,
    },
    TransactionReceipt {
        transaction_hash: String,
    },
    FinalizedHead,
    CanonicalInclusionBlock {
        number: String,
    },
}

/// One strict read intent shared by the bounded EVM read capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmReadIntent {
    operation: String,
    chain_id: u64,
    subject: EvmReadSubject,
}

impl_checked_deserialize!(EvmReadIntent {
    operation: String,
    chain_id: u64,
    subject: EvmReadSubject,
});

impl EvmReadIntent {
    fn new(
        operation: String,
        chain_id: u64,
        subject: EvmReadSubject,
    ) -> Result<Self, EvmDomainError> {
        let intent = Self {
            operation,
            chain_id,
            subject,
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Returns the exact operation and public chain target fixed by this intent.
    pub fn operation_and_chain_id(&self) -> (&str, u64) {
        (&self.operation, self.chain_id)
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if StableId::new(&self.operation).is_err() || self.chain_id == 0 {
            return Err(EvmDomainError::InvalidValue);
        }
        match &self.subject {
            EvmReadSubject::ChainIdentity
            | EvmReadSubject::InitialAnchor
            | EvmReadSubject::FinalizedHead => {}
            EvmReadSubject::NativeBalance { source, anchor }
            | EvmReadSubject::TokenDecimals { source, anchor }
            | EvmReadSubject::TokenBalance { source, anchor }
            | EvmReadSubject::ConfirmAnchor { source, anchor } => {
                source.validate()?;
                anchor.validate()?;
                if source.chain_id != self.chain_id {
                    return Err(EvmDomainError::InvalidValue);
                }
            }
            EvmReadSubject::TransactionReceipt { transaction_hash } => {
                if !valid_public_text(transaction_hash, 256) {
                    return Err(EvmDomainError::InvalidValue);
                }
            }
            EvmReadSubject::CanonicalInclusionBlock { number } => {
                if !is_decimal_integer(number) {
                    return Err(EvmDomainError::InvalidValue);
                }
            }
        }
        Ok(())
    }
}

/// Typed provider values admitted after raw EVM ingress is discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmReadValue {
    /// Authenticated chain id.
    ChainId(u64),
    /// Authenticated block anchor components.
    Anchor {
        /// Canonical decimal block number.
        number: String,
        /// Canonical public block hash.
        hash: String,
    },
    /// Canonical unsigned raw units.
    RawUnits(String),
    /// Token decimal scale.
    TokenDecimals(u8),
    /// Receipt observation bound to its transaction hash.
    Receipt {
        /// Transaction hash.
        transaction_hash: String,
        /// Receipt execution disposition code.
        execution_disposition: String,
        /// Canonical inclusion block number.
        inclusion_block_number: String,
        /// Canonical inclusion block hash.
        inclusion_block_hash: String,
    },
    /// Canonical finalized head number.
    FinalizedHead {
        /// Canonical decimal finalized-head number.
        number: String,
    },
    /// Canonical block at the requested height.
    CanonicalBlock {
        /// Canonical decimal block number.
        number: String,
        /// Canonical public block hash.
        hash: String,
    },
}

impl<'de> Deserialize<'de> for EvmReadValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields
        )]
        enum Wire {
            ChainId(u64),
            Anchor {
                number: String,
                hash: String,
            },
            RawUnits(String),
            TokenDecimals(u8),
            Receipt {
                transaction_hash: String,
                execution_disposition: String,
                inclusion_block_number: String,
                inclusion_block_hash: String,
            },
            FinalizedHead {
                number: String,
            },
            CanonicalBlock {
                number: String,
                hash: String,
            },
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::ChainId(chain_id) => Self::ChainId(chain_id),
            Wire::Anchor { number, hash } => Self::Anchor { number, hash },
            Wire::RawUnits(units) => Self::RawUnits(units),
            Wire::TokenDecimals(decimals) => Self::TokenDecimals(decimals),
            Wire::Receipt {
                transaction_hash,
                execution_disposition,
                inclusion_block_number,
                inclusion_block_hash,
            } => Self::Receipt {
                transaction_hash,
                execution_disposition,
                inclusion_block_number,
                inclusion_block_hash,
            },
            Wire::FinalizedHead { number } => Self::FinalizedHead { number },
            Wire::CanonicalBlock { number, hash } => Self::CanonicalBlock { number, hash },
        };
        read_value_valid(&value)
            .then_some(value)
            .ok_or_else(|| de::Error::custom(EvmDomainError::InvalidValue))
    }
}

/// Closed EVM read evidence sum; raw provider material is discarded before construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmReadEvidence {
    /// Provider returned an exact structured value for the committed intent.
    Returned {
        /// Interpreted provider value.
        value: EvmReadValue,
    },
    /// Provider returned a reviewed rejection.
    Rejected,
    /// Adapter accepted a safe failure.
    SafeFailure,
    /// Authentication/integrity failed and grants no retry authority.
    IntegrityBlocked,
}

impl EvmReadEvidence {
    /// Validates the exact operation-bound evidence envelope.
    fn validate_for(&self, intent: &EvmReadIntent) -> Result<(), EvmDomainError> {
        let valid = match self {
            Self::Returned { value } => read_value_valid_for_intent(intent, value),
            Self::Rejected | Self::SafeFailure | Self::IntegrityBlocked => true,
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

#[derive(Debug, Clone, Copy)]
enum ReadCapabilityFamily {
    ChainIdentity,
    LatestAnchor,
    Balance,
    SubmissionStatus,
}

fn validate_read_capability_intent(
    intent: &EvmReadIntent,
    family: ReadCapabilityFamily,
) -> Result<(), EvmDomainError> {
    intent.validate()?;
    let valid = match family {
        ReadCapabilityFamily::ChainIdentity => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-chain-identity@1",
                EvmReadSubject::ChainIdentity
            )
        ),
        ReadCapabilityFamily::LatestAnchor => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-initial-anchor@1",
                EvmReadSubject::InitialAnchor
            ) | (
                "mfm.evm.confirm-balance-anchor@1",
                EvmReadSubject::ConfirmAnchor { .. }
            )
        ),
        ReadCapabilityFamily::Balance => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-native-balance@1",
                EvmReadSubject::NativeBalance { .. }
            ) | (
                "mfm.evm.read-token-decimals@1",
                EvmReadSubject::TokenDecimals { .. }
            ) | (
                "mfm.evm.read-token-balance@1",
                EvmReadSubject::TokenBalance { .. }
            )
        ),
        ReadCapabilityFamily::SubmissionStatus => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-transaction-receipt@1",
                EvmReadSubject::TransactionReceipt { .. }
            ) | (
                "mfm.evm.read-finalized-head@1",
                EvmReadSubject::FinalizedHead
            ) | (
                "mfm.evm.read-canonical-inclusion-block@1",
                EvmReadSubject::CanonicalInclusionBlock { .. }
            )
        ),
    };
    valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
}

/// Exact EVM Access capability selected by its closed contract kind.
///
/// Kinds 0 and 1 are the one-entry nonce and broadcast Effects. Kind 3 is the submission-status
/// Read contract; its closed intent distinguishes receipt, finalized-head, and canonical-block
/// observations. The remaining Read contracts cover chain, anchor, and balance observations.
pub struct EvmCapability<const KIND: u8>;

macro_rules! impl_read_capability {
    ($ty:ty, $name:literal, $family:ident) => {
        impl AccessCapabilityContract for $ty {
            type Mode = ReadMode;
            type Intent = EvmReadIntent;
            type Evidence = EvmReadEvidence;
            type Facts = NoPriorFacts;

            fn contract_id() -> mfm_capabilities::Result<StableId> {
                StableId::new($name).map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
            }

            fn total_attempt_bound() -> NonZeroU16 {
                NonZeroU16::new(3).expect("constant")
            }

            fn bind_evidence(
                intent: &Self::Intent,
                evidence: &Self::Evidence,
            ) -> mfm_capabilities::Result<()> {
                validate_read_capability_intent(intent, ReadCapabilityFamily::$family)
                    .and_then(|_| evidence.validate_for(intent))
                    .map_err(|_| mfm_capabilities::CapabilityError::EvidenceBinding)
            }
        }
    };
}

impl_read_capability!(
    EvmCapability<2>,
    "mfm.evm.capability.read-chain-identity@1",
    ChainIdentity
);
impl_read_capability!(
    EvmCapability<3>,
    "mfm.evm.capability.read-submission-status@1",
    SubmissionStatus
);
impl_read_capability!(
    EvmCapability<6>,
    "mfm.evm.capability.read-anchor@1",
    LatestAnchor
);
impl_read_capability!(
    EvmCapability<7>,
    "mfm.evm.capability.read-balance@1",
    Balance
);

/// Intent for the one-entry wallet nonce reservation effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct NonceReservationIntent {
    /// Exact public target and nonce domain.
    pub target: EvmTransactionTarget,
    /// Stable idempotency key that makes reservation idempotent at the authority.
    pub idempotency_key: String,
}

impl_checked_deserialize!(NonceReservationIntent {
    target: EvmTransactionTarget,
    idempotency_key: String,
});

impl NonceReservationIntent {
    /// Creates one bounded reservation intent.
    fn new(target: EvmTransactionTarget, idempotency_key: String) -> Result<Self, EvmDomainError> {
        target.validate()?;
        if !valid_public_text(&idempotency_key, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            target,
            idempotency_key,
        })
    }

    /// Validates a decoded reservation intent.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        self.target.validate()?;
        valid_public_text(&self.idempotency_key, 256)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

/// Closed wallet nonce reservation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NonceReservationEvidence {
    /// The exact nonce was durably reserved.
    Reserved {
        /// Exact reserved nonce.
        nonce: u64,
    },
    /// The authority rejected reservation before entry.
    Rejected,
    /// The adapter authenticated an integrity block.
    IntegrityBlocked,
}

impl NonceReservationEvidence {
    /// Validates this evidence against its exact reservation intent.
    fn validate_for(&self, intent: &NonceReservationIntent) -> Result<(), EvmDomainError> {
        intent.validate()?;
        match self {
            Self::Reserved { .. } | Self::Rejected | Self::IntegrityBlocked => Ok(()),
        }
    }
}

impl AccessCapabilityContract for EvmCapability<0> {
    type Mode = EffectMode;
    type Intent = NonceReservationIntent;
    type Evidence = NonceReservationEvidence;
    type Facts = NoPriorFacts;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.evm.capability.reserve-wallet-nonce@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }

    fn total_attempt_bound() -> NonZeroU16 {
        NonZeroU16::new(1).expect("constant")
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        evidence
            .validate_for(intent)
            .map_err(|_| mfm_capabilities::CapabilityError::EvidenceBinding)
    }
}

/// Deterministic transaction candidate intent fixed before provider entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct BroadcastIntent {
    /// Exact public chain and sender target.
    pub target: EvmTransactionTarget,
    /// Non-secret idempotency key.
    pub idempotency_key: String,
    /// Deterministic candidate identity.
    pub candidate_id: String,
    /// Exact wallet nonce.
    pub nonce: u64,
    /// Exact sender identity selected by the wallet domain.
    pub sender: String,
    /// Exact wallet nonce domain selected for this operation.
    pub nonce_domain: String,
    /// Canonical unsigned transaction data.
    pub data: Vec<u8>,
    /// Gas limit fixed before provider entry.
    pub gas_limit: u64,
    /// Fee cap fixed before provider entry.
    pub max_fee: String,
    /// Exact public signer key-instance identity.
    pub public_signer_key_instance_ref: ContentRef,
}

impl_checked_deserialize!(BroadcastIntent {
    target: EvmTransactionTarget,
    idempotency_key: String,
    candidate_id: String,
    nonce: u64,
    sender: String,
    nonce_domain: String,
    data: Vec<u8>,
    gas_limit: u64,
    max_fee: String,
    public_signer_key_instance_ref: ContentRef,
});

impl BroadcastIntent {
    fn from_progress(
        request: &EvmSubmissionRequest,
        nonce: u64,
        candidate_id: &str,
    ) -> Result<Self, EvmDomainError> {
        request.validate()?;
        valid_submission_candidate(&request.target, nonce, candidate_id)?;
        let intent = Self {
            target: request.target.clone(),
            idempotency_key: request.idempotency_key.clone(),
            candidate_id: candidate_id.to_owned(),
            nonce,
            sender: request.target.sender.clone(),
            nonce_domain: request.target.nonce_domain.clone(),
            data: request.data.clone(),
            gas_limit: request.gas_limit,
            max_fee: request.max_fee.clone(),
            public_signer_key_instance_ref: request.public_signer_key_instance_ref.clone(),
        };
        intent.validate().map(|_| intent)
    }

    /// Validates decoded broadcast intent without exposing request bytes.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if self.target.validate().is_err()
            || self.target.sender != self.sender
            || self.target.nonce_domain != self.nonce_domain
            || !valid_public_text(&self.idempotency_key, 256)
            || valid_submission_candidate(&self.target, self.nonce, &self.candidate_id).is_err()
            || !valid_public_text(&self.sender, 128)
            || self.sender != self.sender.to_ascii_lowercase()
            || !valid_public_text(&self.nonce_domain, 256)
            || self.data.len() > EVM_TRANSACTION_DATA_LIMIT
            || self.gas_limit == 0
            || !is_decimal_integer(&self.max_fee)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Closed broadcast evidence; a one-entry Effect has no generic retry authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BroadcastEvidence {
    /// Provider returned the authenticated transaction hash.
    Returned {
        /// Provider-asserted transaction hash.
        transaction_hash: String,
    },
    /// Provider rejected before durable entry.
    Rejected,
    /// Request/response integrity failed.
    IntegrityBlocked,
}

impl BroadcastEvidence {
    /// Validates evidence for the exact committed broadcast intent.
    fn validate_for(&self, intent: &BroadcastIntent) -> Result<(), EvmDomainError> {
        intent.validate()?;
        let valid = match self {
            Self::Returned { transaction_hash } => valid_public_text(transaction_hash, 256),
            Self::Rejected | Self::IntegrityBlocked => true,
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

impl AccessCapabilityContract for EvmCapability<1> {
    type Mode = EffectMode;
    type Intent = BroadcastIntent;
    type Evidence = BroadcastEvidence;
    type Facts = NoPriorFacts;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.evm.capability.broadcast-transaction@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }

    fn total_attempt_bound() -> NonZeroU16 {
        NonZeroU16::new(1).expect("constant")
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        intent
            .validate()
            .and_then(|_| evidence.validate_for(intent))
            .map_err(|_| mfm_capabilities::CapabilityError::EvidenceBinding)
    }
}

/// Reusable semantic EVM State selected by its closed family and stage.
///
/// Family 0 is submission: reserve, derive, broadcast, receipt, finality, canonical inclusion,
/// and disposition consolidation. Family 1 is balance: chain identity, initial anchor, asset
/// selection, native balance, token decimals, token balance, anchor confirmation, and
/// consolidation. `K` is used only by the balance family and defaults to the submission progress
/// value so submission registrations stay concise.
pub struct EvmState<const FAMILY: u8, const STAGE: u8, K: MfmValueTrait = EvmSubmissionProgress>(
    PhantomData<fn() -> K>,
);

/// Domain-owned implementation for a callback-free pure EVM State.
pub trait EvmPureState: State {
    /// Consumes one complete State input and produces its typed outcome.
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure>;
}

/// Domain-owned implementation for a capability-bound EVM State.
pub trait EvmAccessState<C: AccessCapabilityContract>: State {
    /// Prepares the exact capability intent from the complete retained input.
    fn prepare(input: &Self::Input) -> Result<C::Intent, EvmDomainError>;
    /// Consumes the retained input only after exact authenticated evidence is accepted.
    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure>;
}

macro_rules! impl_submission_state {
    ($stage:literal, $input:ty, $output:ty, $id:literal, $integrity:expr) => {
        impl State for EvmState<0, $stage> {
            type Input = $input;
            type Output = $output;
            type Failure = EvmSubmissionFailure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| mfm_program::ProgramError::InvalidContract)
            }

            fn integrity_failure(_: &Self::Input) -> Self::Failure {
                $integrity
            }
        }
    };
}

impl_submission_state!(
    0,
    EvmSubmissionRequest,
    EvmSubmissionProgress,
    "mfm.evm.state.reserve-wallet-nonce@1",
    EvmSubmissionFailure::NonceAuthorityUnavailable
);
impl_submission_state!(
    1,
    EvmSubmissionProgress,
    EvmSubmissionProgress,
    "mfm.evm.state.derive-candidate@1",
    EvmSubmissionFailure::NonceLineageDiverged
);
impl_submission_state!(
    2,
    EvmSubmissionProgress,
    EvmSubmissionProgress,
    "mfm.evm.state.broadcast-transaction@1",
    EvmSubmissionFailure::ProviderUnavailable
);
impl_submission_state!(
    3,
    EvmSubmissionProgress,
    EvmSubmissionProgress,
    "mfm.evm.state.read-transaction-receipt@1",
    EvmSubmissionFailure::ProviderUnavailable
);
impl_submission_state!(
    4,
    EvmSubmissionProgress,
    EvmSubmissionProgress,
    "mfm.evm.state.read-finalized-head@1",
    EvmSubmissionFailure::ProviderUnavailable
);
impl_submission_state!(
    5,
    EvmSubmissionProgress,
    EvmSubmissionProgress,
    "mfm.evm.state.read-canonical-inclusion-block@1",
    EvmSubmissionFailure::ProviderUnavailable
);
impl_submission_state!(
    6,
    EvmSubmissionProgress,
    EvmSubmissionOutput,
    "mfm.evm.state.consolidate-execution-disposition@1",
    EvmSubmissionFailure::NonceLineageDiverged
);
fn prepare_reserve_wallet_nonce(
    input: &EvmSubmissionRequest,
) -> Result<NonceReservationIntent, EvmDomainError> {
    input.validate()?;
    NonceReservationIntent::new(input.target.clone(), input.idempotency_key.clone())
}

fn interpret_reserve_wallet_nonce(
    input: EvmSubmissionRequest,
    evidence: &NonceReservationEvidence,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let intent = match prepare_reserve_wallet_nonce(&input) {
        Ok(intent) => intent,
        Err(_) => return failure(EvmSubmissionFailure::NonceLineageDiverged),
    };
    if evidence.validate_for(&intent).is_err() {
        return failure(EvmSubmissionFailure::NonceAuthorityUnavailable);
    }
    match evidence {
        NonceReservationEvidence::Reserved { nonce } => {
            submission_progress(input, SubmissionPhase::Reserved { nonce: *nonce })
        }
        NonceReservationEvidence::Rejected => {
            failure(EvmSubmissionFailure::NonceAuthorityUnavailable)
        }
        NonceReservationEvidence::IntegrityBlocked => {
            failure(EvmSubmissionFailure::NonceAuthorityUnavailable)
        }
    }
}

fn derive_submission_candidate(
    input: EvmSubmissionProgress,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let EvmSubmissionProgress {
        request,
        phase: SubmissionPhase::Reserved { nonce },
    } = input
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    let candidate_id = submission_candidate_id(&request.target, nonce);
    submission_progress(
        request,
        SubmissionPhase::Candidate {
            nonce,
            candidate_id,
        },
    )
}

fn prepare_broadcast_transaction(
    input: &EvmSubmissionProgress,
) -> Result<BroadcastIntent, EvmDomainError> {
    match (&input.request, &input.phase) {
        (
            request,
            SubmissionPhase::Candidate {
                nonce,
                candidate_id,
            },
        ) => BroadcastIntent::from_progress(request, *nonce, candidate_id),
        _ => Err(EvmDomainError::InvalidValue),
    }
}

fn interpret_broadcast_transaction(
    input: EvmSubmissionProgress,
    evidence: &BroadcastEvidence,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let intent = match prepare_broadcast_transaction(&input) {
        Ok(intent) => intent,
        Err(_) => return failure(EvmSubmissionFailure::NonceLineageDiverged),
    };
    if evidence.validate_for(&intent).is_err() {
        return failure(EvmSubmissionFailure::ProviderUnavailable);
    }
    let EvmSubmissionProgress {
        request,
        phase: SubmissionPhase::Candidate {
            nonce,
            candidate_id,
        },
    } = input
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    match evidence {
        BroadcastEvidence::Returned { transaction_hash } => submission_progress(
            request,
            SubmissionPhase::Broadcast {
                nonce,
                candidate_id,
                transaction_hash: transaction_hash.clone(),
            },
        ),
        BroadcastEvidence::Rejected => failure(EvmSubmissionFailure::DestinationRejected),
        BroadcastEvidence::IntegrityBlocked => failure(EvmSubmissionFailure::ProviderUnavailable),
    }
}

fn prepare_transaction_receipt(
    input: &EvmSubmissionProgress,
) -> Result<EvmReadIntent, EvmDomainError> {
    match (&input.request, &input.phase) {
        (
            request,
            SubmissionPhase::Broadcast {
                transaction_hash, ..
            },
        ) => EvmReadIntent::new(
            "mfm.evm.read-transaction-receipt@1".to_owned(),
            request.target.chain_id,
            EvmReadSubject::TransactionReceipt {
                transaction_hash: transaction_hash.clone(),
            },
        ),
        _ => Err(EvmDomainError::InvalidValue),
    }
}

fn interpret_transaction_receipt(
    input: EvmSubmissionProgress,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let expected = match prepare_transaction_receipt(&input) {
        Ok(intent) => intent,
        Err(_) => return failure(EvmSubmissionFailure::NonceLineageDiverged),
    };
    let EvmSubmissionProgress {
        request,
        phase:
            SubmissionPhase::Broadcast {
                nonce,
                candidate_id,
                transaction_hash: expected_hash,
            },
    } = input
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    match read_returned(evidence, &expected) {
        Some(EvmReadValue::Receipt {
            transaction_hash,
            execution_disposition,
            inclusion_block_number,
            inclusion_block_hash,
        }) if transaction_hash == &expected_hash => {
            match (
                EvmExecutionDisposition::from_code(execution_disposition),
                read_anchor(inclusion_block_number, inclusion_block_hash),
            ) {
                (Some(execution_disposition), Some(inclusion_block)) => submission_progress(
                    request,
                    SubmissionPhase::Receipt {
                        nonce,
                        candidate_id,
                        transaction_hash: expected_hash,
                        execution_disposition,
                        inclusion_block,
                    },
                ),
                _ => failure(EvmSubmissionFailure::ProviderUnavailable),
            }
        }
        Some(_) => failure(EvmSubmissionFailure::ProviderUnavailable),
        None => failure(EvmSubmissionFailure::ProviderUnavailable),
    }
}

fn prepare_finalized_head(input: &EvmSubmissionProgress) -> Result<EvmReadIntent, EvmDomainError> {
    match (&input.request, &input.phase) {
        (request, SubmissionPhase::Receipt { .. }) => EvmReadIntent::new(
            "mfm.evm.read-finalized-head@1".to_owned(),
            request.target.chain_id,
            EvmReadSubject::FinalizedHead,
        ),
        _ => Err(EvmDomainError::InvalidValue),
    }
}

fn interpret_finalized_head(
    input: EvmSubmissionProgress,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let expected = match prepare_finalized_head(&input) {
        Ok(intent) => intent,
        Err(_) => return failure(EvmSubmissionFailure::NonceLineageDiverged),
    };
    let EvmSubmissionProgress {
        request,
        phase:
            SubmissionPhase::Receipt {
                nonce,
                candidate_id,
                transaction_hash,
                execution_disposition,
                inclusion_block,
            },
    } = input
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    match read_returned(evidence, &expected) {
        Some(EvmReadValue::FinalizedHead { number })
            if decimal_at_least(number, &inclusion_block.number) =>
        {
            submission_progress(
                request,
                SubmissionPhase::Finalized {
                    nonce,
                    candidate_id,
                    transaction_hash,
                    execution_disposition,
                    inclusion_block,
                    finalized_head_number: number.clone(),
                },
            )
        }
        _ => failure(EvmSubmissionFailure::ProviderUnavailable),
    }
}

fn prepare_canonical_inclusion_block(
    input: &EvmSubmissionProgress,
) -> Result<EvmReadIntent, EvmDomainError> {
    match (&input.request, &input.phase) {
        (
            request,
            SubmissionPhase::Finalized {
                inclusion_block, ..
            },
        ) => EvmReadIntent::new(
            "mfm.evm.read-canonical-inclusion-block@1".to_owned(),
            request.target.chain_id,
            EvmReadSubject::CanonicalInclusionBlock {
                number: inclusion_block.number.clone(),
            },
        ),
        _ => Err(EvmDomainError::InvalidValue),
    }
}

fn interpret_canonical_inclusion_block(
    input: EvmSubmissionProgress,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    let expected = match prepare_canonical_inclusion_block(&input) {
        Ok(intent) => intent,
        Err(_) => return failure(EvmSubmissionFailure::NonceLineageDiverged),
    };
    let EvmSubmissionProgress {
        request,
        phase:
            SubmissionPhase::Finalized {
                nonce,
                candidate_id,
                transaction_hash,
                execution_disposition,
                inclusion_block,
                finalized_head_number,
            },
    } = input
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    match read_returned(evidence, &expected) {
        Some(EvmReadValue::CanonicalBlock { number, hash }) => match read_anchor(number, hash) {
            Some(canonical_inclusion_block) if canonical_inclusion_block == inclusion_block => {
                submission_progress(
                    request,
                    SubmissionPhase::Canonical {
                        nonce,
                        candidate_id,
                        transaction_hash,
                        execution_disposition,
                        inclusion_block,
                        finalized_head_number,
                        canonical_inclusion_block,
                    },
                )
            }
            _ => failure(EvmSubmissionFailure::ProviderUnavailable),
        },
        _ => failure(EvmSubmissionFailure::ProviderUnavailable),
    }
}

fn consolidate_execution_disposition(
    input: EvmSubmissionProgress,
) -> ProposedStateOutcome<EvmSubmissionOutput, EvmSubmissionFailure> {
    if input.validate().is_err() {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    }
    let SubmissionPhase::Canonical {
        execution_disposition,
        ..
    } = input.phase
    else {
        return failure(EvmSubmissionFailure::NonceLineageDiverged);
    };
    success(EvmSubmissionOutput::new(execution_disposition))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmBalanceFailureStage {
    CheckChainIdentity,
    ReadInitialAnchor,
    SelectAsset,
    ReadNativeBalance,
    ReadTokenDecimals,
    ReadTokenBalance,
    ConfirmAnchor,
    Consolidate,
}

impl EvmBalanceFailureStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CheckChainIdentity => "check_chain_identity",
            Self::ReadInitialAnchor => "read_initial_anchor",
            Self::SelectAsset => "select_asset",
            Self::ReadNativeBalance => "read_native_balance",
            Self::ReadTokenDecimals => "read_token_decimals",
            Self::ReadTokenBalance => "read_token_balance",
            Self::ConfirmAnchor => "confirm_anchor",
            Self::Consolidate => "consolidate",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmBalanceFailureCode {
    ObservationUnavailable,
    CollectionInvalid,
    IntegrityBlocked,
}

impl EvmBalanceFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ObservationUnavailable => "observation_unavailable",
            Self::CollectionInvalid => "collection_invalid",
            Self::IntegrityBlocked => "integrity_blocked",
        }
    }
}

/// EVM-owned typed failure that leaves Portfolio semantics to its mapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmBalanceFailure {
    /// A bounded source-stage failure.
    SourceUnavailable {
        /// Stage that could not be completed.
        stage: String,
        /// Planner-owned collection ordinal retained for the caller's failure mapper.
        collection_ordinal: u32,
        /// Stable redacted failure code.
        code: String,
    },
    /// The adapter accepted a capability integrity block.
    IntegrityBlocked {
        /// Stage whose capability accepted the integrity block.
        stage: String,
        /// Planner-owned collection ordinal retained for the caller's failure mapper.
        collection_ordinal: u32,
        /// Stable redacted integrity code.
        code: String,
    },
}

impl<'de> Deserialize<'de> for EvmBalanceFailure {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields
        )]
        enum Wire {
            SourceUnavailable {
                stage: EvmBalanceFailureStage,
                collection_ordinal: u32,
                code: EvmBalanceFailureCode,
            },
            IntegrityBlocked {
                stage: EvmBalanceFailureStage,
                collection_ordinal: u32,
                code: EvmBalanceFailureCode,
            },
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::SourceUnavailable {
                stage,
                collection_ordinal,
                code:
                    code @ (EvmBalanceFailureCode::ObservationUnavailable
                    | EvmBalanceFailureCode::CollectionInvalid),
            } => Self::SourceUnavailable {
                stage: stage.as_str().to_owned(),
                collection_ordinal,
                code: code.as_str().to_owned(),
            },
            Wire::IntegrityBlocked {
                stage,
                collection_ordinal,
                code: EvmBalanceFailureCode::IntegrityBlocked,
            } => Self::IntegrityBlocked {
                stage: stage.as_str().to_owned(),
                collection_ordinal,
                code: EvmBalanceFailureCode::IntegrityBlocked.as_str().to_owned(),
            },
            _ => return Err(de::Error::custom(EvmDomainError::InvalidValue)),
        };
        Ok(value)
    }
}

impl FailureValue for EvmBalanceFailure {
    fn integrity_blocked() -> Self {
        Self::IntegrityBlocked {
            stage: EvmBalanceFailureStage::Consolidate.as_str().to_owned(),
            collection_ordinal: 0,
            code: EvmBalanceFailureCode::IntegrityBlocked.as_str().to_owned(),
        }
    }
}

fn balance_integrity_failure<K: MfmValueTrait>(
    context: &EvmBalanceContext<K>,
    stage: EvmBalanceFailureStage,
) -> EvmBalanceFailure {
    EvmBalanceFailure::IntegrityBlocked {
        stage: stage.as_str().to_owned(),
        collection_ordinal: context.metadata.collection_ordinal,
        code: EvmBalanceFailureCode::IntegrityBlocked.as_str().to_owned(),
    }
}

macro_rules! impl_balance_state {
    ($stage:literal, $input:ty, $output:ty, $id:literal, $failure_stage:expr) => {
        impl<K: MfmValueTrait> State for EvmState<1, $stage, K> {
            type Input = $input;
            type Output = $output;
            type Failure = EvmBalanceFailure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| mfm_program::ProgramError::InvalidContract)
            }

            fn integrity_failure(input: &Self::Input) -> Self::Failure {
                balance_integrity_failure(input, $failure_stage)
            }
        }
    };
}

impl_balance_state!(
    0,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.check-chain-identity@1",
    EvmBalanceFailureStage::CheckChainIdentity
);
impl_balance_state!(
    1,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-initial-anchor@1",
    EvmBalanceFailureStage::ReadInitialAnchor
);
impl_balance_state!(
    2,
    EvmBalanceContext<K>,
    EvmBalanceAsset<K>,
    "mfm.evm.state.select-asset@1",
    EvmBalanceFailureStage::SelectAsset
);
impl_balance_state!(
    3,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-native-balance@1",
    EvmBalanceFailureStage::ReadNativeBalance
);
impl_balance_state!(
    4,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-token-decimals@1",
    EvmBalanceFailureStage::ReadTokenDecimals
);
impl_balance_state!(
    5,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-token-balance@1",
    EvmBalanceFailureStage::ReadTokenBalance
);
impl_balance_state!(
    6,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.confirm-balance-anchor@1",
    EvmBalanceFailureStage::ConfirmAnchor
);
impl_balance_state!(
    7,
    EvmBalanceContext<K>,
    EvmBalanceCollectionCompletion<K>,
    "mfm.evm.state.consolidate-balance-collection@1",
    EvmBalanceFailureStage::Consolidate
);
fn prepare_check_chain_identity<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::CheckChainIdentity { source } = &input.work else {
        return Err(EvmDomainError::InvalidValue);
    };
    EvmReadIntent::new(
        "mfm.evm.read-chain-identity@1".to_owned(),
        source.chain_id,
        EvmReadSubject::ChainIdentity,
    )
}

fn interpret_check_chain_identity<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_check_chain_identity(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity),
    };
    let EvmBalanceWork::CheckChainIdentity { source } = &input.work else {
        return balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::ChainId(chain_id)) if *chain_id == source.chain_id => {
            let work = EvmBalanceWork::ReadInitialAnchor {
                source: source.clone(),
                checked_chain_id: *chain_id,
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::CheckChainIdentity)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity),
    }
}

fn prepare_read_initial_anchor<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ReadInitialAnchor {
        source,
        checked_chain_id,
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    EvmReadIntent::new(
        "mfm.evm.read-initial-anchor@1".to_owned(),
        *checked_chain_id,
        EvmReadSubject::InitialAnchor,
    )
    .and_then(|intent| {
        (source.chain_id == *checked_chain_id)
            .then_some(intent)
            .ok_or(EvmDomainError::InvalidValue)
    })
}

fn interpret_read_initial_anchor<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_read_initial_anchor(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
    };
    let EvmBalanceWork::ReadInitialAnchor {
        source,
        checked_chain_id,
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::Anchor { number, hash }) => match read_anchor(number, hash) {
            Some(initial_anchor) => {
                let work = EvmBalanceWork::SelectAsset {
                    source: source.clone(),
                    checked_chain_id: *checked_chain_id,
                    initial_anchor,
                };
                advance_balance_context(input, work, EvmBalanceFailureStage::ReadInitialAnchor)
            }
            None => balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
        },
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
    }
}

fn select_balance_asset<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
) -> ProposedStateOutcome<EvmBalanceAsset<K>, EvmBalanceFailure> {
    let native = matches!(input.work, EvmBalanceWork::SelectAsset { ref source, .. } if source.token.is_none());
    let token = matches!(input.work, EvmBalanceWork::SelectAsset { ref source, .. } if source.token.is_some());
    if native {
        success(EvmBalanceAsset::Native(input))
    } else if token {
        success(EvmBalanceAsset::Token(input))
    } else {
        balance_failure(&input, EvmBalanceFailureStage::SelectAsset)
    }
}

fn prepare_read_native_balance<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadNativeBalance {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    if source.token.is_some() {
        return Err(EvmDomainError::InvalidValue);
    }
    EvmReadIntent::new(
        "mfm.evm.read-native-balance@1".to_owned(),
        *checked_chain_id,
        EvmReadSubject::NativeBalance {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_native_balance<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_read_native_balance(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance),
    };
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadNativeBalance {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::RawUnits(raw_balance)) => {
            let work = EvmBalanceWork::ConfirmAnchor {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                source_decimals: input.request.decimals,
                raw_balance: raw_balance.clone(),
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadNativeBalance)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance),
    }
}

fn prepare_read_token_decimals<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadTokenDecimals {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    if source.token.is_none() {
        return Err(EvmDomainError::InvalidValue);
    }
    EvmReadIntent::new(
        "mfm.evm.read-token-decimals@1".to_owned(),
        *checked_chain_id,
        EvmReadSubject::TokenDecimals {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_token_decimals<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_read_token_decimals(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals),
    };
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadTokenDecimals {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::TokenDecimals(token_decimals)) if *token_decimals <= 30 => {
            let work = EvmBalanceWork::ReadTokenBalance {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                token_decimals: *token_decimals,
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadTokenDecimals)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals),
    }
}

fn prepare_read_token_balance<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ReadTokenBalance {
        source,
        checked_chain_id,
        initial_anchor,
        ..
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    EvmReadIntent::new(
        "mfm.evm.read-token-balance@1".to_owned(),
        *checked_chain_id,
        EvmReadSubject::TokenBalance {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_token_balance<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_read_token_balance(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance),
    };
    let EvmBalanceWork::ReadTokenBalance {
        source,
        checked_chain_id,
        initial_anchor,
        token_decimals,
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::RawUnits(raw_balance)) => {
            let work = EvmBalanceWork::ConfirmAnchor {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                source_decimals: *token_decimals,
                raw_balance: raw_balance.clone(),
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadTokenBalance)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance),
    }
}

fn prepare_confirm_balance_anchor<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ConfirmAnchor {
        source,
        checked_chain_id,
        initial_anchor,
        ..
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    EvmReadIntent::new(
        "mfm.evm.confirm-balance-anchor@1".to_owned(),
        *checked_chain_id,
        EvmReadSubject::ConfirmAnchor {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_confirm_balance_anchor<K: MfmValueTrait>(
    mut input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    let intent = match prepare_confirm_balance_anchor(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor),
    };
    let EvmBalanceWork::ConfirmAnchor {
        source,
        initial_anchor,
        source_decimals,
        raw_balance,
        ..
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    let Some(EvmReadValue::Anchor { number, hash }) = read_returned(evidence, &intent) else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    let Some(anchor) = read_anchor(number, hash) else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    if &anchor != initial_anchor {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    }
    if input
        .request
        .scale_units(raw_balance, *source_decimals)
        .is_none()
    {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    }
    input.completed.push(EvmBalanceResult {
        source: source.clone(),
        decimals: *source_decimals,
        raw_units: raw_balance.clone(),
        anchor: initial_anchor.clone(),
    });
    let work = match input.request.sources.get(input.completed.len()).cloned() {
        Some(source) => EvmBalanceWork::CheckChainIdentity { source },
        None => EvmBalanceWork::Complete,
    };
    advance_balance_context(input, work, EvmBalanceFailureStage::ConfirmAnchor)
}

fn consolidate_balance_collection<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
) -> ProposedStateOutcome<EvmBalanceCollectionCompletion<K>, EvmBalanceFailure> {
    if !matches!(input.work, EvmBalanceWork::Complete) || input.validate().is_err() {
        return balance_failure(&input, EvmBalanceFailureStage::Consolidate);
    }
    let total_scaled = match completed_total_scaled(&input.request, &input.completed) {
        Some(total) => total,
        None => return balance_failure(&input, EvmBalanceFailureStage::Consolidate),
    };
    let collection_ordinal = input.metadata.collection_ordinal;
    let chain_id = input.request.sources[0].chain_id;
    let anchor = input.completed[0].anchor.clone();
    let result = EvmBalanceCollectionResult {
        chain_id,
        anchor,
        balances: input
            .completed
            .into_iter()
            .map(EvmCollectedBalance::from_result)
            .collect(),
        total_scaled,
    };
    match EvmBalanceCollectionCompletion::new(input.caller_continuation, collection_ordinal, result)
    {
        Ok(output) => success(output),
        Err(_) => failure(EvmBalanceFailure::SourceUnavailable {
            stage: EvmBalanceFailureStage::Consolidate.as_str().to_owned(),
            collection_ordinal,
            code: EvmBalanceFailureCode::CollectionInvalid.as_str().to_owned(),
        }),
    }
}

macro_rules! impl_submission_access {
    ($stage:literal, $capability:ty, $prepare:path, $interpret:path) => {
        impl EvmAccessState<$capability> for EvmState<0, $stage> {
            fn prepare(
                input: &Self::Input,
            ) -> Result<<$capability as AccessCapabilityContract>::Intent, EvmDomainError> {
                $prepare(input)
            }

            fn interpret(
                input: Self::Input,
                evidence: &<$capability as AccessCapabilityContract>::Evidence,
            ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $interpret(input, evidence)
            }
        }
    };
}

macro_rules! impl_balance_access {
    ($stage:literal, $capability:ty, $prepare:path, $interpret:path) => {
        impl<K: MfmValueTrait> EvmAccessState<$capability> for EvmState<1, $stage, K> {
            fn prepare(
                input: &Self::Input,
            ) -> Result<<$capability as AccessCapabilityContract>::Intent, EvmDomainError> {
                $prepare(input)
            }

            fn interpret(
                input: Self::Input,
                evidence: &<$capability as AccessCapabilityContract>::Evidence,
            ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $interpret(input, evidence)
            }
        }
    };
}

macro_rules! impl_evm_pure {
    ($state:ty, $evaluate:path) => {
        impl EvmPureState for $state {
            fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $evaluate(input)
            }
        }
    };
}

impl_submission_access!(
    0,
    EvmCapability<0>,
    prepare_reserve_wallet_nonce,
    interpret_reserve_wallet_nonce
);
impl_submission_access!(
    2,
    EvmCapability<1>,
    prepare_broadcast_transaction,
    interpret_broadcast_transaction
);
impl_submission_access!(
    3,
    EvmCapability<3>,
    prepare_transaction_receipt,
    interpret_transaction_receipt
);
impl_submission_access!(
    4,
    EvmCapability<3>,
    prepare_finalized_head,
    interpret_finalized_head
);
impl_submission_access!(
    5,
    EvmCapability<3>,
    prepare_canonical_inclusion_block,
    interpret_canonical_inclusion_block
);
impl_evm_pure!(EvmState<0, 1>, derive_submission_candidate);
impl_evm_pure!(EvmState<0, 6>, consolidate_execution_disposition);

impl_balance_access!(
    0,
    EvmCapability<2>,
    prepare_check_chain_identity,
    interpret_check_chain_identity
);
impl_balance_access!(
    1,
    EvmCapability<6>,
    prepare_read_initial_anchor,
    interpret_read_initial_anchor
);
impl_balance_access!(
    3,
    EvmCapability<7>,
    prepare_read_native_balance,
    interpret_read_native_balance
);
impl_balance_access!(
    4,
    EvmCapability<7>,
    prepare_read_token_decimals,
    interpret_read_token_decimals
);
impl_balance_access!(
    5,
    EvmCapability<7>,
    prepare_read_token_balance,
    interpret_read_token_balance
);
impl_balance_access!(
    6,
    EvmCapability<6>,
    prepare_confirm_balance_anchor,
    interpret_confirm_balance_anchor
);
impl<K: MfmValueTrait> EvmPureState for EvmState<1, 2, K> {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        select_balance_asset(input)
    }
}
impl<K: MfmValueTrait> EvmPureState for EvmState<1, 7, K> {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        consolidate_balance_collection(input)
    }
}

fn submission_progress(
    request: EvmSubmissionRequest,
    phase: SubmissionPhase,
) -> ProposedStateOutcome<EvmSubmissionProgress, EvmSubmissionFailure> {
    match EvmSubmissionProgress::new(request, phase) {
        Ok(output) => success(output),
        Err(_) => failure(EvmSubmissionFailure::NonceLineageDiverged),
    }
}

fn balance_failure<K: MfmValueTrait, O>(
    context: &EvmBalanceContext<K>,
    stage: EvmBalanceFailureStage,
) -> ProposedStateOutcome<O, EvmBalanceFailure> {
    failure(EvmBalanceFailure::SourceUnavailable {
        stage: stage.as_str().to_owned(),
        collection_ordinal: context.metadata.collection_ordinal,
        code: EvmBalanceFailureCode::ObservationUnavailable
            .as_str()
            .to_owned(),
    })
}

fn advance_balance_context<K: MfmValueTrait>(
    mut context: EvmBalanceContext<K>,
    work: EvmBalanceWork,
    stage: EvmBalanceFailureStage,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    context.work = work;
    if context.validate().is_ok() {
        success(context)
    } else {
        balance_failure(&context, stage)
    }
}

/// Exact live bindings required by the EVM submission Program.
#[derive(Debug, Clone)]
pub struct EvmSubmissionBindings {
    target: EvmTransactionTarget,
    descriptors: [BindingDescriptor; 5],
}

impl EvmSubmissionBindings {
    /// Constructs one complete immutable binding set in reserve, broadcast, receipt, finality,
    /// and canonical-block order.
    pub fn new(
        target: EvmTransactionTarget,
        descriptors: [BindingDescriptor; 5],
    ) -> Result<Self, EvmDomainError> {
        target.validate()?;
        let [reserve_nonce, broadcast, receipt, finalized_head, canonical_inclusion_block] =
            &descriptors;
        validate_access_binding::<EvmState<0, 0>, EvmCapability<0>>(reserve_nonce)?;
        validate_access_binding::<EvmState<0, 2>, EvmCapability<1>>(broadcast)?;
        validate_access_binding::<EvmState<0, 3>, EvmCapability<3>>(receipt)?;
        validate_access_binding::<EvmState<0, 4>, EvmCapability<3>>(finalized_head)?;
        validate_access_binding::<EvmState<0, 5>, EvmCapability<3>>(canonical_inclusion_block)?;
        if reserve_nonce.effect_domain().is_none()
            || reserve_nonce.public_signer_key_instance_ref().is_some()
            || broadcast.effect_domain().is_none()
            || broadcast.public_signer_key_instance_ref().is_none()
            || [receipt, finalized_head, canonical_inclusion_block]
                .iter()
                .any(|binding| {
                    binding.effect_domain().is_some()
                        || binding.public_signer_key_instance_ref().is_some()
                })
            || descriptors
                .iter()
                .skip(1)
                .any(|binding| binding.physical_target_ref() != reserve_nonce.physical_target_ref())
        {
            return Err(EvmDomainError::Program);
        }
        Ok(Self {
            target,
            descriptors,
        })
    }

    /// Returns the deployment-selected target for this exact binding set.
    pub const fn target(&self) -> &EvmTransactionTarget {
        &self.target
    }

    /// Returns descriptors in the constructor's fixed semantic order.
    pub const fn descriptors(&self) -> &[BindingDescriptor; 5] {
        &self.descriptors
    }

    fn source_refs(&self) -> Vec<mfm_ids::ContentRef> {
        vec![self.descriptors[0].physical_target_ref().clone()]
    }

    /// Returns the exact public signer identity selected for broadcast planning.
    fn public_signer_key_instance_ref(&self) -> Result<ContentRef, EvmDomainError> {
        self.descriptors[1]
            .public_signer_key_instance_ref()
            .cloned()
            .ok_or(EvmDomainError::Program)
    }
}

/// Exact live bindings required by one reusable EVM balance fragment route.
#[derive(Debug, Clone)]
pub struct EvmBalanceBindings {
    /// Chain id selected by this route.
    pub chain_id: u64,
    descriptors: [BindingDescriptor; 6],
}

impl EvmBalanceBindings {
    /// Constructs one complete immutable binding set in the fragment's State order.
    pub fn new(chain_id: u64, descriptors: [BindingDescriptor; 6]) -> Result<Self, EvmDomainError> {
        if chain_id == 0 {
            return Err(EvmDomainError::Program);
        }
        let [check_chain_identity, read_initial_anchor, read_native_balance, read_token_decimals, read_token_balance, confirm_anchor] =
            &descriptors;
        validate_access_binding::<EvmState<1, 0>, EvmCapability<2>>(check_chain_identity)?;
        validate_access_binding::<EvmState<1, 1>, EvmCapability<6>>(read_initial_anchor)?;
        validate_access_binding::<EvmState<1, 3>, EvmCapability<7>>(read_native_balance)?;
        validate_access_binding::<EvmState<1, 4>, EvmCapability<7>>(read_token_decimals)?;
        validate_access_binding::<EvmState<1, 5>, EvmCapability<7>>(read_token_balance)?;
        validate_access_binding::<EvmState<1, 6>, EvmCapability<6>>(confirm_anchor)?;
        if descriptors.iter().any(|binding| {
            binding.effect_domain().is_some() || binding.public_signer_key_instance_ref().is_some()
        }) || descriptors.iter().skip(1).any(|binding| {
            binding.physical_target_ref() != check_chain_identity.physical_target_ref()
        }) {
            return Err(EvmDomainError::Program);
        }
        Ok(Self {
            chain_id,
            descriptors,
        })
    }

    /// Returns the one physical route identity shared by all reads in this fragment.
    pub fn route_ref(&self) -> mfm_ids::ContentRef {
        self.descriptors[0].physical_target_ref().clone()
    }

    /// Returns descriptors in the constructor's fixed semantic order.
    pub const fn descriptors(&self) -> &[BindingDescriptor; 6] {
        &self.descriptors
    }
}

/// Authors the exact EVM submission Program for one planned request shape.
fn submission_program(bindings: &EvmSubmissionBindings) -> Result<ProgramDocument, EvmDomainError> {
    let [reserve_nonce, broadcast, receipt, finalized_head, canonical_inclusion_block] =
        bindings.descriptors();
    let request =
        nominal_contract_ref::<EvmSubmissionRequest>().map_err(|_| EvmDomainError::Program)?;
    let progress =
        nominal_contract_ref::<EvmSubmissionProgress>().map_err(|_| EvmDomainError::Program)?;
    let output =
        nominal_contract_ref::<EvmSubmissionOutput>().map_err(|_| EvmDomainError::Program)?;
    let failure =
        nominal_contract_ref::<EvmSubmissionFailure>().map_err(|_| EvmDomainError::Program)?;
    let declarations = vec![
        Declaration::State(Box::new(access_state::<EvmState<0, 0>, EvmCapability<0>>(
            0,
            request.clone(),
            progress.clone(),
            failure.clone(),
            address(1)?,
            reserve_nonce,
        )?)),
        Declaration::State(Box::new(pure_state::<EvmState<0, 1>>(
            1,
            progress.clone(),
            progress.clone(),
            failure.clone(),
            Some(address(2)?),
        )?)),
        Declaration::State(Box::new(access_state::<EvmState<0, 2>, EvmCapability<1>>(
            2,
            progress.clone(),
            progress.clone(),
            failure.clone(),
            address(3)?,
            broadcast,
        )?)),
        Declaration::State(Box::new(access_state::<EvmState<0, 3>, EvmCapability<3>>(
            3,
            progress.clone(),
            progress.clone(),
            failure.clone(),
            address(4)?,
            receipt,
        )?)),
        Declaration::State(Box::new(access_state::<EvmState<0, 4>, EvmCapability<3>>(
            4,
            progress.clone(),
            progress.clone(),
            failure.clone(),
            address(5)?,
            finalized_head,
        )?)),
        Declaration::State(Box::new(access_state::<EvmState<0, 5>, EvmCapability<3>>(
            5,
            progress.clone(),
            progress.clone(),
            failure.clone(),
            address(6)?,
            canonical_inclusion_block,
        )?)),
        Declaration::State(Box::new(pure_state::<EvmState<0, 6>>(
            6,
            progress,
            output.clone(),
            failure,
            None,
        )?)),
    ];
    ProgramDocument::new(
        StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
            .map_err(|_| EvmDomainError::Program)?,
        output,
        request,
        declarations,
    )
    .map_err(|_| EvmDomainError::Program)
}

/// One indivisible domain-planned EVM submission admission product.
pub struct EvmSubmissionPlan {
    input: EvmSubmissionRequest,
    program: ProgramDocument,
    source_refs: Vec<ContentRef>,
}

impl EvmSubmissionPlan {
    /// Consumes the plan into the only values App needs to qualify and admit it.
    pub fn into_parts(self) -> (EvmSubmissionRequest, ProgramDocument, Vec<ContentRef>) {
        (self.input, self.program, self.source_refs)
    }
}

/// Selects one trusted EVM route and authors its final submission Program.
pub fn plan_submission(
    selector: EvmSubmissionSelector,
    config: &EvmConfig,
    bindings: &[EvmSubmissionBindings],
) -> Result<EvmSubmissionPlan, EvmDomainError> {
    validate_evm_config(config)?;
    selector.validate()?;
    let mut routes = config
        .submission_routes
        .iter()
        .filter(|route| route.target == selector.target);
    let route = routes.next().ok_or(EvmDomainError::InvalidValue)?;
    if routes.next().is_some() {
        return Err(EvmDomainError::InvalidValue);
    }
    let binding = submission_binding_for_route(route, bindings)?;
    let input =
        EvmSubmissionRequest::planned(selector, route.public_signer_key_instance_ref.clone())?;
    Ok(EvmSubmissionPlan {
        input,
        program: submission_program(binding)?,
        source_refs: binding.source_refs(),
    })
}

/// Returns the final EVM submission Programs needed to validate a trusted composition closure.
///
/// The domain owns the route-to-binding relation, so App validates only the resulting Programs
/// against its one Runtime rather than inspecting EVM configuration or binding internals.
pub fn submission_closure_documents(
    config: &EvmConfig,
    bindings: &[EvmSubmissionBindings],
) -> Result<Vec<ProgramDocument>, EvmDomainError> {
    validate_evm_config(config)?;
    if bindings.len() != config.submission_routes.len() {
        return Err(EvmDomainError::Program);
    }
    config
        .submission_routes
        .iter()
        .map(|route| submission_binding_for_route(route, bindings).and_then(submission_program))
        .collect()
}

fn submission_binding_for_route<'a>(
    route: &EvmSubmissionRoute,
    bindings: &'a [EvmSubmissionBindings],
) -> Result<&'a EvmSubmissionBindings, EvmDomainError> {
    let mut matches = bindings
        .iter()
        .filter(|binding| binding.target == route.target);
    let binding = matches.next().ok_or(EvmDomainError::InvalidValue)?;
    if matches.next().is_some()
        || binding.public_signer_key_instance_ref()? != route.public_signer_key_instance_ref
    {
        return Err(EvmDomainError::InvalidValue);
    }
    Ok(binding)
}

/// Appends one unrolled, reusable balance fragment to a Program declaration list.
pub fn append_balance_fragment<K: MfmValueTrait>(
    declarations: &mut Vec<Declaration>,
    start_ordinal: u32,
    bindings: &EvmBalanceBindings,
    failure_next: SequentialControlAddress,
    completion_next: SequentialControlAddress,
) -> Result<(), EvmDomainError> {
    let [check_chain_identity, read_initial_anchor, read_native_balance, read_token_decimals, read_token_balance, confirm_anchor] =
        bindings.descriptors();
    let context =
        nominal_contract_ref::<EvmBalanceContext<K>>().map_err(|_| EvmDomainError::Program)?;
    let asset =
        nominal_contract_ref::<EvmBalanceAsset<K>>().map_err(|_| EvmDomainError::Program)?;
    let completion = nominal_contract_ref::<EvmBalanceCollectionCompletion<K>>()
        .map_err(|_| EvmDomainError::Program)?;
    let failure =
        nominal_contract_ref::<EvmBalanceFailure>().map_err(|_| EvmDomainError::Program)?;
    let initial = address(start_ordinal + 1)?;
    let select = address(start_ordinal + 2)?;
    let selector = address(start_ordinal + 3)?;
    let native = address(start_ordinal + 4)?;
    let decimals = address(start_ordinal + 5)?;
    let token = address(start_ordinal + 6)?;
    let confirm = address(start_ordinal + 7)?;
    let consolidate = address(start_ordinal + 8)?;
    declarations.extend([
        state_with_failure(
            access_state::<EvmState<1, 0, K>, EvmCapability<2>>(
                start_ordinal,
                context.clone(),
                context.clone(),
                failure.clone(),
                initial.clone(),
                check_chain_identity,
            ),
            &failure_next,
        )?,
        state_with_failure(
            access_state::<EvmState<1, 1, K>, EvmCapability<6>>(
                start_ordinal + 1,
                context.clone(),
                context.clone(),
                failure.clone(),
                select.clone(),
                read_initial_anchor,
            ),
            &failure_next,
        )?,
        state_with_failure(
            pure_state::<EvmState<1, 2, K>>(
                start_ordinal + 2,
                context.clone(),
                asset.clone(),
                failure.clone(),
                Some(selector.clone()),
            ),
            &failure_next,
        )?,
        Declaration::Match(
            MatchDeclaration::new(
                selector,
                asset,
                vec![
                    MatchVariant::new(
                        StableId::new("native").map_err(|_| EvmDomainError::Program)?,
                        context.clone(),
                        context.clone(),
                        native.clone(),
                    ),
                    MatchVariant::new(
                        StableId::new("token").map_err(|_| EvmDomainError::Program)?,
                        context.clone(),
                        context.clone(),
                        decimals.clone(),
                    ),
                ],
            )
            .map_err(|_| EvmDomainError::Program)?,
        ),
        state_with_failure(
            access_state::<EvmState<1, 3, K>, EvmCapability<7>>(
                start_ordinal + 4,
                context.clone(),
                context.clone(),
                failure.clone(),
                confirm.clone(),
                read_native_balance,
            ),
            &failure_next,
        )?,
        state_with_failure(
            access_state::<EvmState<1, 4, K>, EvmCapability<7>>(
                start_ordinal + 5,
                context.clone(),
                context.clone(),
                failure.clone(),
                token.clone(),
                read_token_decimals,
            ),
            &failure_next,
        )?,
        state_with_failure(
            access_state::<EvmState<1, 5, K>, EvmCapability<7>>(
                start_ordinal + 6,
                context.clone(),
                context.clone(),
                failure.clone(),
                confirm.clone(),
                read_token_balance,
            ),
            &failure_next,
        )?,
        state_with_failure(
            access_state::<EvmState<1, 6, K>, EvmCapability<6>>(
                start_ordinal + 7,
                context.clone(),
                context.clone(),
                failure.clone(),
                consolidate.clone(),
                confirm_anchor,
            ),
            &failure_next,
        )?,
        state_with_failure(
            pure_state::<EvmState<1, 7, K>>(
                start_ordinal + 8,
                context,
                completion,
                failure,
                Some(completion_next),
            ),
            &failure_next,
        )?,
    ]);
    Ok(())
}

fn address(ordinal: u32) -> Result<SequentialControlAddress, EvmDomainError> {
    SequentialControlAddress::new(ordinal, Vec::new()).map_err(|_| EvmDomainError::Program)
}

fn state_with_failure(
    state: Result<StateDeclaration, EvmDomainError>,
    failure_next: &SequentialControlAddress,
) -> Result<Declaration, EvmDomainError> {
    state?
        .with_failure_next(failure_next.clone())
        .map(|state| Declaration::State(Box::new(state)))
        .map_err(|_| EvmDomainError::Program)
}

fn pure_state<S: State>(
    ordinal: u32,
    input: mfm_ids::ContentRef,
    output: mfm_ids::ContentRef,
    failure: mfm_ids::ContentRef,
    next: Option<SequentialControlAddress>,
) -> Result<StateDeclaration, EvmDomainError> {
    let implementation = state_implementation_ref::<S>().map_err(|_| EvmDomainError::Program)?;
    match next {
        Some(next) => StateDeclaration::with_next(
            address(ordinal)?,
            implementation,
            input,
            output,
            Some(failure),
            ExecutionMode::Pure,
            next,
        )
        .map_err(|_| EvmDomainError::Program),
        None => StateDeclaration::new(
            address(ordinal)?,
            implementation,
            input,
            output,
            Some(failure),
            ExecutionMode::Pure,
            true,
        )
        .map_err(|_| EvmDomainError::Program),
    }
}

fn access_state<S: State, C: AccessCapabilityContract>(
    ordinal: u32,
    input: mfm_ids::ContentRef,
    output: mfm_ids::ContentRef,
    failure: mfm_ids::ContentRef,
    next: SequentialControlAddress,
    binding: &BindingDescriptor,
) -> Result<StateDeclaration, EvmDomainError> {
    validate_access_binding::<S, C>(binding)?;
    let implementation = state_implementation_ref::<S>().map_err(|_| EvmDomainError::Program)?;
    let capability_contract_ref =
        capability_contract_ref::<C>().map_err(|_| EvmDomainError::Program)?;
    let execution = match binding.effect_domain() {
        Some(effect_domain) => ExecutionMode::Effect {
            capability_contract_ref,
            effect_domain: effect_domain.clone(),
            fact_selection_required: C::requires_prior_facts(),
        },
        None => ExecutionMode::Read {
            capability_contract_ref,
            total_attempt_bound: C::total_attempt_bound().get(),
            fact_selection_required: C::requires_prior_facts(),
        },
    };
    StateDeclaration::with_next(
        address(ordinal)?,
        implementation,
        input,
        output,
        Some(failure),
        execution,
        next,
    )
    .map_err(|_| EvmDomainError::Program)?
    .with_execution_binding(binding.clone())
    .map_err(|_| EvmDomainError::Program)
}

fn validate_access_binding<S: State, C: AccessCapabilityContract>(
    binding: &BindingDescriptor,
) -> Result<(), EvmDomainError> {
    if binding.state_implementation_ref()
        != &state_implementation_ref::<S>().map_err(|_| EvmDomainError::Program)?
        || binding.capability_contract_ref()
            != Some(&capability_contract_ref::<C>().map_err(|_| EvmDomainError::Program)?)
        || binding.adapter_implementation_ref().is_none()
    {
        return Err(EvmDomainError::Program);
    }
    Ok(())
}

fn success<O, F>(output: O) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Success {
        output,
        facts: mfm_facts::FactProposalSet::empty(),
    }
}

fn failure<O, F>(failure: F) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Failure { failure }
}

fn read_returned<'a>(
    evidence: &'a EvmReadEvidence,
    intent: &EvmReadIntent,
) -> Option<&'a EvmReadValue> {
    if evidence.validate_for(intent).is_err() {
        return None;
    }
    match evidence {
        EvmReadEvidence::Returned { value } => Some(value),
        EvmReadEvidence::Rejected
        | EvmReadEvidence::SafeFailure
        | EvmReadEvidence::IntegrityBlocked => None,
    }
}

fn work_initial_anchor(work: &EvmBalanceWork) -> Option<&EvmBlockAnchor> {
    match work {
        EvmBalanceWork::SelectAsset { initial_anchor, .. }
        | EvmBalanceWork::ReadNativeBalance { initial_anchor, .. }
        | EvmBalanceWork::ReadTokenDecimals { initial_anchor, .. }
        | EvmBalanceWork::ReadTokenBalance { initial_anchor, .. }
        | EvmBalanceWork::ConfirmAnchor { initial_anchor, .. } => Some(initial_anchor),
        EvmBalanceWork::CheckChainIdentity { .. }
        | EvmBalanceWork::ReadInitialAnchor { .. }
        | EvmBalanceWork::Complete => None,
    }
}

fn decimal_at_least(left: &str, right: &str) -> bool {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    let left = if left.is_empty() { "0" } else { left };
    let right = if right.is_empty() { "0" } else { right };
    left.len() > right.len() || (left.len() == right.len() && left >= right)
}

fn scale_units(raw: &str, source_decimals: u8, target_decimals: u8) -> Option<String> {
    if !is_decimal_integer(raw) || source_decimals > 30 || target_decimals > 30 {
        return None;
    }
    if source_decimals == target_decimals {
        return Some(raw.to_owned());
    }
    if source_decimals < target_decimals {
        let zeros = usize::from(target_decimals - source_decimals);
        let mut scaled = raw.to_owned();
        scaled.reserve(zeros);
        scaled.extend(std::iter::repeat_n('0', zeros));
        return (scaled.len() <= 80).then_some(scaled);
    }
    let shift = usize::from(source_decimals - target_decimals);
    if raw.len() <= shift {
        return Some("0".to_owned());
    }
    let (whole, discarded) = raw.split_at(raw.len() - shift);
    discarded
        .bytes()
        .all(|byte| byte == b'0')
        .then(|| whole.trim_start_matches('0'))
        .map(|value| {
            if value.is_empty() {
                "0".to_owned()
            } else {
                value.to_owned()
            }
        })
}

fn completed_total_scaled(
    request: &EvmBalanceRequest,
    completed: &[EvmBalanceResult],
) -> Option<String> {
    let amounts = completed
        .iter()
        .map(|result| request.scale_units(&result.raw_units, result.decimals))
        .collect::<Option<Vec<_>>>()?;
    sum_decimal(amounts.iter().map(String::as_str))
}

fn sum_decimal<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut digits = vec![0u8];
    for value in values {
        if !is_decimal_integer(value) {
            return None;
        }
        let mut carry = 0u16;
        let width = digits.len().max(value.len());
        digits.resize(width, 0);
        for offset in 0..width {
            let right = value
                .as_bytes()
                .get(value.len().wrapping_sub(offset + 1))
                .copied()
                .unwrap_or(b'0')
                .saturating_sub(b'0') as u16;
            let index = digits.len() - 1 - offset;
            let sum = u16::from(digits[index]) + right + carry;
            digits[index] = (sum % 10) as u8;
            carry = sum / 10;
        }
        if carry != 0 {
            digits.insert(0, carry as u8);
        }
        if digits.len() > 80 {
            return None;
        }
    }
    Some(
        digits
            .into_iter()
            .map(|digit| char::from(b'0' + digit))
            .collect(),
    )
}

fn is_decimal_integer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes().iter().all(u8::is_ascii_digit)
        && (value == "0" || !value.starts_with('0'))
}

fn read_value_valid(value: &EvmReadValue) -> bool {
    match value {
        EvmReadValue::ChainId(chain_id) => *chain_id != 0,
        EvmReadValue::Anchor { number, hash } | EvmReadValue::CanonicalBlock { number, hash } => {
            read_anchor(number, hash).is_some()
        }
        EvmReadValue::RawUnits(units) => is_decimal_integer(units),
        EvmReadValue::TokenDecimals(decimals) => *decimals <= 30,
        EvmReadValue::Receipt {
            transaction_hash,
            execution_disposition,
            inclusion_block_number,
            inclusion_block_hash,
        } => {
            valid_public_text(transaction_hash, 256)
                && EvmExecutionDisposition::from_code(execution_disposition).is_some()
                && read_anchor(inclusion_block_number, inclusion_block_hash).is_some()
        }
        EvmReadValue::FinalizedHead { number } => is_decimal_integer(number),
    }
}

fn read_value_valid_for_intent(intent: &EvmReadIntent, value: &EvmReadValue) -> bool {
    read_value_valid(value)
        && match (&intent.subject, value) {
            (EvmReadSubject::ChainIdentity, EvmReadValue::ChainId(chain_id))
                if *chain_id == intent.chain_id =>
            {
                true
            }
            (EvmReadSubject::ChainIdentity, EvmReadValue::ChainId(_)) => false,
            (EvmReadSubject::InitialAnchor, EvmReadValue::Anchor { .. })
            | (EvmReadSubject::NativeBalance { .. }, EvmReadValue::RawUnits(_))
            | (EvmReadSubject::TokenDecimals { .. }, EvmReadValue::TokenDecimals(_))
            | (EvmReadSubject::TokenBalance { .. }, EvmReadValue::RawUnits(_))
            | (EvmReadSubject::ConfirmAnchor { .. }, EvmReadValue::Anchor { .. })
            | (EvmReadSubject::FinalizedHead, EvmReadValue::FinalizedHead { .. }) => true,
            (
                EvmReadSubject::TransactionReceipt { transaction_hash },
                EvmReadValue::Receipt {
                    transaction_hash: returned_hash,
                    ..
                },
            ) => transaction_hash == returned_hash,
            (
                EvmReadSubject::CanonicalInclusionBlock { number },
                EvmReadValue::CanonicalBlock {
                    number: returned_number,
                    ..
                },
            ) => number == returned_number,
            _ => false,
        }
}

fn read_anchor(number: &str, hash: &str) -> Option<EvmBlockAnchor> {
    EvmBlockAnchor::new(number.to_owned(), hash.to_owned()).ok()
}

fn duplicate_source_ids(sources: &[EvmBalanceSource]) -> bool {
    let mut ids = BTreeSet::new();
    sources
        .iter()
        .any(|source| !ids.insert(source.source_id.as_str()))
}

fn duplicate_collected_source_ids(balances: &[EvmCollectedBalance]) -> bool {
    let mut ids = BTreeSet::new();
    balances
        .iter()
        .any(|balance| !ids.insert(balance.source.source_id.as_str()))
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !string_contains_secret_marker(value)
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "../tests/unit.rs"]
mod tests;
