#![warn(missing_docs)]
//! Secret-free EVM State values and capability contracts.
//!
//! The domain owns complete cumulative contexts for the two current operations.  Provider
//! clients, signing authority, nonce storage, and response ingress belong to the live adapter
//! crate; no domain value contains a client, credential, raw response, or generic context map.

use std::collections::BTreeSet;
use std::num::NonZeroU16;

use mfm_capabilities::{AccessCapabilityContract, EffectMode, NoPriorFacts, ReadMode};
use mfm_ids::StableId;
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_values::{string_contains_secret_marker, MfmValue as MfmValueTrait};
use serde::de;
use serde::{Deserialize, Serialize};

/// Stable entry-point identity for EVM submission.
pub const EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID: &str = "mfm.evm/submit-transaction@1";
/// Stable operation identity for EVM submission.
pub const EVM_SUBMIT_TRANSACTION_OPERATION_ID: &str = "mfm.evm.submit-transaction@1";
/// Stable entry-point identity for EVM balance collection fragments.
pub const EVM_BALANCE_COLLECTION_OPERATION_ID: &str = "mfm.evm.balance-collection@1";
/// Maximum admitted EVM balance sources.
pub const EVM_BALANCE_SOURCE_LIMIT: usize = 64;
/// Maximum EVM transaction payload bytes.
pub const EVM_TRANSACTION_DATA_LIMIT: usize = 128 * 1024;

/// Secret-free EVM configuration selected by trusted composition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[serde(deny_unknown_fields)]
pub struct EvmConfig {}

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

impl<'de> Deserialize<'de> for EvmTransactionTarget {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            chain_id: u64,
            sender: String,
            nonce_domain: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.chain_id, wire.sender, wire.nonce_domain).map_err(de::Error::custom)
    }
}

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
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(
            self.chain_id,
            self.sender.clone(),
            self.nonce_domain.clone(),
        )
        .map(|_| ())
    }
}

/// A singular domain-planned admission value for EVM submission.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionRequest {
    /// Public target and nonce domain.
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

impl<'de> Deserialize<'de> for EvmSubmissionRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            target: EvmTransactionTarget,
            idempotency_key: String,
            data: Vec<u8>,
            gas_limit: u64,
            max_fee: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.target,
            wire.idempotency_key,
            wire.data,
            wire.gas_limit,
            wire.max_fee,
        )
        .map_err(de::Error::custom)
    }
}

impl EvmSubmissionRequest {
    /// Constructs one bounded submission request.
    pub fn new(
        target: EvmTransactionTarget,
        idempotency_key: String,
        data: Vec<u8>,
        gas_limit: u64,
        max_fee: String,
    ) -> Result<Self, EvmDomainError> {
        target.validate()?;
        if idempotency_key.is_empty()
            || idempotency_key.len() > 256
            || contains_secret_marker(&idempotency_key)
            || data.len() > EVM_TRANSACTION_DATA_LIMIT
            || gas_limit == 0
            || !is_decimal_integer(&max_fee)
            || max_fee.len() > 80
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            target,
            idempotency_key,
            data,
            gas_limit,
            max_fee,
        })
    }

    /// Validates a decoded submission request by representation and domain bounds.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        self.target.validate()?;
        if self.idempotency_key.is_empty()
            || self.idempotency_key.len() > 256
            || contains_secret_marker(&self.idempotency_key)
            || self.data.len() > EVM_TRANSACTION_DATA_LIMIT
            || self.gas_limit == 0
            || !is_decimal_integer(&self.max_fee)
            || self.max_fee.len() > 80
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Complete cumulative submission context retained across sequential States.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionContext {
    /// Original admitted request.
    pub request: EvmSubmissionRequest,
    /// Selected wallet nonce after the upstream Read State.
    pub nonce: u64,
    /// Deterministic candidate identity fixed before broadcast.
    pub candidate_id: String,
}

impl<'de> Deserialize<'de> for EvmSubmissionContext {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: EvmSubmissionRequest,
            nonce: u64,
            candidate_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.request, wire.nonce, wire.candidate_id).map_err(de::Error::custom)
    }
}

impl EvmSubmissionContext {
    /// Constructs one complete candidate context after validating the admitted request.
    pub fn new(
        request: EvmSubmissionRequest,
        nonce: u64,
        candidate_id: String,
    ) -> Result<Self, EvmDomainError> {
        let context = Self {
            request,
            nonce,
            candidate_id,
        };
        context.validate()?;
        Ok(context)
    }

