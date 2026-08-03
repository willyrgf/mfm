//! Pure immutable EVM transaction and transport-value contracts.
//!
//! The values in this module contain only reviewed public transaction
//! material. Signatures, signed envelopes, endpoints, credentials, provider
//! bodies, and secret-provider paths have no representation here.

use std::collections::BTreeSet;
use std::str::FromStr;

use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, LocalPublicId, SchemaId};
use mfm_program::boundary_content_ref;
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::EvmBlockAnchor;

/// Published transaction-submission entry point.
pub const EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID: &str = "mfm.evm/submit-transaction@1";
/// Stable structured submission operation identity.
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
/// Maximum UTF-8 bytes in an ordinary caller submission token.
pub const EVM_CALLER_SUBMISSION_TOKEN_MAX_BYTES: usize = 256;
/// Maximum receipt logs retained in terminal evidence.
pub const EVM_WALLET_RECEIPT_LOG_LIMIT: usize = 4_096;
/// Maximum unindexed bytes retained in one receipt log.
pub const EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Exact version of the selected EVM account-sequence policy descriptor.
pub const EVM_WALLET_NONCE_POLICY_VERSION: &str = "mfm.evm.wallet-nonce-policy.v1";
/// Exact version of the finalized-tag finality policy.
pub const EVM_WALLET_FINALITY_POLICY_VERSION: &str = "mfm.evm.wallet-finality-policy.v1";
/// Exact version of the terminal wallet assurance policy.
pub const EVM_WALLET_ASSURANCE_POLICY_VERSION: &str = "mfm.evm.wallet-assurance-policy.v1";
/// Exact version of the deterministic EVM transaction-signing profile.
pub const EVM_DETERMINISTIC_SIGNING_PROFILE_VERSION: &str =
    "mfm.evm.deterministic-signing-profile.v1";
/// Exact version of the bounded EVM submission-expansion policy.
pub const EVM_SUBMISSION_EXPANSION_POLICY_VERSION: &str = "mfm.evm.submission-expansion-policy.v1";

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

/// Returns the canonical EVM account-sequence policy selected by deployment.
///
/// ```
/// let canonical = mfm_evm::evm_wallet_nonce_policy_canonical()?;
/// let reference = mfm_evm::evm_wallet_nonce_policy_ref()?;
///
/// assert!(canonical.as_str().contains("\"fencing\":\"required\""));
/// assert!(reference
///     .to_content_ref()?
///     .schema_id()
///     .as_str()
///     .contains("mfm.evm.wallet-nonce-policy"));
/// # Ok::<(), mfm_evm::EvmWalletError>(())
/// ```
pub fn evm_wallet_nonce_policy_canonical() -> Result<PlainCanonicalJsonBytes, EvmWalletError> {
    wallet_descriptor_canonical(&serde_json::json!({
        "advance_requires_prior_terminal": true,
        "allocation": "monotonic_u64",
        "first_sequence": "configuration.initial_nonce",
        "fencing": "required",
        "reassignment": false,
        "version": EVM_WALLET_NONCE_POLICY_VERSION,
    }))
}

/// Returns the exact EVM account-sequence policy identity.
pub fn evm_wallet_nonce_policy_ref() -> Result<EvmWalletReference, EvmWalletError> {
    wallet_descriptor_ref(
        "mfm.evm.wallet-nonce-policy",
        &evm_wallet_nonce_policy_canonical()?,
    )
}
/// Returns the canonical finalized-tag finality policy.
pub fn evm_wallet_finality_policy_canonical() -> Result<PlainCanonicalJsonBytes, EvmWalletError> {
    wallet_descriptor_canonical(&serde_json::json!({
        "canonical_inclusion_recheck": "fresh_number_lookup",
        "finalized_head": "fresh_finalized_tag",
        "finalized_head_at_or_after_receipt": true,
        "version": EVM_WALLET_FINALITY_POLICY_VERSION,
    }))
}

