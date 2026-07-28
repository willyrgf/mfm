//! Pure immutable EVM wallet request and evidence contracts.
//!
//! The values in this module contain only reviewed public transaction
//! material. Signatures, signed envelopes, endpoints, credentials, provider
//! bodies, and secret-provider paths have no representation here.

use std::collections::BTreeSet;
use std::str::FromStr;

use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_executor::{CanonicalExecutorRequest, EvidenceBounds, SchemaQualifiedCanonicalValue};
use mfm_ids::{ContentDigest, ContentRef, LocalPublicId, SchemaId, TenantScopeId};
use mfm_program::{boundary_content_ref, decode_boundary, encode_boundary};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs, StateInput};
use mfm_values::SchemaDescriptor;
use serde::{de, Deserialize, Serialize};

use crate::EvmBlockAnchor;

/// Published transaction-submission entry point.
pub const EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID: &str = "mfm.evm/submit-transaction@1";
/// Stable operation and executor-operation identity.
pub const EVM_SUBMIT_TRANSACTION_OPERATION_ID: &str = "mfm.evm/submit-transaction";
/// Exact EVM transaction envelope type.
pub const EVM_WALLET_TRANSACTION_TYPE: u8 = 2;
/// Only admitted finality selector.
pub const EVM_WALLET_FINALITY_TAG: &str = "finalized";
/// Maximum call data or creation init-code bytes.
pub const EVM_WALLET_DATA_MAX_BYTES: usize = 128 * 1024;
/// Maximum access-list entries.
pub const EVM_WALLET_ACCESS_LIST_MAX_ENTRIES: usize = 256;
/// Maximum total access-list storage keys.
pub const EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS: usize = 4_096;
/// Maximum number of immutable EIP-1559 fee candidates.
pub const EVM_WALLET_REPLACEMENT_LIMIT: usize = 32;
/// Maximum receipt logs retained in terminal evidence.
pub const EVM_WALLET_RECEIPT_LOG_LIMIT: usize = 4_096;
/// Maximum unindexed bytes retained in one receipt log.
pub const EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Conservative retained wrapper allowance per executor evidence record.
pub const EVM_WALLET_EXECUTOR_RECORD_OVERHEAD_BYTES: u64 = 4 * 1024;

/// Exact target operation for a raw type-2 broadcast or rebroadcast.
pub const EVM_WALLET_BROADCAST_OPERATION_ID: &str = "eth_send_raw_transaction";
/// Exact target operation for transaction lookup by hash.
pub const EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID: &str = "eth_get_transaction_by_hash";
/// Exact target operation for receipt lookup by hash.
pub const EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID: &str = "eth_get_transaction_receipt";
/// Exact target operation for the finalized head.
pub const EVM_WALLET_FINALIZED_HEAD_OPERATION_ID: &str = "eth_get_block_by_number_finalized";
/// Exact target operation for canonical inclusion-block lookup.
pub const EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID: &str = "eth_get_block_by_number_inclusion";

/// Closed error taxonomy for pure wallet values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmWalletError {
    /// A field is not in its one canonical representation.
    #[error("invalid EVM wallet field: {0}")]
    Invalid(&'static str),
    /// A bounded wallet collection exceeded its fixed limit.
    #[error("EVM wallet bound exceeded: {0}")]
    BoundExceeded(&'static str),
    /// The fee schedule is empty or not strictly monotonic.
    #[error("EVM replacement schedule is invalid")]
    InvalidReplacementSchedule,
    /// Retained candidate or terminal evidence is inconsistent.
    #[error("EVM wallet evidence is inconsistent")]
    InconsistentEvidence,
    /// Canonical request construction failed.
    #[error("EVM wallet request encoding failed")]
    RequestEncoding,
}

/// Schema-qualified identity used for reviewed wallet contracts and
/// attestations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-reference",
    version = "1",
    schema = "mfm.evm.wallet_reference"
)]
pub struct EvmWalletReference {
    schema_id: String,
    content_digest: String,
}

impl EvmWalletReference {
    /// Brands one already reviewed content identity.
    pub fn from_content_ref(reference: ContentRef) -> Self {
        Self {
            schema_id: reference.schema_id().as_str().to_owned(),
            content_digest: reference.content_digest().as_str().to_owned(),
        }
    }

    /// Reconstructs the exact non-authorizing content identity.
    pub fn to_content_ref(&self) -> Result<ContentRef, EvmWalletError> {
        let schema = SchemaId::from_str(&self.schema_id)
            .map_err(|_| EvmWalletError::Invalid("wallet_reference"))?;
        let digest = ContentDigest::from_str(&self.content_digest)
            .map_err(|_| EvmWalletError::Invalid("wallet_reference"))?;
        ContentRef::new(schema, digest).map_err(|_| EvmWalletError::Invalid("wallet_reference"))
    }

    /// Returns the exact schema identity.
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    /// Returns the exact raw-byte digest identity.
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }
}

impl<'de> Deserialize<'de> for EvmWalletReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_id: String,
            content_digest: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let reference = Self {
            schema_id: wire.schema_id,
            content_digest: wire.content_digest,
        };
        reference.to_content_ref().map_err(de::Error::custom)?;
        Ok(reference)
    }
}

/// Stable configured-value target selected by the public entry-point input.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-target",
    version = "1",
    schema = "mfm.evm.transaction_target"
)]
pub struct EvmTransactionTarget {
    value: String,
}

impl EvmTransactionTarget {
    /// Creates one checked configured-value target.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmWalletError> {
        let value = value.into();
        LocalPublicId::new(&value).map_err(|_| EvmWalletError::Invalid("transaction_target"))?;
        Ok(Self { value })
    }

    /// Returns the stable configured-value target.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<'de> Deserialize<'de> for EvmTransactionTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Public value-only selector for one configured immutable transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submit-transaction-selector",
    version = "1",
    schema = "mfm.evm.submit_transaction_selector"
)]
pub struct EvmSubmitTransactionSelector {
    target: EvmTransactionTarget,
}

impl EvmSubmitTransactionSelector {
    /// Selects one exact configured transaction target.
    pub const fn new(target: EvmTransactionTarget) -> Self {
        Self { target }
    }

    /// Returns the configured-value target.
    pub const fn target(&self) -> &EvmTransactionTarget {
        &self.target
    }
}

/// One exact EIP-2930 access-list entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-access-list-entry",
    version = "1",
    schema = "mfm.evm.wallet_access_list_entry"
)]
pub struct EvmWalletAccessListEntry {
    address: String,
    storage_keys: Vec<String>,
}

impl EvmWalletAccessListEntry {
    /// Creates an entry while preserving the exact transaction order.
    pub fn new(address: Address, storage_keys: Vec<B256>) -> Result<Self, EvmWalletError> {
        if storage_keys.len() > EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS {
            return Err(EvmWalletError::BoundExceeded("access_list_storage_keys"));
        }
        let entry = Self {
            address: canonical_address(address),
            storage_keys: storage_keys.into_iter().map(canonical_hash).collect(),
        };
        entry.validate()?;
        Ok(entry)
    }

    /// Returns the canonical address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns canonical storage keys in signed transaction order.
    pub fn storage_keys(&self) -> &[String] {
        &self.storage_keys
    }