    /// Validates the complete cumulative submission context.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        self.request.validate()?;
        if !valid_public_text(&self.candidate_id, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Public terminal EVM submission result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmSubmissionOutput {
    /// Deterministic candidate identity.
    pub candidate_id: String,
    /// Provider-asserted transaction hash.
    pub transaction_hash: String,
    /// Inclusion status chosen by the domain State.
    pub included: bool,
}

impl<'de> Deserialize<'de> for EvmSubmissionOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            candidate_id: String,
            transaction_hash: String,
            included: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        let output = Self {
            candidate_id: wire.candidate_id,
            transaction_hash: wire.transaction_hash,
            included: wire.included,
        };
        output.validate().map(|_| output).map_err(de::Error::custom)
    }
}

impl EvmSubmissionOutput {
    /// Validates one terminal public submission result.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.candidate_id, 256)
            || !valid_public_text(&self.transaction_hash, 256)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Explicit fail-fast EVM submission failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmSubmissionFailure {
    /// The public input or binding was invalid.
    InvalidInput,
    /// A wallet nonce could not be read safely.
    NonceUnavailable,
    /// The candidate was rejected before provider entry.
    Rejected {
        /// Stable redacted rejection code.
        code: String,
    },
    /// The provider result could not be trusted for interpretation.
    IntegrityBlocked,
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

impl<'de> Deserialize<'de> for EvmBalanceSource {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source_id: String,
            chain_id: u64,
            address: String,
            token: Option<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let source = Self {
            source_id: wire.source_id,
            chain_id: wire.chain_id,
            address: wire.address,
            token: wire.token,
        };
        source.validate().map(|_| source).map_err(de::Error::custom)
    }
}

impl EvmBalanceSource {
    /// Validates one public source identity without changing ownership.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
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
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceRequest {
    /// Ordered sources; the sequential Program expands one State per source.
    pub sources: Vec<EvmBalanceSource>,
    /// Integer decimal scale for public amounts.
    pub decimals: u8,
}

impl<'de> Deserialize<'de> for EvmBalanceRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            sources: Vec<EvmBalanceSource>,
            decimals: u8,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.sources, wire.decimals).map_err(de::Error::custom)
    }
}

impl EvmBalanceRequest {
    /// Creates one bounded declaration-ordered source list.
    pub fn new(sources: Vec<EvmBalanceSource>, decimals: u8) -> Result<Self, EvmDomainError> {
        if sources.is_empty()
            || sources.len() > EVM_BALANCE_SOURCE_LIMIT
            || decimals > 30
            || sources.iter().any(|source| source.validate().is_err())
            || duplicate_source_ids(&sources)
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
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Public fields carried unchanged beside one EVM collection result.
///
/// This metadata is deliberately separate from the caller continuation: EVM may validate its
/// own bounded correlation fields, but it cannot inspect or derive the caller's type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceResultMetadata {
    /// Declaration ordinal of the owning Portfolio collection.
    pub collection_ordinal: u32,
    /// Bounded public correlation allocated by the caller.
    pub correlation: String,
}

impl<'de> Deserialize<'de> for EvmBalanceResultMetadata {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            collection_ordinal: u32,
            correlation: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.collection_ordinal, wire.correlation).map_err(de::Error::custom)
    }
}

/// Exact stage of one declaration-ordered EVM balance source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub enum EvmBalanceStage {
    /// The source asset still needs deterministic native/token selection.
    SelectAsset,
    /// Native balance request.
    NativeBalance,
    /// Token decimal metadata request.
    TokenDecimals,
    /// Token balance request.
    TokenBalance,
    /// Anchor confirmation request.
    Anchor,
    /// Source processing is complete.
    Complete,
}

impl EvmBalanceStage {
    /// Returns whether this stage requires an active source payload.
    pub const fn requires_current_source(&self) -> bool {
        !matches!(self, Self::SelectAsset)
    }
}

/// Closed Match selector for one source asset.
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmBalanceAsset {
    /// Native balance source.
    Native {
        /// Exact source declaration.
        source: EvmBalanceSource,
    },
    /// Token balance source.
    Token {
        /// Exact source declaration.
        source: EvmBalanceSource,
        /// Exact public token contract.
        token: String,
    },
}