/// Returns the exact finalized-tag finality-policy identity.
pub fn evm_wallet_finality_policy_ref() -> Result<EvmWalletReference, EvmWalletError> {
    wallet_descriptor_ref(
        "mfm.evm.wallet-finality-policy",
        &evm_wallet_finality_policy_canonical()?,
    )
}

/// Returns the canonical terminal wallet assurance policy.
pub fn evm_wallet_assurance_policy_canonical() -> Result<PlainCanonicalJsonBytes, EvmWalletError> {
    wallet_descriptor_canonical(&serde_json::json!({
        "candidate_lineage": "complete_through_selected_candidate",
        "canonical_inclusion": "receipt_block_matches_fresh_number_lookup",
        "wallet_authority_lineage_and_fence": "exact",
        "finalized_head": "at_or_after_receipt",
        "outcomes": ["reverted", "succeeded"],
        "receipt": "exact_candidate_and_transaction",
        "request": "exact",
        "transaction": "exact_candidate",
        "version": EVM_WALLET_ASSURANCE_POLICY_VERSION,
    }))
}

/// Returns the exact terminal wallet assurance-policy identity.
pub fn evm_wallet_assurance_policy_ref() -> Result<EvmWalletReference, EvmWalletError> {
    wallet_descriptor_ref(
        "mfm.evm.wallet-assurance-policy",
        &evm_wallet_assurance_policy_canonical()?,
    )
}

/// Returns the canonical RFC 6979, recoverable, low-s EVM signing profile.
pub fn evm_deterministic_signing_profile_canonical(
) -> Result<PlainCanonicalJsonBytes, EvmWalletError> {
    wallet_descriptor_canonical(&serde_json::json!({
        "algorithm": "secp256k1.keccak256.recoverable",
        "canonical_low_s": true,
        "deterministic_nonce": "rfc6979",
        "recoverable": true,
        "version": EVM_DETERMINISTIC_SIGNING_PROFILE_VERSION,
    }))
}

/// Returns the exact deterministic EVM signing-profile identity.
pub fn evm_deterministic_signing_profile_ref() -> Result<EvmWalletReference, EvmWalletError> {
    wallet_descriptor_ref(
        "mfm.evm.deterministic-signing-profile",
        &evm_deterministic_signing_profile_canonical()?,
    )
}

/// Returns the canonical bounded submission-expansion policy.
pub fn evm_submission_expansion_policy_canonical() -> Result<PlainCanonicalJsonBytes, EvmWalletError>
{
    wallet_descriptor_canonical(&serde_json::json!({
        "candidate_slots": EVM_WALLET_REPLACEMENT_LIMIT,
        "expansion": "mfm.evm.expansion/submit-transaction",
        "observation_round_limit": crate::EVM_WALLET_OBSERVATION_ROUND_LIMIT,
        "version": EVM_SUBMISSION_EXPANSION_POLICY_VERSION,
    }))
}

/// Returns the exact bounded submission-expansion policy identity.
pub fn evm_submission_expansion_policy_ref() -> Result<EvmWalletReference, EvmWalletError> {
    wallet_descriptor_ref(
        "mfm.evm.submission-expansion-policy",
        &evm_submission_expansion_policy_canonical()?,
    )
}

/// Stable configured-value target selected by the public entry-point input.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-target",
    version = "1",
    schema = "mfm.evm.transaction_target",
    transparent_string
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

/// Bounded ordinary-caller idempotency token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "caller-submission-token",
    version = "1",
    schema = "mfm.evm.caller_submission_token",
    transparent_string
)]
pub struct EvmCallerSubmissionToken {
    value: String,
}

impl EvmCallerSubmissionToken {
    /// Creates one non-empty, whitespace-free, bounded token.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmWalletError> {
        let value = value.into();
        validate_caller_submission_token(&value)?;
        Ok(Self { value })
    }

    /// Returns the caller token exactly as admitted.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn into_string(self) -> String {
        self.value
    }
}

impl<'de> Deserialize<'de> for EvmCallerSubmissionToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