    /// Converts to the Alloy access-list representation.
    pub fn to_alloy(&self) -> Result<AccessListItem, EvmWalletError> {
        Ok(AccessListItem {
            address: parse_address(&self.address)?,
            storage_keys: self
                .storage_keys
                .iter()
                .map(|value| parse_hash(value))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        parse_address(&self.address)?;
        if self.storage_keys.len() > EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS {
            return Err(EvmWalletError::BoundExceeded("access_list_storage_keys"));
        }
        let mut unique = BTreeSet::new();
        for key in &self.storage_keys {
            parse_hash(key)?;
            if !unique.insert(key) {
                return Err(EvmWalletError::Invalid("duplicate_access_list_key"));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletAccessListEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            address: String,
            storage_keys: Vec<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let entry = Self {
            address: wire.address,
            storage_keys: wire.storage_keys,
        };
        entry.validate().map_err(de::Error::custom)?;
        Ok(entry)
    }
}

/// Closed direct EOA transaction action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-transaction-action",
    version = "1",
    schema = "mfm.evm.wallet_transaction_action"
)]
pub enum EvmWalletTransactionAction {
    /// Direct EOA contract creation.
    Create,
    /// Ordinary call, including a native transfer with empty input.
    Call {
        /// Canonical lower-case destination.
        to: String,
    },
}

impl EvmWalletTransactionAction {
    /// Creates one checked direct-creation action.
    pub const fn create() -> Self {
        Self::Create
    }

    /// Creates one checked ordinary call.
    pub fn call(to: Address) -> Self {
        Self::Call {
            to: canonical_address(to),
        }
    }

    /// Returns the exact Alloy transaction target, value-independent.
    pub fn to_alloy(&self) -> Result<TxKind, EvmWalletError> {
        match self {
            Self::Create => Ok(TxKind::Create),
            Self::Call { to, .. } => Ok(TxKind::Call(parse_address(to)?)),
        }
    }

    /// Returns the call destination, or `None` for direct creation.
    pub fn call_destination(&self) -> Result<Option<Address>, EvmWalletError> {
        match self {
            Self::Create => Ok(None),
            Self::Call { to, .. } => parse_address(to).map(Some),
        }
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        match self {
            Self::Create => {}
            Self::Call { to } => {
                parse_address(to)?;
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletTransactionAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Create,
            Call { to: String },
        }

        let action = match Wire::deserialize(deserializer)? {
            Wire::Create => Self::Create,
            Wire::Call { to } => Self::Call { to },
        };
        action.validate().map_err(de::Error::custom)?;
        Ok(action)
    }
}

/// One immutable EIP-1559 fee candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-fee-candidate",
    version = "1",
    schema = "mfm.evm.wallet_fee_candidate"
)]
pub struct EvmWalletFeeCandidate {
    max_fee_per_gas: String,
    max_priority_fee_per_gas: String,
}

impl EvmWalletFeeCandidate {
    /// Creates one checked EIP-1559 fee pair.
    pub fn new(
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self, EvmWalletError> {
        let candidate = Self {
            max_fee_per_gas: max_fee_per_gas.to_string(),
            max_priority_fee_per_gas: max_priority_fee_per_gas.to_string(),
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Returns the canonical maximum total fee.
    pub fn max_fee_per_gas(&self) -> &str {
        &self.max_fee_per_gas
    }

    /// Returns the canonical maximum priority fee.
    pub fn max_priority_fee_per_gas(&self) -> &str {
        &self.max_priority_fee_per_gas
    }

    /// Parses the checked maximum total fee.
    pub fn max_fee_quantity(&self) -> Result<U256, EvmWalletError> {
        parse_quantity(&self.max_fee_per_gas)
    }

    /// Parses the checked maximum priority fee.
    pub fn max_priority_fee_quantity(&self) -> Result<U256, EvmWalletError> {
        parse_quantity(&self.max_priority_fee_per_gas)
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        let maximum = self.max_fee_quantity()?;
        let priority = self.max_priority_fee_quantity()?;
        if priority > maximum {
            return Err(EvmWalletError::Invalid("priority_fee_exceeds_max_fee"));
        }
        u128::try_from(maximum).map_err(|_| EvmWalletError::Invalid("max_fee_width"))?;
        u128::try_from(priority).map_err(|_| EvmWalletError::Invalid("priority_fee_width"))?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletFeeCandidate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            max_fee_per_gas: String,
            max_priority_fee_per_gas: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let candidate = Self {
            max_fee_per_gas: wire.max_fee_per_gas,
            max_priority_fee_per_gas: wire.max_priority_fee_per_gas,
        };
        candidate.validate().map_err(de::Error::custom)?;
        Ok(candidate)
    }
}

/// Finite, strictly monotonic replacement schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-replacement-policy",
    version = "1",
    schema = "mfm.evm.wallet_replacement_policy"
)]
pub struct EvmWalletReplacementPolicy {
    fee_candidates: Vec<EvmWalletFeeCandidate>,
}

impl EvmWalletReplacementPolicy {
    /// Creates a non-empty finite fee schedule.
    pub fn new(fee_candidates: Vec<EvmWalletFeeCandidate>) -> Result<Self, EvmWalletError> {
        let policy = Self { fee_candidates };
        policy.validate()?;
        Ok(policy)
    }

    /// Returns fee candidates in immutable attempt order.
    pub fn fee_candidates(&self) -> &[EvmWalletFeeCandidate] {
        &self.fee_candidates
    }