impl EvmBalanceAsset {
    /// Validates that a closed Match payload selects the exact next source and asset kind.
    pub fn validate_for(&self, expected: &EvmBalanceSource) -> Result<(), EvmDomainError> {
        match self {
            Self::Native { source } if source == expected && source.token.is_none() => Ok(()),
            Self::Token { source, token }
                if source == expected
                    && expected.token.as_deref() == Some(token.as_str())
                    && !token.is_empty() =>
            {
                Ok(())
            }
            _ => Err(EvmDomainError::InvalidValue),
        }
    }
}

/// Complete payload consumed by the native Match arm.
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmNativeBalanceInput {
    /// Exact selected source.
    pub source: EvmBalanceSource,
}

impl EvmNativeBalanceInput {
    /// Creates one native Match-arm payload after checking its asset kind.
    pub fn new(source: EvmBalanceSource) -> Result<Self, EvmDomainError> {
        source.validate()?;
        if source.token.is_some() {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { source })
    }
}

/// Complete payload consumed by the token Match arm.
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmTokenBalanceInput {
    /// Exact selected source.
    pub source: EvmBalanceSource,
    /// Exact selected token contract.
    pub token: String,
}

impl EvmTokenBalanceInput {
    /// Creates one token Match-arm payload after checking source/token agreement.
    pub fn new(source: EvmBalanceSource, token: String) -> Result<Self, EvmDomainError> {
        source.validate()?;
        if token.is_empty() || source.token.as_deref() != Some(token.as_str()) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { source, token })
    }
}