pub(crate) fn validate_caller_submission_token(value: &str) -> Result<(), EvmWalletError> {
    if value.is_empty()
        || value.len() > EVM_CALLER_SUBMISSION_TOKEN_MAX_BYTES
        || value.chars().any(char::is_whitespace)
    {
        return Err(EvmWalletError::Invalid("caller_submission_token"));
    }
    Ok(())
}

/// Public value-only selector for one configured immutable transaction and caller intent.
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
    caller_submission_token: EvmCallerSubmissionToken,
}

impl EvmSubmitTransactionSelector {
    /// Selects one exact configured transaction target and caller intent.
    pub const fn new(
        target: EvmTransactionTarget,
        caller_submission_token: EvmCallerSubmissionToken,
    ) -> Self {
        Self {
            target,
            caller_submission_token,
        }
    }

    /// Returns the configured-value target.
    pub const fn target(&self) -> &EvmTransactionTarget {
        &self.target
    }

    /// Returns the bounded caller idempotency token.
    pub const fn caller_submission_token(&self) -> &EvmCallerSubmissionToken {
        &self.caller_submission_token
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
/// Closed, payload-free submission failure contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-failure",
    version = "1",
    schema = "mfm.evm.submission_failure"
)]
pub enum EvmSubmissionFailure {
    /// The bounded transport was unavailable.
    TransportUnavailable,
    /// The selected provider was unavailable.
    ProviderUnavailable,
    /// The qualified signer was unavailable.
    SignerUnavailable,
    /// The wallet nonce authority was unavailable.
    NonceAuthorityUnavailable,
    /// The destination definitely rejected the candidate.
    DestinationRejected,
    /// The bounded observation policy was exhausted.
    ObservationPolicyExhausted,
    /// The bounded replacement policy was exhausted.
    ReplacementPolicyExhausted,
    /// Another submission intent currently owns the nonce domain.
    NonceDomainBusy,
    /// Provider pending state diverged from the exclusive local lineage.
    NonceLineageDiverged,
    /// The nonce space cannot advance without overflow.
    NonceCapacityExhausted,
    /// The canonically included transaction reverted.
    ExecutionReverted,
}

impl<'de> Deserialize<'de> for EvmSubmissionFailure {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: String,
        }

        match Wire::deserialize(deserializer)?.kind.as_str() {
            "transport_unavailable" => Ok(Self::TransportUnavailable),
            "provider_unavailable" => Ok(Self::ProviderUnavailable),
            "signer_unavailable" => Ok(Self::SignerUnavailable),
            "nonce_authority_unavailable" => Ok(Self::NonceAuthorityUnavailable),
            "destination_rejected" => Ok(Self::DestinationRejected),
            "observation_policy_exhausted" => Ok(Self::ObservationPolicyExhausted),
            "replacement_policy_exhausted" => Ok(Self::ReplacementPolicyExhausted),
            "nonce_domain_busy" => Ok(Self::NonceDomainBusy),
            "nonce_lineage_diverged" => Ok(Self::NonceLineageDiverged),
            "nonce_capacity_exhausted" => Ok(Self::NonceCapacityExhausted),
            "execution_reverted" => Ok(Self::ExecutionReverted),
            _ => Err(de::Error::custom("unknown EVM submission failure")),
        }
    }
}
fn wallet_descriptor_canonical(
    value: &impl Serialize,
) -> Result<PlainCanonicalJsonBytes, EvmWalletError> {
    let json = serde_json::to_string(value).map_err(|_| EvmWalletError::RequestEncoding)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| EvmWalletError::RequestEncoding)
}

fn wallet_descriptor_ref(
    schema_name: &'static str,
    canonical: &PlainCanonicalJsonBytes,
) -> Result<EvmWalletReference, EvmWalletError> {
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{schema_name}:1").as_bytes()),
    )
    .map_err(|_| EvmWalletError::RequestEncoding)?;
    let reference =
        boundary_content_ref(schema, canonical).map_err(|_| EvmWalletError::RequestEncoding)?;
    Ok(EvmWalletReference::from_content_ref(reference))
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