    /// Returns one candidate by exact schedule ordinal.
    pub fn candidate(&self, ordinal: u16) -> Option<&EvmWalletFeeCandidate> {
        self.fee_candidates.get(usize::from(ordinal))
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        if self.fee_candidates.is_empty()
            || self.fee_candidates.len() > EVM_WALLET_REPLACEMENT_LIMIT
        {
            return Err(EvmWalletError::InvalidReplacementSchedule);
        }
        for candidate in &self.fee_candidates {
            candidate.validate()?;
        }
        for pair in self.fee_candidates.windows(2) {
            if pair[0].max_fee_quantity()? >= pair[1].max_fee_quantity()?
                || pair[0].max_priority_fee_quantity()? >= pair[1].max_priority_fee_quantity()?
            {
                return Err(EvmWalletError::InvalidReplacementSchedule);
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletReplacementPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            fee_candidates: Vec<EvmWalletFeeCandidate>,
        }

        Self::new(Wire::deserialize(deserializer)?.fee_candidates).map_err(de::Error::custom)
    }
}

/// Fixed integer-bounded convergence script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-convergence-plan",
    version = "1",
    schema = "mfm.evm.wallet_convergence_plan"
)]
pub struct EvmWalletConvergencePlan {
    broadcasts_per_candidate: u16,
    transaction_lookups_per_candidate: u16,
    receipt_lookups_per_candidate: u16,
    finalized_head_lookups: u16,
    canonical_inclusion_lookups: u16,
    max_attempt_result_bytes: u64,
}

impl EvmWalletConvergencePlan {
    /// Creates one finite, clock-free convergence script.
    pub fn new(
        broadcasts_per_candidate: u16,
        transaction_lookups_per_candidate: u16,
        receipt_lookups_per_candidate: u16,
        finalized_head_lookups: u16,
        canonical_inclusion_lookups: u16,
        max_attempt_result_bytes: u64,
    ) -> Result<Self, EvmWalletError> {
        let plan = Self {
            broadcasts_per_candidate,
            transaction_lookups_per_candidate,
            receipt_lookups_per_candidate,
            finalized_head_lookups,
            canonical_inclusion_lookups,
            max_attempt_result_bytes,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Returns the maximum broadcasts or rebroadcasts per fee candidate.
    pub const fn broadcasts_per_candidate(&self) -> u16 {
        self.broadcasts_per_candidate
    }

    /// Returns the maximum transaction lookups per fee candidate.
    pub const fn transaction_lookups_per_candidate(&self) -> u16 {
        self.transaction_lookups_per_candidate
    }

    /// Returns the maximum receipt lookups per fee candidate.
    pub const fn receipt_lookups_per_candidate(&self) -> u16 {
        self.receipt_lookups_per_candidate
    }

    /// Returns the finite finalized-head lookup budget.
    pub const fn finalized_head_lookups(&self) -> u16 {
        self.finalized_head_lookups
    }

    /// Returns the finite canonical-inclusion lookup budget.
    pub const fn canonical_inclusion_lookups(&self) -> u16 {
        self.canonical_inclusion_lookups
    }

    /// Returns the admitted canonical size of any returned attempt result.
    pub const fn max_attempt_result_bytes(&self) -> u64 {
        self.max_attempt_result_bytes
    }

    /// Computes the script's maximum target-entry count.
    pub fn max_attempts(&self, fee_candidates: usize) -> Result<u32, EvmWalletError> {
        let fee_candidates =
            u32::try_from(fee_candidates).map_err(|_| EvmWalletError::BoundExceeded("attempts"))?;
        let per_candidate = u32::from(self.broadcasts_per_candidate)
            .checked_add(u32::from(self.transaction_lookups_per_candidate))
            .and_then(|value| value.checked_add(u32::from(self.receipt_lookups_per_candidate)))
            .ok_or(EvmWalletError::BoundExceeded("attempts"))?;
        fee_candidates
            .checked_mul(per_candidate)
            .and_then(|value| value.checked_add(u32::from(self.finalized_head_lookups)))
            .and_then(|value| value.checked_add(u32::from(self.canonical_inclusion_lookups)))
            .ok_or(EvmWalletError::BoundExceeded("attempts"))
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        if self.broadcasts_per_candidate == 0
            || self.transaction_lookups_per_candidate == 0
            || self.receipt_lookups_per_candidate == 0
            || self.finalized_head_lookups == 0
            || self.canonical_inclusion_lookups == 0
            || self.max_attempt_result_bytes == 0
        {
            return Err(EvmWalletError::Invalid("convergence_plan"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-policy",
    version = "1",
    schema = "mfm.evm.wallet_policy"
)]
struct EvmWalletPolicyWire {
    wallet_domain_ref: EvmWalletReference,
    tenant_scope_id: String,
    route_generation_ref: EvmWalletReference,
    chain_id: u64,
    sender: String,
    signer_binding_ref: EvmWalletReference,
    initial_nonce: String,
    initial_nonce_attestation_ref: EvmWalletReference,
    replacement: EvmWalletReplacementPolicy,
    already_known_classifier_ref: EvmWalletReference,
    finality_policy_ref: EvmWalletReference,
    assurance_policy_ref: EvmWalletReference,
    convergence: EvmWalletConvergencePlan,
    evidence_max_attempts: u32,
    evidence_max_records: u32,
    evidence_max_retained_bytes: String,
    evidence_completion_reserve_records: u32,
    evidence_completion_reserve_bytes: String,
}

/// Immutable wallet domain, allocation, replacement, and finality policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmWalletPolicy {
    wire: EvmWalletPolicyWire,
    evidence_bounds: EvidenceBounds,
}

impl EvmWalletPolicy {
    /// Creates one exact wallet policy and validates its full finite script.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        wallet_domain_ref: EvmWalletReference,
        tenant_scope_id: TenantScopeId,
        route_generation_ref: EvmWalletReference,
        chain_id: u64,
        sender: Address,
        signer_binding_ref: EvmWalletReference,
        initial_nonce: u64,
        initial_nonce_attestation_ref: EvmWalletReference,
        replacement: EvmWalletReplacementPolicy,
        already_known_classifier_ref: EvmWalletReference,
        finality_policy_ref: EvmWalletReference,
        assurance_policy_ref: EvmWalletReference,
        convergence: EvmWalletConvergencePlan,
        evidence_bounds: EvidenceBounds,
    ) -> Result<Self, EvmWalletError> {
        let wire = EvmWalletPolicyWire {
            wallet_domain_ref,
            tenant_scope_id: tenant_scope_id.as_str().to_owned(),
            route_generation_ref,
            chain_id,
            sender: canonical_address(sender),
            signer_binding_ref,
            initial_nonce: initial_nonce.to_string(),
            initial_nonce_attestation_ref,
            replacement,
            already_known_classifier_ref,
            finality_policy_ref,
            assurance_policy_ref,
            convergence,
            evidence_max_attempts: evidence_bounds.max_attempts(),
            evidence_max_records: evidence_bounds.max_records(),
            evidence_max_retained_bytes: evidence_bounds.max_retained_bytes().to_string(),
            evidence_completion_reserve_records: evidence_bounds.completion_reserve_records(),
            evidence_completion_reserve_bytes: evidence_bounds
                .completion_reserve_bytes()
                .to_string(),
        };
        let policy = Self {
            wire,
            evidence_bounds,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Returns the exact externally coordinated wallet resource domain.
    pub const fn wallet_domain_ref(&self) -> &EvmWalletReference {
        &self.wire.wallet_domain_ref
    }

    /// Returns the exact tenant partition.
    pub fn tenant_scope_id(&self) -> Result<TenantScopeId, EvmWalletError> {
        TenantScopeId::from_str(&self.wire.tenant_scope_id)
            .map_err(|_| EvmWalletError::Invalid("tenant_scope_id"))
    }

    /// Returns the immutable live-route generation.
    pub const fn route_generation_ref(&self) -> &EvmWalletReference {
        &self.wire.route_generation_ref
    }

    /// Returns the exact chain id.
    pub const fn chain_id(&self) -> u64 {
        self.wire.chain_id
    }

    /// Returns the canonical sender.
    pub fn sender(&self) -> &str {
        &self.wire.sender
    }

    /// Parses the checked sender.
    pub fn sender_address(&self) -> Result<Address, EvmWalletError> {
        parse_address(&self.wire.sender)
    }

    /// Returns the qualified wallet signer-binding identity.
    pub const fn signer_binding_ref(&self) -> &EvmWalletReference {
        &self.wire.signer_binding_ref
    }

    /// Returns the externally attested first unused nonce.
    pub fn initial_nonce(&self) -> Result<u64, EvmWalletError> {
        parse_quantity(&self.wire.initial_nonce)?
            .try_into()
            .map_err(|_| EvmWalletError::Invalid("initial_nonce"))
    }

    /// Returns the attestation for the first unused nonce.
    pub const fn initial_nonce_attestation_ref(&self) -> &EvmWalletReference {
        &self.wire.initial_nonce_attestation_ref
    }

    /// Returns the finite replacement schedule.
    pub const fn replacement(&self) -> &EvmWalletReplacementPolicy {
        &self.wire.replacement
    }

    /// Returns the sole admitted exact already-known classifier.
    pub const fn already_known_classifier_ref(&self) -> &EvmWalletReference {
        &self.wire.already_known_classifier_ref
    }

    /// Returns the finalized-tag finality policy.
    pub const fn finality_policy_ref(&self) -> &EvmWalletReference {
        &self.wire.finality_policy_ref
    }

    /// Returns the exact terminal assurance policy.
    pub const fn assurance_policy_ref(&self) -> &EvmWalletReference {
        &self.wire.assurance_policy_ref
    }

    /// Returns the fixed integer-bounded convergence plan.
    pub const fn convergence(&self) -> &EvmWalletConvergencePlan {
        &self.wire.convergence
    }

    /// Returns the executor's exact finite evidence bounds.
    pub const fn evidence_bounds(&self) -> &EvidenceBounds {
        &self.evidence_bounds
    }

    /// Returns the maximum IO authorizations in the complete script.
    pub fn max_attempts(&self) -> Result<u32, EvmWalletError> {
        self.wire
            .convergence
            .max_attempts(self.wire.replacement.fee_candidates().len())
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        self.wire.wallet_domain_ref.to_content_ref()?;
        self.tenant_scope_id()?;
        self.wire.route_generation_ref.to_content_ref()?;
        if self.wire.chain_id == 0 || self.sender_address()?.is_zero() {
            return Err(EvmWalletError::Invalid("wallet_policy_identity"));
        }
        self.wire.signer_binding_ref.to_content_ref()?;
        self.initial_nonce()?;
        self.wire.initial_nonce_attestation_ref.to_content_ref()?;
        self.wire.replacement.validate()?;
        self.wire.already_known_classifier_ref.to_content_ref()?;
        self.wire.finality_policy_ref.to_content_ref()?;
        self.wire.assurance_policy_ref.to_content_ref()?;
        self.wire.convergence.validate()?;
        if self.wire.evidence_max_attempts != self.evidence_bounds.max_attempts()
            || self.wire.evidence_max_records != self.evidence_bounds.max_records()
            || self.wire.evidence_max_retained_bytes
                != self.evidence_bounds.max_retained_bytes().to_string()
            || self.wire.evidence_completion_reserve_records
                != self.evidence_bounds.completion_reserve_records()
            || self.wire.evidence_completion_reserve_bytes
                != self.evidence_bounds.completion_reserve_bytes().to_string()
        {
            return Err(EvmWalletError::Invalid("evidence_bounds"));
        }
        self.validate_evidence_budget(0)
    }

    fn validate_evidence_budget(&self, request_bytes: usize) -> Result<(), EvmWalletError> {
        let attempts = self.max_attempts()?;
        if attempts > self.evidence_bounds.max_attempts() {
            return Err(EvmWalletError::BoundExceeded("evidence_attempts"));
        }
        let records = attempts
            .checked_mul(2)
            .and_then(|value| value.checked_add(3))
            .ok_or(EvmWalletError::BoundExceeded("evidence_records"))?;
        if records > self.evidence_bounds.max_records() {
            return Err(EvmWalletError::BoundExceeded("evidence_records"));
        }
        let attempt_bytes = u64::from(attempts)
            .checked_mul(self.wire.convergence.max_attempt_result_bytes)
            .ok_or(EvmWalletError::BoundExceeded("evidence_bytes"))?;
        let wrapper_bytes = u64::from(records)
            .checked_mul(EVM_WALLET_EXECUTOR_RECORD_OVERHEAD_BYTES)
            .ok_or(EvmWalletError::BoundExceeded("evidence_bytes"))?;
        let lineage_copies = u64::try_from(self.wire.replacement.fee_candidates().len() + 1)
            .map_err(|_| EvmWalletError::BoundExceeded("evidence_bytes"))?;
        let request_bytes = u64::try_from(request_bytes)
            .map_err(|_| EvmWalletError::BoundExceeded("evidence_bytes"))?
            .checked_mul(lineage_copies)
            .ok_or(EvmWalletError::BoundExceeded("evidence_bytes"))?;
        let required = attempt_bytes
            .checked_add(wrapper_bytes)
            .and_then(|value| value.checked_add(request_bytes))
            .and_then(|value| {
                value.checked_add(
                    u64::try_from(self.evidence_bounds.completion_reserve_bytes()).ok()?,
                )
            })
            .ok_or(EvmWalletError::BoundExceeded("evidence_bytes"))?;
        if required
            > u64::try_from(self.evidence_bounds.max_retained_bytes())
                .map_err(|_| EvmWalletError::BoundExceeded("evidence_bytes"))?
        {
            return Err(EvmWalletError::BoundExceeded("evidence_bytes"));
        }
        Ok(())
    }
}

impl Serialize for EvmWalletPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EvmWalletPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EvmWalletPolicyWire::deserialize(deserializer)?;
        let max_retained_bytes = wire
            .evidence_max_retained_bytes
            .parse::<u64>()
            .map_err(de::Error::custom)?;
        let completion_reserve_bytes = wire
            .evidence_completion_reserve_bytes
            .parse::<u64>()
            .map_err(de::Error::custom)?;
        let evidence_bounds = EvidenceBounds::new(
            wire.evidence_max_attempts,
            wire.evidence_max_records,
            max_retained_bytes,
            wire.evidence_completion_reserve_records,
            completion_reserve_bytes,
        )
        .map_err(de::Error::custom)?;
        let policy = Self {
            wire,
            evidence_bounds,
        };
        policy.validate().map_err(de::Error::custom)?;
        Ok(policy)
    }
}

impl mfm_values::MfmValue for EvmWalletPolicy {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        EvmWalletPolicyWire::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        EvmWalletPolicyWire::semantic_id()
    }
}

/// Complete immutable configured type-2 transaction template.
///
/// The externally attested first-unused nonce is fixed before admission. Live
/// pending-nonce bootstrap, fee estimation, gas estimation, and route
/// substitution are not represented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-transaction-template",
    version = "1",
    schema = "mfm.evm.wallet_transaction_template",
    validate = "validate_wallet_template_config"
)]
pub struct EvmWalletTransactionTemplate {
    target: EvmTransactionTarget,
    action: EvmWalletTransactionAction,
    value: String,
    input: String,
    access_list: Vec<EvmWalletAccessListEntry>,
    gas_limit: String,
}

impl EvmWalletTransactionTemplate {
    /// Creates one fully fixed type-2 transaction template.
    pub fn new(
        target: EvmTransactionTarget,
        action: EvmWalletTransactionAction,
        value: U256,
        input: impl AsRef<[u8]>,
        access_list: Vec<EvmWalletAccessListEntry>,
        gas_limit: U256,
    ) -> Result<Self, EvmWalletError> {
        let template = Self {
            target,
            action,
            value: value.to_string(),
            input: canonical_bytes(input.as_ref()),
            access_list,
            gas_limit: gas_limit.to_string(),
        };
        template.validate()?;
        Ok(template)
    }

    /// Returns the configured-value target.
    pub const fn target(&self) -> &EvmTransactionTarget {
        &self.target
    }

    /// Returns the exact envelope type.
    pub const fn transaction_type(&self) -> u8 {
        EVM_WALLET_TRANSACTION_TYPE
    }

    /// Returns the direct call or creation action.
    pub const fn action(&self) -> &EvmWalletTransactionAction {
        &self.action
    }

    /// Returns the canonical wei value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Parses the checked wei value.
    pub fn value_quantity(&self) -> Result<U256, EvmWalletError> {
        parse_quantity(&self.value)
    }