impl EvmBalanceResultMetadata {
    /// Creates one bounded public result correlation.
    pub fn new(collection_ordinal: u32, correlation: String) -> Result<Self, EvmDomainError> {
        if !valid_public_text(&correlation, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            collection_ordinal,
            correlation,
        })
    }

    /// Validates decoded metadata without changing ownership.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(self.collection_ordinal, self.correlation.clone()).map(|_| ())
    }
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
    /// Singular admitted request.
    pub request: EvmBalanceRequest,
    /// Opaque caller continuation retained unchanged until completion.
    pub caller_continuation: K,
    /// Public collection correlation owned by the caller.
    pub metadata: EvmBalanceResultMetadata,
    /// Current declaration-ordered source, when a source is active.
    pub current_source: Option<EvmBalanceSource>,
    /// Exact source-stage discriminator.
    pub stage: EvmBalanceStage,
    /// Declaration-ordered results already completed.
    pub completed: Vec<EvmBalanceResult>,
    /// Next source ordinal.
    pub next_source: u16,
    /// Remaining source declarations after the current source.
    pub remaining_sources: Vec<EvmBalanceSource>,
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
            current_source: Option<EvmBalanceSource>,
            stage: EvmBalanceStage,
            completed: Vec<EvmBalanceResult>,
            next_source: u16,
            remaining_sources: Vec<EvmBalanceSource>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let context = Self {
            request: wire.request,
            caller_continuation: wire.caller_continuation,
            metadata: wire.metadata,
            current_source: wire.current_source,
            stage: wire.stage,
            completed: wire.completed,
            next_source: wire.next_source,
            remaining_sources: wire.remaining_sources,
        };
        context
            .validate()
            .map(|_| context)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceContext<K> {
    /// Validates the declaration-ordered cumulative work envelope.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        self.request.validate()?;
        self.metadata.validate()?;
        if self.next_source as usize > self.request.sources.len()
            || self.completed.len() != self.next_source as usize
            || self
                .completed
                .iter()
                .zip(&self.request.sources)
                .any(|(result, source)| {
                    result.validate().is_err() || result.source_id != source.source_id
                })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        let next = self.next_source as usize;
        let remaining_start = if let Some(current) = &self.current_source {
            if next >= self.request.sources.len() || current != &self.request.sources[next] {
                return Err(EvmDomainError::InvalidValue);
            }
            next + 1
        } else {
            next
        };
        if self.stage.requires_current_source() != self.current_source.is_some()
            || self.remaining_sources != self.request.sources[remaining_start..]
            || self.remaining_sources.len() > EVM_BALANCE_SOURCE_LIMIT
        {
            return Err(EvmDomainError::InvalidValue);
        }
        if let Some(current) = &self.current_source {
            current.validate()?;
            match self.stage {
                EvmBalanceStage::NativeBalance if current.token.is_some() => {
                    return Err(EvmDomainError::InvalidValue)
                }
                EvmBalanceStage::TokenDecimals | EvmBalanceStage::TokenBalance
                    if current.token.is_none() =>
                {
                    return Err(EvmDomainError::InvalidValue)
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// One source result retained in the cumulative context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceResult {
    /// Source identity.
    pub source_id: String,
    /// Integer-scaled amount.
    pub amount_scaled: String,
    /// Anchor identity used by the adapter assertion.
    pub anchor: String,
}

impl<'de> Deserialize<'de> for EvmBalanceResult {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source_id: String,
            amount_scaled: String,
            anchor: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let result = Self {
            source_id: wire.source_id,
            amount_scaled: wire.amount_scaled,
            anchor: wire.anchor,
        };
        result.validate().map(|_| result).map_err(de::Error::custom)
    }
}

impl EvmBalanceResult {
    /// Validates one interpreted source result.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.source_id, 256)
            || !is_decimal_integer(&self.amount_scaled)
            || !valid_public_text(&self.anchor, 256)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Final public EVM balance collection result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceCollectionResult {
    /// Ordered source results.
    pub results: Vec<EvmBalanceResult>,
    /// Total integer-scaled amount.
    pub total_scaled: String,
}

impl<'de> Deserialize<'de> for EvmBalanceCollectionResult {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            results: Vec<EvmBalanceResult>,
            total_scaled: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let result = Self {
            results: wire.results,
            total_scaled: wire.total_scaled,
        };
        result.validate().map(|_| result).map_err(de::Error::custom)
    }
}

impl EvmBalanceCollectionResult {
    /// Validates ordered public results and integer-scaled total output.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if self.results.is_empty()
            || self.results.len() > EVM_BALANCE_SOURCE_LIMIT
            || self.results.iter().any(|result| result.validate().is_err())
            || duplicate_result_ids(&self.results)
            || !is_decimal_integer(&self.total_scaled)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// One consuming EVM completion returned to the caller-owned continuation.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub struct EvmBalanceCollectionCompletion<K: MfmValueTrait> {
    /// The exact caller continuation, moved without EVM interpretation.
    pub caller_continuation: K,
    /// The one completed public collection result.
    pub result: EvmBalanceCollectionResult,
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
            caller_continuation: K,
            result: EvmBalanceCollectionResult,
        }

        let wire = Wire::deserialize(deserializer)?;
        let completion = Self {
            caller_continuation: wire.caller_continuation,
            result: wire.result,
        };
        completion
            .validate()
            .map(|_| completion)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceCollectionCompletion<K> {
    /// Constructs one completed collection handoff without interpreting the caller value.
    pub fn new(
        caller_continuation: K,
        result: EvmBalanceCollectionResult,
    ) -> Result<Self, EvmDomainError> {
        result.validate()?;
        Ok(Self {
            caller_continuation,
            result,
        })
    }

    /// Validates the EVM-owned result portion of the handoff.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        self.result.validate()
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
}

/// One strict read intent shared by the bounded EVM read capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmReadIntent {
    /// Capability operation identity.
    pub operation: String,
    /// Public chain target.
    pub chain_id: u64,
    /// Public account or transaction selector.
    pub subject: String,
}

impl<'de> Deserialize<'de> for EvmReadIntent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            operation: String,
            chain_id: u64,
            subject: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.operation, wire.chain_id, wire.subject).map_err(de::Error::custom)
    }
}