    /// Returns the canonical lower-case call data or creation init code.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Parses the exact transaction input bytes.
    pub fn input_bytes(&self) -> Result<Bytes, EvmWalletError> {
        parse_bytes(&self.input, EVM_WALLET_DATA_MAX_BYTES).map(Into::into)
    }

    /// Returns the exact signed access-list order.
    pub fn access_list(&self) -> &[EvmWalletAccessListEntry] {
        &self.access_list
    }

    /// Converts the exact access list to Alloy.
    pub fn access_list_alloy(&self) -> Result<AccessList, EvmWalletError> {
        self.access_list
            .iter()
            .map(EvmWalletAccessListEntry::to_alloy)
            .collect::<Result<Vec<_>, _>>()
            .map(AccessList)
    }

    /// Returns the canonical fixed gas limit.
    pub fn gas_limit(&self) -> &str {
        &self.gas_limit
    }

    /// Parses the checked fixed gas limit.
    pub fn gas_limit_quantity(&self) -> Result<U256, EvmWalletError> {
        parse_quantity(&self.gas_limit)
    }

    /// Revalidates all public template material.
    pub fn validate(&self) -> Result<(), EvmWalletError> {
        LocalPublicId::new(self.target.as_str())
            .map_err(|_| EvmWalletError::Invalid("transaction_target"))?;
        self.action.validate()?;
        self.value_quantity()?;
        let input = self.input_bytes()?;
        if matches!(self.action, EvmWalletTransactionAction::Create) && input.is_empty() {
            return Err(EvmWalletError::Invalid("empty_init_code"));
        }
        let gas = self.gas_limit_quantity()?;
        let gas = u64::try_from(gas).map_err(|_| EvmWalletError::Invalid("gas_limit_width"))?;
        if gas == 0 {
            return Err(EvmWalletError::Invalid("gas_limit"));
        }
        if self.access_list.len() > EVM_WALLET_ACCESS_LIST_MAX_ENTRIES {
            return Err(EvmWalletError::BoundExceeded("access_list_entries"));
        }
        let mut address_set = BTreeSet::new();
        let mut storage_key_count = 0_usize;
        for entry in &self.access_list {
            entry.validate()?;
            if !address_set.insert(entry.address()) {
                return Err(EvmWalletError::Invalid("duplicate_access_list_address"));
            }
            storage_key_count = storage_key_count
                .checked_add(entry.storage_keys().len())
                .ok_or(EvmWalletError::BoundExceeded("access_list_storage_keys"))?;
        }
        if storage_key_count > EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS {
            return Err(EvmWalletError::BoundExceeded("access_list_storage_keys"));
        }
        Ok(())
    }
}

fn validate_wallet_template_config(template: &EvmWalletTransactionTemplate) -> Result<(), String> {
    template.validate().map_err(|error| error.to_string())
}

impl<'de> Deserialize<'de> for EvmWalletTransactionTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            target: EvmTransactionTarget,
            action: EvmWalletTransactionAction,
            value: String,
            input: String,
            access_list: Vec<EvmWalletAccessListEntry>,
            gas_limit: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let template = Self {
            target: wire.target,
            action: wire.action,
            value: wire.value,
            input: wire.input,
            access_list: wire.access_list,
            gas_limit: wire.gas_limit,
        };
        template.validate().map_err(de::Error::custom)?;
        Ok(template)
    }
}

/// Exact state input binding the public selector to its configured template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.submit_transaction", version = "1")]
pub struct EvmSubmitTransactionInput {
    request: EvmSubmitTransactionRequest,
    selector: EvmSubmitTransactionSelector,
}

impl EvmSubmitTransactionInput {
    /// Binds one configured semantic request to its public selector.
    pub fn new(
        request: EvmSubmitTransactionRequest,
        selector: EvmSubmitTransactionSelector,
    ) -> Result<Self, EvmWalletError> {
        Self::from_request(request, selector)
    }

    /// Binds one already committed semantic request to its public selector.
    pub fn from_request(
        request: EvmSubmitTransactionRequest,
        selector: EvmSubmitTransactionSelector,
    ) -> Result<Self, EvmWalletError> {
        if request.template().target() != selector.target() {
            return Err(EvmWalletError::Invalid("configured_target_mismatch"));
        }
        Ok(Self { request, selector })
    }

    /// Returns the immutable configured template.
    pub fn template(&self) -> &EvmWalletTransactionTemplate {
        self.request.template()
    }

    /// Returns the already canonicalized semantic request.
    pub const fn request(&self) -> &EvmSubmitTransactionRequest {
        &self.request
    }

    /// Returns the public selector.
    pub const fn selector(&self) -> &EvmSubmitTransactionSelector {
        &self.selector
    }
}

impl<'de> Deserialize<'de> for EvmSubmitTransactionInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: EvmSubmitTransactionRequest,
            selector: EvmSubmitTransactionSelector,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_request(wire.request, wire.selector).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submit-transaction-request",
    version = "1",
    schema = "mfm.evm.submit_transaction_request"
)]
struct EvmSubmitTransactionRequestWire {
    version: String,
    template_ref: EvmWalletReference,
    policy_ref: EvmWalletReference,
    template: EvmWalletTransactionTemplate,
    policy: EvmWalletPolicy,
    wallet_domain_ref: EvmWalletReference,
    tenant_scope_id: String,
    route_generation_ref: EvmWalletReference,
    chain_id: u64,
    sender: String,
    signer_binding_ref: EvmWalletReference,
}

/// Immutable semantic request committed before any wallet access.
///
/// This value contains no endpoint, credential, signature, signed envelope,
/// provider body, key material, or secret-bearing path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSubmitTransactionRequest {
    wire: EvmSubmitTransactionRequestWire,
    canonical: SchemaQualifiedCanonicalValue,
}

impl EvmSubmitTransactionRequest {
    /// Returns the exact already committed request from validated state input.
    pub fn from_input(input: &EvmSubmitTransactionInput) -> Self {
        input.request.clone()
    }

    /// Constructs the exact request from separately referenced template and policy objects.
    pub fn new(
        template_ref: EvmWalletReference,
        template: EvmWalletTransactionTemplate,
        policy_ref: EvmWalletReference,
        policy: EvmWalletPolicy,
    ) -> Result<Self, EvmWalletError> {
        template.validate()?;
        policy.validate()?;
        if template_ref != wallet_value_reference(&template)?
            || policy_ref != wallet_value_reference(&policy)?
        {
            return Err(EvmWalletError::Invalid("request_object_reference"));
        }
        let wire = EvmSubmitTransactionRequestWire {
            version: "mfm.evm.submit-transaction-request.v1".to_owned(),
            template_ref,
            policy_ref,
            template,
            wallet_domain_ref: policy.wallet_domain_ref().clone(),
            tenant_scope_id: policy.tenant_scope_id()?.as_str().to_owned(),
            route_generation_ref: policy.route_generation_ref().clone(),
            chain_id: policy.chain_id(),
            sender: policy.sender().to_owned(),
            signer_binding_ref: policy.signer_binding_ref().clone(),
            policy,
        };
        validate_request_wire(&wire)?;
        let canonical = encode_boundary(&wire).map_err(|_| EvmWalletError::RequestEncoding)?;
        let schema = <Self as mfm_values::MfmValue>::schema_id()
            .map_err(|_| EvmWalletError::RequestEncoding)?;
        let canonical = SchemaQualifiedCanonicalValue::new(schema, canonical.as_bytes())
            .map_err(|_| EvmWalletError::RequestEncoding)?;
        wire.policy
            .validate_evidence_budget(canonical.as_bytes().len())?;
        Ok(Self { wire, canonical })
    }

    /// Returns the fixed template.
    pub const fn template(&self) -> &EvmWalletTransactionTemplate {
        &self.wire.template
    }

    /// Returns the exact referenced wallet policy.
    pub const fn policy(&self) -> &EvmWalletPolicy {
        &self.wire.policy
    }

    /// Returns the immutable template object identity.
    pub const fn template_ref(&self) -> &EvmWalletReference {
        &self.wire.template_ref
    }

    /// Returns the immutable policy object identity.
    pub const fn policy_ref(&self) -> &EvmWalletReference {
        &self.wire.policy_ref
    }

    /// Reconstructs one exact unsigned candidate from an allocated nonce and fee ordinal.
    pub fn unsigned_envelope(
        &self,
        allocated_nonce: u64,
        fee_ordinal: u16,
    ) -> Result<crate::UnsignedEip1559Envelope, EvmWalletError> {
        if allocated_nonce < self.policy().initial_nonce()? {
            return Err(EvmWalletError::Invalid("allocated_nonce"));
        }
        let fee = self
            .policy()
            .replacement()
            .candidate(fee_ordinal)
            .ok_or(EvmWalletError::Invalid("fee_ordinal"))?;
        crate::UnsignedEip1559Envelope::new(
            U256::from(self.policy().chain_id()),
            U256::from(allocated_nonce),
            fee.max_priority_fee_quantity()?,
            fee.max_fee_quantity()?,
            self.template().gas_limit_quantity()?,
            self.template().action().to_alloy()?,
            self.template().value_quantity()?,
            self.template().access_list_alloy()?,
            self.template().input_bytes()?,
        )
        .map_err(|_| EvmWalletError::InconsistentEvidence)
    }

    /// Strictly reconstructs a request from its canonical value.
    pub fn strict_decode(canonical: &[u8]) -> Result<Self, EvmWalletError> {
        let canonical =
            mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(canonical)
                .map_err(|_| EvmWalletError::RequestEncoding)?;
        let wire = decode_boundary::<EvmSubmitTransactionRequestWire>(&canonical)
            .map_err(|_| EvmWalletError::RequestEncoding)?;
        Self::new(
            wire.template_ref,
            wire.template,
            wire.policy_ref,
            wire.policy,
        )
    }
}

impl Serialize for EvmSubmitTransactionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EvmSubmitTransactionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EvmSubmitTransactionRequestWire::deserialize(deserializer)?;
        Self::new(
            wire.template_ref,
            wire.template,
            wire.policy_ref,
            wire.policy,
        )
        .map_err(de::Error::custom)
    }
}

impl mfm_values::MfmValue for EvmSubmitTransactionRequest {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        EvmSubmitTransactionRequestWire::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        EvmSubmitTransactionRequestWire::semantic_id()
    }
}

impl CanonicalExecutorRequest for EvmSubmitTransactionRequest {
    fn canonical_request(&self) -> &SchemaQualifiedCanonicalValue {
        &self.canonical
    }
}

/// One schedule-selected transaction candidate.
///
/// Candidate reconstruction is deterministic: every field other than the
/// selected fee pair comes from the immutable template. The transaction hash
/// is public signer/executor evidence; the signature and signed bytes remain
/// transient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-transaction-candidate",
    version = "1",
    schema = "mfm.evm.wallet_transaction_candidate"
)]
pub struct EvmWalletTransactionCandidate {
    request: EvmSubmitTransactionRequest,
    allocated_nonce: String,
    fee_ordinal: u16,
    transaction_hash: String,
}

impl EvmWalletTransactionCandidate {
    /// Binds one schedule ordinal to its exact signed transaction hash.
    pub fn new(
        request: EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        fee_ordinal: u16,
        transaction_hash: B256,
    ) -> Result<Self, EvmWalletError> {
        let candidate = Self {
            request,
            allocated_nonce: allocated_nonce.to_string(),
            fee_ordinal,
            transaction_hash: canonical_hash(transaction_hash),
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Returns the immutable request template.
    pub fn template(&self) -> &EvmWalletTransactionTemplate {
        self.request.template()
    }

    /// Returns the complete immutable semantic request.
    pub const fn request(&self) -> &EvmSubmitTransactionRequest {
        &self.request
    }

    /// Returns the immutable wallet policy.
    pub fn policy(&self) -> &EvmWalletPolicy {
        self.request.policy()
    }

    /// Returns the permanently allocated account nonce.
    pub fn allocated_nonce(&self) -> Result<u64, EvmWalletError> {
        parse_quantity(&self.allocated_nonce)?
            .try_into()
            .map_err(|_| EvmWalletError::Invalid("allocated_nonce"))
    }

    /// Returns the selected fee ordinal.
    pub const fn fee_ordinal(&self) -> u16 {
        self.fee_ordinal
    }

    /// Returns the exact selected fee pair.
    pub fn fee(&self) -> Result<&EvmWalletFeeCandidate, EvmWalletError> {
        self.policy()
            .replacement()
            .candidate(self.fee_ordinal)
            .ok_or(EvmWalletError::Invalid("fee_ordinal"))
    }

    /// Returns the canonical signed transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Parses the checked signed transaction hash.
    pub fn transaction_hash_value(&self) -> Result<B256, EvmWalletError> {
        parse_hash(&self.transaction_hash)
    }

    /// Returns this candidate's schema-qualified public content identity.
    pub fn reference(&self) -> Result<EvmWalletReference, EvmWalletError> {
        wallet_value_reference(self)
    }

    /// Reconstructs the exact unsigned Alloy envelope.
    pub fn unsigned_envelope(&self) -> Result<crate::UnsignedEip1559Envelope, EvmWalletError> {
        self.request
            .unsigned_envelope(self.allocated_nonce()?, self.fee_ordinal)
    }

    /// Revalidates the immutable candidate relation.
    pub fn validate(&self) -> Result<(), EvmWalletError> {
        self.request.template().validate()?;
        self.request.policy().validate()?;
        if self.allocated_nonce()? < self.policy().initial_nonce()? {
            return Err(EvmWalletError::Invalid("allocated_nonce"));
        }
        self.fee()?;
        self.transaction_hash_value()?;
        self.unsigned_envelope()?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletTransactionCandidate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: EvmSubmitTransactionRequest,
            allocated_nonce: String,
            fee_ordinal: u16,
            transaction_hash: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let candidate = Self {
            request: wire.request,
            allocated_nonce: wire.allocated_nonce,
            fee_ordinal: wire.fee_ordinal,
            transaction_hash: wire.transaction_hash,
        };
        candidate.validate().map_err(de::Error::custom)?;
        Ok(candidate)
    }
}

/// Exact inclusion placement of an observed transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-transaction-placement",
    version = "1",
    schema = "mfm.evm.wallet_transaction_placement"
)]
pub struct EvmWalletTransactionPlacement {
    block: EvmBlockAnchor,
    transaction_index: String,
}

impl EvmWalletTransactionPlacement {
    /// Creates one exact block placement.
    pub fn new(block: EvmBlockAnchor, transaction_index: U256) -> Result<Self, EvmWalletError> {
        block
            .validate()
            .map_err(|_| EvmWalletError::Invalid("placement_block"))?;
        Ok(Self {
            block,
            transaction_index: transaction_index.to_string(),
        })
    }

    /// Returns the exact block.
    pub const fn block(&self) -> &EvmBlockAnchor {
        &self.block
    }

    /// Returns the canonical transaction index.
    pub fn transaction_index(&self) -> &str {
        &self.transaction_index
    }

    /// Parses the checked transaction index.
    pub fn transaction_index_quantity(&self) -> Result<U256, EvmWalletError> {
        parse_quantity(&self.transaction_index)
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        self.block
            .validate()
            .map_err(|_| EvmWalletError::Invalid("placement_block"))?;
        self.transaction_index_quantity()?;
        Ok(())
    }
}

/// Public fields returned by `eth_getTransactionByHash`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-observed-transaction",
    version = "1",
    schema = "mfm.evm.wallet_observed_transaction"
)]
pub struct EvmWalletObservedTransaction {
    transaction_hash: String,
    transaction_type: u8,
    chain_id: String,
    nonce: String,
    from: String,
    to: Option<String>,
    value: String,
    input: String,
    gas_limit: String,
    max_fee_per_gas: String,
    max_priority_fee_per_gas: String,
    access_list: Vec<EvmWalletAccessListEntry>,
    placement: Option<EvmWalletTransactionPlacement>,
}

impl EvmWalletObservedTransaction {
    /// Creates one checked public type-2 transaction observation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_hash: B256,
        chain_id: U256,
        nonce: U256,
        from: Address,
        to: TxKind,
        value: U256,
        input: impl AsRef<[u8]>,
        gas_limit: U256,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
        access_list: Vec<EvmWalletAccessListEntry>,
        placement: Option<EvmWalletTransactionPlacement>,
    ) -> Result<Self, EvmWalletError> {
        let transaction = Self {
            transaction_hash: canonical_hash(transaction_hash),
            transaction_type: EVM_WALLET_TRANSACTION_TYPE,
            chain_id: chain_id.to_string(),
            nonce: nonce.to_string(),
            from: canonical_address(from),
            to: match to {
                TxKind::Create => None,
                TxKind::Call(address) => Some(canonical_address(address)),
            },
            value: value.to_string(),
            input: canonical_bytes(input.as_ref()),
            gas_limit: gas_limit.to_string(),
            max_fee_per_gas: max_fee_per_gas.to_string(),
            max_priority_fee_per_gas: max_priority_fee_per_gas.to_string(),
            access_list,
            placement,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    /// Returns the canonical transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the exact transaction type.
    pub const fn transaction_type(&self) -> u8 {
        self.transaction_type
    }

    /// Returns optional inclusion placement.
    pub const fn placement(&self) -> Option<&EvmWalletTransactionPlacement> {
        self.placement.as_ref()
    }

    /// Returns whether every immutable public field equals one candidate.
    pub fn matches_candidate(
        &self,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<bool, EvmWalletError> {
        self.validate()?;
        candidate.validate()?;
        let template = candidate.template();
        let fee = candidate.fee()?;
        let expected_to = template.action().call_destination()?.map(canonical_address);
        Ok(self.transaction_hash == candidate.transaction_hash()
            && self.transaction_type == EVM_WALLET_TRANSACTION_TYPE
            && parse_quantity(&self.chain_id)? == U256::from(candidate.policy().chain_id())
            && parse_quantity(&self.nonce)? == U256::from(candidate.allocated_nonce()?)
            && parse_address(&self.from)? == candidate.policy().sender_address()?
            && self.to == expected_to
            && parse_quantity(&self.value)? == template.value_quantity()?
            && parse_bytes(&self.input, EVM_WALLET_DATA_MAX_BYTES)?
                == template.input_bytes()?.to_vec()
            && parse_quantity(&self.gas_limit)? == template.gas_limit_quantity()?
            && parse_quantity(&self.max_fee_per_gas)? == fee.max_fee_quantity()?
            && parse_quantity(&self.max_priority_fee_per_gas)?
                == fee.max_priority_fee_quantity()?
            && self.access_list == template.access_list)
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        parse_hash(&self.transaction_hash)?;
        if self.transaction_type != EVM_WALLET_TRANSACTION_TYPE {
            return Err(EvmWalletError::Invalid("observed_transaction_type"));
        }
        let chain_id = parse_quantity(&self.chain_id)?;
        if u64::try_from(chain_id)
            .ok()
            .filter(|value| *value != 0)
            .is_none()
        {
            return Err(EvmWalletError::Invalid("observed_chain_id"));
        }
        u64::try_from(parse_quantity(&self.nonce)?)
            .map_err(|_| EvmWalletError::Invalid("observed_nonce_width"))?;
        parse_address(&self.from)?;
        if let Some(to) = &self.to {
            parse_address(to)?;
        }
        parse_quantity(&self.value)?;
        parse_bytes(&self.input, EVM_WALLET_DATA_MAX_BYTES)?;
        let gas = u64::try_from(parse_quantity(&self.gas_limit)?)
            .map_err(|_| EvmWalletError::Invalid("observed_gas_limit_width"))?;
        if gas == 0 {
            return Err(EvmWalletError::Invalid("observed_gas_limit"));
        }
        let maximum = parse_quantity(&self.max_fee_per_gas)?;
        let priority = parse_quantity(&self.max_priority_fee_per_gas)?;
        if priority > maximum {
            return Err(EvmWalletError::Invalid("observed_fee_relation"));
        }
        u128::try_from(maximum).map_err(|_| EvmWalletError::Invalid("observed_max_fee_width"))?;
        u128::try_from(priority)
            .map_err(|_| EvmWalletError::Invalid("observed_priority_fee_width"))?;
        validate_access_list(&self.access_list)?;
        if let Some(placement) = &self.placement {
            placement.validate()?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletObservedTransaction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            transaction_hash: String,
            transaction_type: u8,
            chain_id: String,
            nonce: String,
            from: String,
            to: Option<String>,
            value: String,
            input: String,
            gas_limit: String,
            max_fee_per_gas: String,
            max_priority_fee_per_gas: String,
            access_list: Vec<EvmWalletAccessListEntry>,
            placement: Option<EvmWalletTransactionPlacement>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let transaction = Self {
            transaction_hash: wire.transaction_hash,
            transaction_type: wire.transaction_type,
            chain_id: wire.chain_id,
            nonce: wire.nonce,
            from: wire.from,
            to: wire.to,
            value: wire.value,
            input: wire.input,
            gas_limit: wire.gas_limit,
            max_fee_per_gas: wire.max_fee_per_gas,
            max_priority_fee_per_gas: wire.max_priority_fee_per_gas,
            access_list: wire.access_list,
            placement: wire.placement,
        };
        transaction.validate().map_err(de::Error::custom)?;
        Ok(transaction)
    }
}

/// Explicit top-level receipt execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-receipt-status",
    version = "1",
    schema = "mfm.evm.wallet_receipt_status"
)]
pub enum EvmWalletReceiptStatus {
    /// Top-level execution succeeded.
    Success,
    /// Top-level execution reverted after consuming nonce and gas.
    Reverted,
}

/// One complete receipt log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-receipt-log",
    version = "1",
    schema = "mfm.evm.wallet_receipt_log"
)]
pub struct EvmWalletReceiptLog {
    address: String,
    topics: Vec<String>,
    data: String,
    block: EvmBlockAnchor,
    transaction_hash: String,
    transaction_index: String,
    log_index: String,
    removed: bool,
}