impl EvmReadIntent {
    /// Creates one bounded provider intent.
    pub fn new(operation: String, chain_id: u64, subject: String) -> Result<Self, EvmDomainError> {
        let intent = Self {
            operation,
            chain_id,
            subject,
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Validates a decoded provider intent.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if StableId::new(&self.operation).is_err()
            || self.chain_id == 0
            || !valid_public_text(&self.subject, 256)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
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
    /// Provider returned a bounded value.
    Returned {
        /// Operation identity echoed by the authenticated adapter.
        operation: String,
        /// Subject identity echoed by the authenticated adapter.
        subject: String,
        /// Interpreted provider value.
        value: String,
        /// Provider anchor bound to the request.
        anchor: String,
    },
    /// Provider returned a reviewed rejection.
    Rejected {
        /// Operation identity echoed by the authenticated adapter.
        operation: String,
        /// Stable redacted rejection code.
        code: String,
    },
    /// Adapter accepted a safe failure.
    SafeFailure {
        /// Operation identity echoed by the authenticated adapter.
        operation: String,
        /// Stable redacted failure code.
        code: String,
    },
    /// Authentication/integrity failed and grants no retry authority.
    IntegrityBlocked {
        /// Operation identity echoed by the authenticated adapter.
        operation: String,
        /// Stable redacted integrity code.
        code: String,
    },
}

impl EvmReadEvidence {
    /// Validates the exact operation-bound evidence envelope.
    pub fn validate_for(&self, intent: &EvmReadIntent) -> Result<(), EvmDomainError> {
        let valid = match self {
            Self::Returned {
                operation,
                subject,
                value,
                anchor,
            } => {
                operation == &intent.operation
                    && subject == &intent.subject
                    && valid_public_text(value, 2 * 1024 * 1024)
                    && valid_public_text(anchor, 256)
            }
            Self::Rejected { operation, code }
            | Self::SafeFailure { operation, code }
            | Self::IntegrityBlocked { operation, code } => {
                operation == &intent.operation && valid_public_text(code, 256)
            }
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

/// Read capability for wallet nonce status.
pub enum ReadWalletNonceStatus {}
/// Read capability for a chain anchor.
pub enum ReadLatestAnchor {}
/// Read capability for a balance result.
pub enum ReadBalance {}

macro_rules! impl_read_capability {
    ($ty:ty, $name:literal) => {
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
                intent
                    .validate()
                    .and_then(|_| evidence.validate_for(intent))
                    .map_err(|_| mfm_capabilities::CapabilityError::EvidenceBinding)
            }
        }
    };
}

impl_read_capability!(
    ReadWalletNonceStatus,
    "mfm.evm.capability.read-wallet-nonce@1"
);
impl_read_capability!(ReadLatestAnchor, "mfm.evm.capability.read-anchor@1");
impl_read_capability!(ReadBalance, "mfm.evm.capability.read-balance@1");

/// Deterministic transaction candidate intent fixed before provider entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct BroadcastIntent {
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
}

impl<'de> Deserialize<'de> for BroadcastIntent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            idempotency_key: String,
            candidate_id: String,
            nonce: u64,
            sender: String,
            nonce_domain: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.idempotency_key,
            wire.candidate_id,
            wire.nonce,
            wire.sender,
            wire.nonce_domain,
        )
        .map_err(de::Error::custom)
    }
}

impl BroadcastIntent {
    /// Creates one bounded deterministic broadcast intent.
    pub fn new(
        idempotency_key: String,
        candidate_id: String,
        nonce: u64,
        sender: String,
        nonce_domain: String,
    ) -> Result<Self, EvmDomainError> {
        let intent = Self {
            idempotency_key,
            candidate_id,
            nonce,
            sender,
            nonce_domain,
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Validates decoded broadcast intent without exposing request bytes.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.idempotency_key, 256)
            || !valid_public_text(&self.candidate_id, 256)
            || !valid_public_text(&self.sender, 128)
            || self.sender != self.sender.to_ascii_lowercase()
            || !valid_public_text(&self.nonce_domain, 256)
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
        /// Deterministic candidate identity echoed by the authenticated adapter.
        candidate_id: String,
        /// Provider-asserted transaction hash.
        transaction_hash: String,
    },
    /// Provider rejected before durable entry.
    Rejected {
        /// Deterministic candidate identity echoed by the authenticated adapter.
        candidate_id: String,
        /// Stable redacted rejection code.
        code: String,
    },
    /// Stable-key operation is known to have entered but result is not available.
    PossibleEntry {
        /// Deterministic candidate identity whose entry may have occurred.
        candidate_id: String,
    },
    /// Request/response integrity failed.
    IntegrityBlocked {
        /// Deterministic candidate identity bound by the request.
        candidate_id: String,
    },
}