impl EvmWalletReceiptLog {
    /// Creates one exact non-removed receipt log.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: Address,
        topics: Vec<B256>,
        data: impl AsRef<[u8]>,
        block: EvmBlockAnchor,
        transaction_hash: B256,
        transaction_index: U256,
        log_index: U256,
        removed: bool,
    ) -> Result<Self, EvmWalletError> {
        let log = Self {
            address: canonical_address(address),
            topics: topics.into_iter().map(canonical_hash).collect(),
            data: canonical_bytes(data.as_ref()),
            block,
            transaction_hash: canonical_hash(transaction_hash),
            transaction_index: transaction_index.to_string(),
            log_index: log_index.to_string(),
            removed,
        };
        log.validate()?;
        Ok(log)
    }

    /// Returns the exact enclosing block.
    pub const fn block(&self) -> &EvmBlockAnchor {
        &self.block
    }

    /// Returns the enclosing transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the enclosing transaction index.
    pub fn transaction_index(&self) -> &str {
        &self.transaction_index
    }

    /// Returns whether the provider marked this log removed.
    pub const fn removed(&self) -> bool {
        self.removed
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        parse_address(&self.address)?;
        if self.topics.len() > 4 {
            return Err(EvmWalletError::BoundExceeded("receipt_log_topics"));
        }
        for topic in &self.topics {
            parse_hash(topic)?;
        }
        parse_bytes(&self.data, EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES)?;
        self.block
            .validate()
            .map_err(|_| EvmWalletError::Invalid("receipt_log_block"))?;
        parse_hash(&self.transaction_hash)?;
        parse_quantity(&self.transaction_index)?;
        parse_quantity(&self.log_index)?;
        if self.removed {
            return Err(EvmWalletError::Invalid("removed_receipt_log"));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletReceiptLog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            address: String,
            topics: Vec<String>,
            data: String,
            block: EvmBlockAnchor,
            transaction_hash: String,
            transaction_index: String,
            log_index: String,
            removed: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        let log = Self {
            address: wire.address,
            topics: wire.topics,
            data: wire.data,
            block: wire.block,
            transaction_hash: wire.transaction_hash,
            transaction_index: wire.transaction_index,
            log_index: wire.log_index,
            removed: wire.removed,
        };
        log.validate().map_err(de::Error::custom)?;
        Ok(log)
    }
}

/// Fresh exact transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-receipt",
    version = "1",
    schema = "mfm.evm.wallet_receipt"
)]
pub struct EvmWalletReceipt {
    transaction_hash: String,
    transaction_index: String,
    block: EvmBlockAnchor,
    from: String,
    to: Option<String>,
    contract_address: Option<String>,
    status: EvmWalletReceiptStatus,
    gas_used: String,
    cumulative_gas_used: String,
    logs: Vec<EvmWalletReceiptLog>,
}

impl EvmWalletReceipt {
    /// Creates one fresh checked receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_hash: B256,
        transaction_index: U256,
        block: EvmBlockAnchor,
        from: Address,
        to: Option<Address>,
        contract_address: Option<Address>,
        status: EvmWalletReceiptStatus,
        gas_used: U256,
        cumulative_gas_used: U256,
        logs: Vec<EvmWalletReceiptLog>,
    ) -> Result<Self, EvmWalletError> {
        let receipt = Self {
            transaction_hash: canonical_hash(transaction_hash),
            transaction_index: transaction_index.to_string(),
            block,
            from: canonical_address(from),
            to: to.map(canonical_address),
            contract_address: contract_address.map(canonical_address),
            status,
            gas_used: gas_used.to_string(),
            cumulative_gas_used: cumulative_gas_used.to_string(),
            logs,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Returns the canonical transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the exact inclusion block.
    pub const fn block(&self) -> &EvmBlockAnchor {
        &self.block
    }

    /// Returns the explicit execution status.
    pub const fn status(&self) -> EvmWalletReceiptStatus {
        self.status
    }

    /// Returns the created contract address when present.
    pub fn contract_address(&self) -> Option<&str> {
        self.contract_address.as_deref()
    }

    /// Returns complete successful receipt logs.
    pub fn logs(&self) -> &[EvmWalletReceiptLog] {
        &self.logs
    }

    /// Returns whether receipt and transaction fields exactly match a candidate.
    pub fn matches_candidate(
        &self,
        candidate: &EvmWalletTransactionCandidate,
        transaction: &EvmWalletObservedTransaction,
    ) -> Result<bool, EvmWalletError> {
        self.validate()?;
        if !transaction.matches_candidate(candidate)? {
            return Ok(false);
        }
        let Some(placement) = transaction.placement() else {
            return Ok(false);
        };
        if self.transaction_hash != candidate.transaction_hash()
            || self.transaction_hash != transaction.transaction_hash
            || self.block != *placement.block()
            || parse_quantity(&self.transaction_index)? != placement.transaction_index_quantity()?
            || parse_address(&self.from)? != candidate.policy().sender_address()?
            || self.to
                != candidate
                    .template()
                    .action()
                    .call_destination()?
                    .map(canonical_address)
        {
            return Ok(false);
        }

        let expected_create = match (candidate.template().action(), self.status) {
            (EvmWalletTransactionAction::Create, EvmWalletReceiptStatus::Success) => {
                let nonce = candidate.allocated_nonce()?;
                Some(canonical_address(
                    candidate.policy().sender_address()?.create(nonce),
                ))
            }
            _ => None,
        };
        Ok(self.contract_address == expected_create)
    }

    fn validate(&self) -> Result<(), EvmWalletError> {
        parse_hash(&self.transaction_hash)?;
        parse_quantity(&self.transaction_index)?;
        self.block
            .validate()
            .map_err(|_| EvmWalletError::Invalid("receipt_block"))?;
        parse_address(&self.from)?;
        if let Some(to) = &self.to {
            parse_address(to)?;
        }
        if let Some(created) = &self.contract_address {
            parse_address(created)?;
        }
        parse_quantity(&self.gas_used)?;
        parse_quantity(&self.cumulative_gas_used)?;
        if self.logs.len() > EVM_WALLET_RECEIPT_LOG_LIMIT {
            return Err(EvmWalletError::BoundExceeded("receipt_logs"));
        }
        if matches!(self.status, EvmWalletReceiptStatus::Reverted) && !self.logs.is_empty() {
            return Err(EvmWalletError::InconsistentEvidence);
        }
        for log in &self.logs {
            log.validate()?;
            if log.transaction_hash != self.transaction_hash
                || log.transaction_index != self.transaction_index
                || log.block != self.block
            {
                return Err(EvmWalletError::InconsistentEvidence);
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmWalletReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            transaction_hash: String,
            transaction_index: String,
            block: EvmBlockAnchor,
            from: String,
            to: Option<String>,
            contract_address: Option<String>,
            status: EvmWalletReceiptStatus,
            gas_used: String,
            cumulative_gas_used: String,
            logs: Vec<EvmWalletReceiptLog>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let receipt = Self {
            transaction_hash: wire.transaction_hash,
            transaction_index: wire.transaction_index,
            block: wire.block,
            from: wire.from,
            to: wire.to,
            contract_address: wire.contract_address,
            status: wire.status,
            gas_used: wire.gas_used,
            cumulative_gas_used: wire.cumulative_gas_used,
            logs: wire.logs,
        };
        receipt.validate().map_err(de::Error::custom)?;
        Ok(receipt)
    }
}

/// One typed returned target-operation result.
///
/// Safe transport, cancellation, and generation-fence failures use the
/// generic executor failure algebra and therefore do not appear in this
/// domain union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-broadcast-status",
    version = "1",
    schema = "mfm.evm.wallet_broadcast_status"
)]
pub enum EvmWalletBroadcastStatus {
    /// The provider returned the exact candidate hash.
    Accepted,
    /// The provider response matched the sole reviewed already-known classifier.
    AlreadyKnown,
}

/// One typed result returned by exactly one wallet target operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-attempt-result",
    version = "1",
    schema = "mfm.evm-wallet-attempt-result"
)]
// The terminal composite deliberately remains directly nested in the frozen
// result union; an indirection wrapper would change that persisted schema.
#[allow(clippy::large_enum_variant)]
pub enum EvmWalletAttemptResult {
    /// One `eth_sendRawTransaction` exchange.
    Broadcast {
        /// Content identity of the exact candidate whose transient bytes were sent.
        candidate_ref: EvmWalletReference,
        /// Exact locally derived transaction hash.
        transaction_hash: String,
        /// Closed accepted or already-known result.
        status: EvmWalletBroadcastStatus,
        /// Exact classifier identity, present only for already-known.
        classifier_ref: Option<EvmWalletReference>,
    },
    /// Exact-hash transaction lookup returned present or absent.
    TransactionLookup {
        /// Exact candidate hash queried.
        transaction_hash: String,
        /// Returned public transaction fields, or `None`.
        transaction: Option<EvmWalletObservedTransaction>,
    },
    /// Exact-hash receipt lookup returned present or absent.
    ReceiptLookup {
        /// Exact candidate hash queried.
        transaction_hash: String,
        /// Fresh receipt, or `None` after disappearance/reorganization.
        receipt: Option<EvmWalletReceipt>,
    },
    /// Finalized-tag block lookup after one fresh receipt.
    FinalizedHead {
        /// Fresh strict block returned for the `finalized` tag.
        block: EvmBlockAnchor,
    },
    /// Inclusion-number block lookup; only `terminal: Some` is terminal.
    CanonicalInclusion {
        /// Current strict block at the receipt inclusion number, or `None`.
        block: Option<EvmBlockAnchor>,
        /// Complete terminal composite when every exact relation is fresh.
        terminal: Option<EvmWalletTerminalEvidence>,
    },
}

impl EvmWalletAttemptResult {
    /// Revalidates the exact typed result without inferring finality.
    pub fn validate(&self) -> Result<(), EvmWalletError> {
        match self {
            Self::Broadcast {
                candidate_ref,
                transaction_hash,
                status,
                classifier_ref,
            } => {
                candidate_ref.to_content_ref()?;
                parse_hash(transaction_hash)?;
                match (status, classifier_ref) {
                    (EvmWalletBroadcastStatus::Accepted, None) => {}
                    (EvmWalletBroadcastStatus::AlreadyKnown, Some(reference)) => {
                        reference.to_content_ref()?;
                    }
                    _ => {
                        return Err(EvmWalletError::InconsistentEvidence);
                    }
                }
            }
            Self::TransactionLookup {
                transaction_hash,
                transaction,
            } => {
                parse_hash(transaction_hash)?;
                if let Some(transaction) = transaction {
                    transaction.validate()?;
                    if transaction.transaction_hash() != transaction_hash {
                        return Err(EvmWalletError::InconsistentEvidence);
                    }
                }
            }
            Self::ReceiptLookup {
                transaction_hash,
                receipt,
            } => {
                parse_hash(transaction_hash)?;
                if let Some(receipt) = receipt {
                    receipt.validate()?;
                    if receipt.transaction_hash() != transaction_hash {
                        return Err(EvmWalletError::InconsistentEvidence);
                    }
                }
            }
            Self::FinalizedHead { block } => {
                block
                    .validate()
                    .map_err(|_| EvmWalletError::Invalid("finalized_head"))?;
            }
            Self::CanonicalInclusion { block, terminal } => {
                if let Some(block) = block {
                    block
                        .validate()
                        .map_err(|_| EvmWalletError::Invalid("inclusion_block"))?;
                }
                if let Some(terminal) = terminal {
                    terminal.outcome()?;
                    if block.as_ref() != Some(terminal.inclusion_block()) {
                        return Err(EvmWalletError::InconsistentEvidence);
                    }
                }
            }
        }
        Ok(())
    }

    /// Verifies a candidate-bound result against the exact immutable request.
    pub fn validate_for_candidate(
        &self,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<(), EvmWalletError> {
        self.validate()?;
        candidate.validate()?;
        match self {
            Self::Broadcast {
                candidate_ref,
                transaction_hash,
                status,
                classifier_ref,
            } => {
                if candidate_ref != &candidate.reference()?
                    || transaction_hash != candidate.transaction_hash()
                {
                    return Err(EvmWalletError::InconsistentEvidence);
                }
                match status {
                    EvmWalletBroadcastStatus::Accepted => {}
                    EvmWalletBroadcastStatus::AlreadyKnown => {
                        if classifier_ref.as_ref()
                            != Some(candidate.policy().already_known_classifier_ref())
                        {
                            return Err(EvmWalletError::InconsistentEvidence);
                        }
                    }
                }
            }
            Self::TransactionLookup {
                transaction_hash,
                transaction,
            } => {
                if transaction_hash != candidate.transaction_hash() {
                    return Err(EvmWalletError::InconsistentEvidence);
                }
                if let Some(transaction) = transaction {
                    if !transaction.matches_candidate(candidate)? {
                        return Err(EvmWalletError::InconsistentEvidence);
                    }
                }
            }
            Self::ReceiptLookup {
                transaction_hash,
                receipt,
            } => {
                if transaction_hash != candidate.transaction_hash() {
                    return Err(EvmWalletError::InconsistentEvidence);
                }
                if receipt.as_ref().is_some_and(|receipt| {
                    receipt.transaction_hash() != candidate.transaction_hash()
                }) {
                    return Err(EvmWalletError::InconsistentEvidence);
                }
            }
            Self::FinalizedHead { .. } | Self::CanonicalInclusion { .. } => {}
        }
        Ok(())
    }

    /// Returns whether this exact result proves the frozen terminal relation.
    pub fn terminal_outcome(&self) -> Result<Option<EvmTransactionOutcome>, EvmWalletError> {
        self.validate()?;
        let Self::CanonicalInclusion {
            terminal: Some(terminal),
            ..
        } = self
        else {
            return Ok(None);
        };
        terminal.outcome().map(Some)
    }

    /// Returns the complete terminal composite only for the terminal variant.
    pub fn terminal_evidence(&self) -> Result<Option<&EvmWalletTerminalEvidence>, EvmWalletError> {
        self.validate()?;
        let Self::CanonicalInclusion { terminal, .. } = self else {
            return Ok(None);
        };
        Ok(terminal.as_ref())
    }
}

/// Complete domain evidence behind one executor terminal tombstone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-terminal-evidence",
    version = "1",
    schema = "mfm.evm.wallet_terminal_evidence"
)]
pub struct EvmWalletTerminalEvidence {
    request: EvmSubmitTransactionRequest,
    attempt_result_refs: Vec<EvmWalletReference>,
    candidate_lineage: Vec<EvmWalletTransactionCandidate>,
    candidate: EvmWalletTransactionCandidate,
    transaction: EvmWalletObservedTransaction,
    receipt: EvmWalletReceipt,
    finalized_head: EvmBlockAnchor,
    inclusion_block: EvmBlockAnchor,
    executor_generation_ref: EvmWalletReference,
    generation_fence_ref: EvmWalletReference,
    assurance_policy_ref: EvmWalletReference,
}

impl EvmWalletTerminalEvidence {
    /// Constructs terminal evidence only from a sufficient final result.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: EvmSubmitTransactionRequest,
        attempt_result_refs: Vec<EvmWalletReference>,
        candidate_lineage: Vec<EvmWalletTransactionCandidate>,
        candidate: EvmWalletTransactionCandidate,
        transaction: EvmWalletObservedTransaction,
        receipt: EvmWalletReceipt,
        finalized_head: EvmBlockAnchor,
        inclusion_block: EvmBlockAnchor,
        executor_generation_ref: EvmWalletReference,
        generation_fence_ref: EvmWalletReference,
        assurance_policy_ref: EvmWalletReference,
    ) -> Result<Self, EvmWalletError> {
        let evidence = Self {
            request,
            attempt_result_refs,
            candidate_lineage,
            candidate,
            transaction,
            receipt,
            finalized_head,
            inclusion_block,
            executor_generation_ref,
            generation_fence_ref,
            assurance_policy_ref,
        };
        evidence.outcome()?;
        Ok(evidence)
    }

    /// Returns the exact immutable semantic request.
    pub const fn request(&self) -> &EvmSubmitTransactionRequest {
        &self.request
    }

    /// Returns the candidate whose receipt is terminal.
    pub const fn candidate(&self) -> &EvmWalletTransactionCandidate {
        &self.candidate
    }

    /// Returns the canonical inclusion block freshly re-read by number.
    pub const fn inclusion_block(&self) -> &EvmBlockAnchor {
        &self.inclusion_block
    }

    /// Returns the executor durable generation used for every attempt.
    pub const fn executor_generation_ref(&self) -> &EvmWalletReference {
        &self.executor_generation_ref
    }

    /// Returns the external generation-fence attestation.
    pub const fn generation_fence_ref(&self) -> &EvmWalletReference {
        &self.generation_fence_ref
    }

    /// Returns the terminal assurance policy.
    pub const fn assurance_policy_ref(&self) -> &EvmWalletReference {
        &self.assurance_policy_ref
    }

    /// Purely verifies and projects the terminal domain outcome.
    pub fn outcome(&self) -> Result<EvmTransactionOutcome, EvmWalletError> {
        self.executor_generation_ref.to_content_ref()?;
        self.generation_fence_ref.to_content_ref()?;
        self.assurance_policy_ref.to_content_ref()?;
        self.request.template().validate()?;
        self.request.policy().validate()?;
        if self.attempt_result_refs.is_empty()
            || self.attempt_result_refs.len()
                > usize::try_from(self.request.policy().max_attempts()?)
                    .map_err(|_| EvmWalletError::BoundExceeded("attempt_result_refs"))?
            || self
                .attempt_result_refs
                .iter()
                .any(|reference| reference.to_content_ref().is_err())
            || self.candidate.request() != &self.request
            || &self.assurance_policy_ref != self.request.policy().assurance_policy_ref()
        {
            return Err(EvmWalletError::InconsistentEvidence);
        }
        validate_candidate_lineage(&self.candidate_lineage, &self.candidate)?;
        if !self.transaction.matches_candidate(&self.candidate)?
            || !self
                .receipt
                .matches_candidate(&self.candidate, &self.transaction)?
            || self.inclusion_block != *self.receipt.block()
            || parse_quantity(self.finalized_head.number())?
                < parse_quantity(self.receipt.block().number())?
        {
            return Err(EvmWalletError::InconsistentEvidence);
        }
        self.finalized_head
            .validate()
            .map_err(|_| EvmWalletError::Invalid("finalized_head"))?;
        self.inclusion_block
            .validate()
            .map_err(|_| EvmWalletError::Invalid("inclusion_block"))?;
        Ok(EvmTransactionOutcome::from_receipt(&self.receipt))
    }
}