impl BroadcastEvidence {
    /// Validates the exact candidate-bound evidence envelope.
    pub fn validate_for(&self, intent: &BroadcastIntent) -> Result<(), EvmDomainError> {
        let valid = match self {
            Self::Returned {
                candidate_id,
                transaction_hash,
            } => candidate_id == &intent.candidate_id && valid_public_text(transaction_hash, 256),
            Self::Rejected { candidate_id, code } => {
                candidate_id == &intent.candidate_id && valid_public_text(code, 256)
            }
            Self::PossibleEntry { candidate_id } | Self::IntegrityBlocked { candidate_id } => {
                candidate_id == &intent.candidate_id
            }
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

/// One-entry broadcast capability.
pub enum BroadcastTransaction {}

impl AccessCapabilityContract for BroadcastTransaction {
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

fn contains_secret_marker(value: &str) -> bool {
    string_contains_secret_marker(value)
}

fn is_decimal_integer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes().iter().all(u8::is_ascii_digit)
        && (value == "0" || !value.starts_with('0'))
}

fn duplicate_source_ids(sources: &[EvmBalanceSource]) -> bool {
    let mut ids = BTreeSet::new();
    sources
        .iter()
        .any(|source| !ids.insert(source.source_id.as_str()))
}

fn duplicate_result_ids(results: &[EvmBalanceResult]) -> bool {
    let mut ids = BTreeSet::new();
    results
        .iter()
        .any(|result| !ids.insert(result.source_id.as_str()))
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !contains_secret_marker(value)
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, MfmValue)]
    #[serde(deny_unknown_fields)]
    struct OpaqueCallerContinuation {
        marker: String,
    }

    #[test]
    fn generic_balance_completion_moves_a_non_clone_continuation() {
        let request = EvmBalanceRequest::new(
            vec![EvmBalanceSource {
                source_id: "source-1".to_owned(),
                chain_id: 1,
                address: "0xabc".to_owned(),
                token: None,
            }],
            18,
        )
        .expect("request");
        let continuation = OpaqueCallerContinuation {
            marker: "opaque".to_owned(),
        };
        let context = EvmBalanceContext {
            request,
            caller_continuation: continuation,
            metadata: EvmBalanceResultMetadata::new(0, "correlation".to_owned()).expect("metadata"),
            current_source: None,
            stage: EvmBalanceStage::SelectAsset,
            completed: Vec::new(),
            next_source: 0,
            remaining_sources: vec![EvmBalanceSource {
                source_id: "source-1".to_owned(),
                chain_id: 1,
                address: "0xabc".to_owned(),
                token: None,
            }],
        };
        assert!(context.validate().is_ok());
        let EvmBalanceContext {
            caller_continuation,
            ..
        } = context;
        let completion = EvmBalanceCollectionCompletion {
            caller_continuation,
            result: EvmBalanceCollectionResult {
                results: Vec::new(),
                total_scaled: "0".to_owned(),
            },
        };
        assert_eq!(completion.caller_continuation.marker, "opaque");
    }

    #[test]
    fn balance_request_rejects_non_adjacent_duplicate_sources() {
        let source = EvmBalanceSource {
            source_id: "source-1".to_owned(),
            chain_id: 1,
            address: "0xabc".to_owned(),
            token: None,
        };
        assert!(EvmBalanceRequest::new(
            vec![
                source.clone(),
                EvmBalanceSource {
                    source_id: "source-2".to_owned(),
                    ..source.clone()
                },
                source,
            ],
            18,
        )
        .is_err());
    }

    #[test]
    fn admission_and_result_deserialization_reenter_domain_validation() {
        assert!(serde_json::from_str::<EvmSubmissionRequest>(
            r#"{"target":{"chain_id":1,"sender":"0xABC","nonce_domain":"main"},"idempotency_key":"request","data":[],"gas_limit":1,"max_fee":"1"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<EvmSubmissionOutput>(
            r#"{"candidate_id":"candidate","transaction_hash":"","included":true}"#
        )
        .is_err());
    }

    #[test]
    fn transaction_data_capacity_accepts_exact_and_rejects_plus_one() {
        let target = EvmTransactionTarget::new(1, "0xabc".to_owned(), "wallet-main".to_owned())
            .expect("target");
        assert!(EvmSubmissionRequest::new(
            target.clone(),
            "capacity-exact".to_owned(),
            vec![0; EVM_TRANSACTION_DATA_LIMIT],
            1,
            "1".to_owned(),
        )
        .is_ok());
        assert!(EvmSubmissionRequest::new(
            target,
            "capacity-plus-one".to_owned(),
            vec![0; EVM_TRANSACTION_DATA_LIMIT + 1],
            1,
            "1".to_owned(),
        )
        .is_err());
    }
}