/// Successful semantic result of one finalized EVM transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-outcome",
    version = "1",
    schema = "mfm.evm.transaction_outcome"
)]
pub enum EvmTransactionOutcome {
    /// The finalized top-level execution succeeded.
    Succeeded {
        /// Exact signed transaction hash.
        transaction_hash: String,
        /// Exact canonical inclusion block.
        block: EvmBlockAnchor,
        /// Canonical transaction index.
        transaction_index: String,
        /// Direct-create address, when applicable.
        contract_address: Option<String>,
        /// Canonical gas consumed.
        gas_used: String,
        /// Complete coherent receipt logs.
        logs: Vec<EvmWalletReceiptLog>,
    },
    /// The finalized top-level execution reverted.
    Reverted {
        /// Exact signed transaction hash.
        transaction_hash: String,
        /// Exact canonical inclusion block.
        block: EvmBlockAnchor,
        /// Canonical transaction index.
        transaction_index: String,
        /// Canonical gas consumed.
        gas_used: String,
    },
}

impl EvmTransactionOutcome {
    fn from_receipt(receipt: &EvmWalletReceipt) -> Self {
        match receipt.status {
            EvmWalletReceiptStatus::Success => Self::Succeeded {
                transaction_hash: receipt.transaction_hash.clone(),
                block: receipt.block.clone(),
                transaction_index: receipt.transaction_index.clone(),
                contract_address: receipt.contract_address.clone(),
                gas_used: receipt.gas_used.clone(),
                logs: receipt.logs.clone(),
            },
            EvmWalletReceiptStatus::Reverted => Self::Reverted {
                transaction_hash: receipt.transaction_hash.clone(),
                block: receipt.block.clone(),
                transaction_index: receipt.transaction_index.clone(),
                gas_used: receipt.gas_used.clone(),
            },
        }
    }

    /// Returns the finalized transaction hash.
    pub fn transaction_hash(&self) -> &str {
        match self {
            Self::Succeeded {
                transaction_hash, ..
            }
            | Self::Reverted {
                transaction_hash, ..
            } => transaction_hash,
        }
    }

    /// Returns the exact canonical inclusion block.
    pub const fn block(&self) -> &EvmBlockAnchor {
        match self {
            Self::Succeeded { block, .. } | Self::Reverted { block, .. } => block,
        }
    }
}

/// Value-only public output of the transaction entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PublicOutputs)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.submit_transaction_public_outputs", version = "1")]
pub struct EvmSubmitTransactionPublicOutputs {
    /// Finalized success or revert.
    pub outcome: EvmTransactionOutcome,
}

/// Uninhabited-by-construction semantic failure contract.
///
/// Revert is a successful terminal output. Invalid or insufficient evidence
/// uses the generic evidence verdict and never fabricates a domain failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submit-transaction-failure",
    version = "1",
    schema = "mfm.evm.submit_transaction_failure"
)]
pub struct EvmSubmitTransactionFailure {}

fn validate_request_wire(wire: &EvmSubmitTransactionRequestWire) -> Result<(), EvmWalletError> {
    wire.template.validate()?;
    wire.policy.validate()?;
    if wire.version != "mfm.evm.submit-transaction-request.v1"
        || wire.template_ref != wallet_value_reference(&wire.template)?
        || wire.policy_ref != wallet_value_reference(&wire.policy)?
        || wire.wallet_domain_ref != *wire.policy.wallet_domain_ref()
        || wire.tenant_scope_id != wire.policy.tenant_scope_id()?.as_str()
        || wire.route_generation_ref != *wire.policy.route_generation_ref()
        || wire.chain_id != wire.policy.chain_id()
        || wire.sender != wire.policy.sender()
        || wire.signer_binding_ref != *wire.policy.signer_binding_ref()
    {
        return Err(EvmWalletError::InconsistentEvidence);
    }
    Ok(())
}

fn wallet_value_reference<T>(value: &T) -> Result<EvmWalletReference, EvmWalletError>
where
    T: mfm_values::MfmValue,
{
    let canonical = encode_boundary(value).map_err(|_| EvmWalletError::RequestEncoding)?;
    let schema = T::schema_id().map_err(|_| EvmWalletError::RequestEncoding)?;
    let reference =
        boundary_content_ref(schema, &canonical).map_err(|_| EvmWalletError::RequestEncoding)?;
    Ok(EvmWalletReference::from_content_ref(reference))
}

fn validate_candidate_lineage(
    lineage: &[EvmWalletTransactionCandidate],
    selected: &EvmWalletTransactionCandidate,
) -> Result<(), EvmWalletError> {
    if lineage.is_empty() || lineage.len() > EVM_WALLET_REPLACEMENT_LIMIT {
        return Err(EvmWalletError::InconsistentEvidence);
    }
    for (index, candidate) in lineage.iter().enumerate() {
        candidate.validate()?;
        if usize::from(candidate.fee_ordinal()) != index
            || candidate.request() != selected.request()
        {
            return Err(EvmWalletError::InconsistentEvidence);
        }
    }
    if lineage.last() != Some(selected) {
        return Err(EvmWalletError::InconsistentEvidence);
    }
    let unique_hashes = lineage
        .iter()
        .map(EvmWalletTransactionCandidate::transaction_hash)
        .collect::<BTreeSet<_>>();
    if unique_hashes.len() != lineage.len() {
        return Err(EvmWalletError::InconsistentEvidence);
    }
    Ok(())
}

fn validate_access_list(entries: &[EvmWalletAccessListEntry]) -> Result<(), EvmWalletError> {
    if entries.len() > EVM_WALLET_ACCESS_LIST_MAX_ENTRIES {
        return Err(EvmWalletError::BoundExceeded("access_list_entries"));
    }
    let mut addresses = BTreeSet::new();
    let mut keys = 0_usize;
    for entry in entries {
        entry.validate()?;
        if !addresses.insert(entry.address()) {
            return Err(EvmWalletError::Invalid("duplicate_access_list_address"));
        }
        keys = keys
            .checked_add(entry.storage_keys().len())
            .ok_or(EvmWalletError::BoundExceeded("access_list_storage_keys"))?;
    }
    if keys > EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS {
        return Err(EvmWalletError::BoundExceeded("access_list_storage_keys"));
    }
    Ok(())
}

fn canonical_address(value: Address) -> String {
    format!("{value:#x}")
}

fn canonical_hash(value: B256) -> String {
    format!("{value:#x}")
}

fn canonical_bytes(value: &[u8]) -> String {
    format!("0x{}", hex::encode(value))
}

fn parse_address(value: &str) -> Result<Address, EvmWalletError> {
    let parsed = Address::from_str(value).map_err(|_| EvmWalletError::Invalid("address"))?;
    if canonical_address(parsed) != value {
        return Err(EvmWalletError::Invalid("address"));
    }
    Ok(parsed)
}

fn parse_hash(value: &str) -> Result<B256, EvmWalletError> {
    let parsed = B256::from_str(value).map_err(|_| EvmWalletError::Invalid("hash"))?;
    if canonical_hash(parsed) != value {
        return Err(EvmWalletError::Invalid("hash"));
    }
    Ok(parsed)
}

fn parse_bytes(value: &str, maximum: usize) -> Result<Vec<u8>, EvmWalletError> {
    let encoded = value
        .strip_prefix("0x")
        .ok_or(EvmWalletError::Invalid("bytes"))?;
    if encoded.len() % 2 != 0 || encoded.len() / 2 > maximum {
        return Err(EvmWalletError::BoundExceeded("bytes"));
    }
    if !encoded
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(EvmWalletError::Invalid("bytes"));
    }
    hex::decode(encoded).map_err(|_| EvmWalletError::Invalid("bytes"))
}

fn parse_quantity(value: &str) -> Result<U256, EvmWalletError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvmWalletError::Invalid("quantity"));
    }
    let parsed = U256::from_str(value).map_err(|_| EvmWalletError::Invalid("quantity"))?;
    if parsed.to_string() != value {
        return Err(EvmWalletError::Invalid("quantity"));
    }
    Ok(parsed)
}

#[cfg(test)]
#[path = "wallet_tests.rs"]
mod tests;
