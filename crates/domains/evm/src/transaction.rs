use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

use mfm_canonical::CanonicalBytes;
use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program::{
    CapabilityInjection, EffectState, PreparationError, ProgramError, ProposedStateOutcome, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::de;
use serde::{Deserialize, Serialize};

use crate::EvmDomainError;

/// Exact Effect capability identity for EIP-1559 transaction execution.
pub const EVM_TRANSACTION_EFFECT_CAPABILITY_ID: &str = "mfm.evm.capability.execute-transaction@1";
/// Exact State identity for EIP-1559 transaction execution.
pub const EXECUTE_EVM_TRANSACTION_STATE_ID: &str = "mfm.evm.state.execute-transaction@1";

const MAX_U256_DECIMAL: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";
/// Maximum EVM contract-creation initcode bytes admitted by one command.
pub const MAX_EVM_INITCODE_BYTES: usize = 49_152;
/// Maximum EVM call calldata bytes admitted by one command or anchored Read.
pub const MAX_EVM_CALLDATA_BYTES: usize = 131_072;

macro_rules! checked_deserialize {
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

/// Exact lowercase 20-byte EVM address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "address",
    version = "1",
    schema = "mfm.evm-address",
    transparent_string
)]
pub struct EvmAddress {
    #[mfm(minimum_bytes = 42, maximum_bytes = 42)]
    value: String,
}

impl EvmAddress {
    /// Parses exact lowercase `0x` plus 40 lowercase hexadecimal digits.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmDomainError> {
        let value = value.into();
        if value.len() != 42
            || !value.starts_with("0x")
            || !value[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { value })
    }

    /// Returns the exact checked address text.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for EvmAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EvmAddress {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Exact lowercase 32-byte EVM hash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "hash",
    version = "1",
    schema = "mfm.evm-hash",
    transparent_string
)]
pub struct EvmHash {
    #[mfm(minimum_bytes = 66, maximum_bytes = 66)]
    value: String,
}

impl EvmHash {
    /// Parses exact lowercase `0x` plus 64 lowercase hexadecimal digits.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmDomainError> {
        let value = value.into();
        if value.len() != 66
            || !value.starts_with("0x")
            || !value[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { value })
    }

    /// Returns the exact checked hash text.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for EvmHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EvmHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Canonical decimal unsigned EVM word in the inclusive range `0..=2^256-1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "uint256",
    version = "1",
    schema = "mfm.evm-uint256",
    transparent_string
)]
pub struct EvmU256 {
    #[mfm(minimum_bytes = 1, maximum_bytes = 78)]
    value: String,
}

impl EvmU256 {
    /// Parses a canonical decimal integer no greater than `2^256-1`.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmDomainError> {
        let value = value.into();
        let canonical = value == "0"
            || (!value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit()));
        if value.is_empty()
            || !canonical
            || value.len() > MAX_U256_DECIMAL.len()
            || (value.len() == MAX_U256_DECIMAL.len() && value.as_str() > MAX_U256_DECIMAL)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { value })
    }

    /// Constructs one quantity from a `u64`.
    pub fn from_u64(value: u64) -> Self {
        Self {
            value: value.to_string(),
        }
    }

    /// Returns the canonical decimal spelling.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn numeric_cmp(&self, other: &Self) -> Ordering {
        self.value
            .len()
            .cmp(&other.value.len())
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl fmt::Display for EvmU256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EvmU256 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Exact public 32-byte authority epoch distinguishing one nonce history.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-authority-epoch",
    version = "1",
    schema = "mfm.evm-transaction-authority-epoch",
    transparent_bytes
)]
pub struct EvmAuthorityEpoch {
    #[mfm(minimum_bytes = 32, maximum_bytes = 32)]
    value: String,
}

impl EvmAuthorityEpoch {
    /// Constructs an epoch from exactly 32 public bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self {
            value: CanonicalBytes::new(bytes.to_vec()).encoded().to_owned(),
        }
    }

    /// Decodes the exact public epoch bytes.
    pub fn as_bytes(&self) -> Result<[u8; 32], EvmDomainError> {
        decode_exact_bytes::<32>(&self.value)
    }
}

impl<'de> Deserialize<'de> for EvmAuthorityEpoch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        decode_exact_bytes::<32>(&value).map_err(de::Error::custom)?;
        Ok(Self { value })
    }
}

/// Chain identity pinned by chain ID and expected genesis block hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-instance",
    version = "1",
    schema = "mfm.evm-chain-instance"
)]
pub struct EvmChainInstance {
    #[mfm(unsigned_minimum = 1, unsigned_maximum = 18446744073709551615)]
    chain_id: u64,
    expected_genesis_hash: EvmHash,
}

impl EvmChainInstance {
    /// Constructs one exact nonzero chain instance.
    pub fn new(chain_id: u64, expected_genesis_hash: EvmHash) -> Result<Self, EvmDomainError> {
        let value = Self {
            chain_id,
            expected_genesis_hash,
        };
        value.validate()?;
        Ok(value)
    }

    /// Returns the nonzero chain ID.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the expected genesis block hash.
    pub const fn expected_genesis_hash(&self) -> &EvmHash {
        &self.expected_genesis_hash
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        (self.chain_id != 0)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

checked_deserialize!(EvmChainInstance {
    chain_id: u64,
    expected_genesis_hash: EvmHash,
});

/// Public route used to bind transaction execution and anchored observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-route",
    version = "1",
    schema = "mfm.evm-transaction-route"
)]
pub struct EvmTransactionRoute {
    chain_instance: EvmChainInstance,
    endpoint_ref: ContentRef,
}

impl EvmTransactionRoute {
    /// Constructs one public transaction route.
    pub fn new(chain_instance: EvmChainInstance, endpoint_ref: ContentRef) -> Self {
        Self {
            chain_instance,
            endpoint_ref,
        }
    }

    /// Returns the exact chain instance.
    pub const fn chain_instance(&self) -> &EvmChainInstance {
        &self.chain_instance
    }

    /// Returns the selected public endpoint reference.
    pub const fn endpoint_ref(&self) -> &ContentRef {
        &self.endpoint_ref
    }

    /// Derives the exact adapter binding reference.
    pub fn binding_ref(&self) -> Result<ContentRef, EvmDomainError> {
        canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }
}

/// Complete public execution binding for one account on one transaction route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-binding",
    version = "1",
    schema = "mfm.evm-transaction-binding"
)]
pub struct EvmTransactionBinding {
    authority_epoch: EvmAuthorityEpoch,
    route: EvmTransactionRoute,
    sender: EvmAddress,
}

impl EvmTransactionBinding {
    /// Constructs one complete public transaction binding.
    pub fn new(
        route: EvmTransactionRoute,
        authority_epoch: EvmAuthorityEpoch,
        sender: EvmAddress,
    ) -> Self {
        Self {
            authority_epoch,
            route,
            sender,
        }
    }

    /// Returns the authority epoch.
    pub const fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    /// Returns the transaction route.
    pub const fn route(&self) -> &EvmTransactionRoute {
        &self.route
    }

    /// Returns the bound sender account.
    pub const fn sender(&self) -> &EvmAddress {
        &self.sender
    }

    /// Derives the exact Effect adapter binding reference.
    pub fn binding_ref(&self) -> Result<ContentRef, EvmDomainError> {
        canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }
}

/// Fixed EIP-1559 transaction action with no optional target ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "eip1559-transaction-action",
    version = "1",
    schema = "mfm.evm-eip1559-transaction-action"
)]
pub enum EvmTransactionAction {
    /// Creates a contract from bounded initcode.
    Create {
        /// Canonical base64url-no-pad initcode bytes.
        #[mfm(minimum_bytes = 0, maximum_bytes = 49152)]
        initcode: String,
    },
    /// Calls one address with bounded calldata.
    Call {
        /// Exact call target.
        to: EvmAddress,
        /// Canonical base64url-no-pad calldata bytes.
        #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
        calldata: String,
    },
}

impl EvmTransactionAction {
    /// Constructs a contract-creation action.
    pub fn create(initcode: Vec<u8>) -> Result<Self, EvmDomainError> {
        if initcode.len() > MAX_EVM_INITCODE_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self::Create {
            initcode: encode_bytes(initcode),
        })
    }

    /// Constructs a contract-call action.
    pub fn call(to: EvmAddress, calldata: Vec<u8>) -> Result<Self, EvmDomainError> {
        if calldata.len() > MAX_EVM_CALLDATA_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self::Call {
            to,
            calldata: encode_bytes(calldata),
        })
    }

    /// Returns creation initcode or call calldata bytes.
    pub fn input_bytes(&self) -> Result<Vec<u8>, EvmDomainError> {
        match self {
            Self::Create { initcode } => decode_bounded_bytes(initcode, MAX_EVM_INITCODE_BYTES),
            Self::Call { calldata, .. } => decode_bounded_bytes(calldata, MAX_EVM_CALLDATA_BYTES),
        }
    }

    /// Returns the call target, or `None` for creation.
    pub const fn call_target(&self) -> Option<&EvmAddress> {
        match self {
            Self::Create { .. } => None,
            Self::Call { to, .. } => Some(to),
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        match self {
            Self::Create { initcode } => {
                decode_bounded_bytes(initcode, MAX_EVM_INITCODE_BYTES).map(|_| ())
            }
            Self::Call { calldata, .. } => {
                decode_bounded_bytes(calldata, MAX_EVM_CALLDATA_BYTES).map(|_| ())
            }
        }
    }
}

impl<'de> Deserialize<'de> for EvmTransactionAction {
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
            Create { initcode: String },
            Call { to: EvmAddress, calldata: String },
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::Create { initcode } => Self::Create { initcode },
            Wire::Call { to, calldata } => Self::Call { to, calldata },
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

/// Complete nonce-free fixed-form EIP-1559 transaction command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "eip1559-transaction-command",
    version = "1",
    schema = "mfm.evm-eip1559-transaction-command"
)]
pub struct Eip1559TransactionCommand {
    action: EvmTransactionAction,
    binding: EvmTransactionBinding,
    #[mfm(unsigned_minimum = 1, unsigned_maximum = 18446744073709551615)]
    gas_limit: u64,
    max_fee_per_gas: EvmU256,
    max_priority_fee_per_gas: EvmU256,
    value: EvmU256,
}

impl Eip1559TransactionCommand {
    /// Constructs one complete EIP-1559 command with an intrinsically empty access list.
    pub fn new(
        binding: EvmTransactionBinding,
        action: EvmTransactionAction,
        value: EvmU256,
        gas_limit: u64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        let command = Self {
            action,
            binding,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            value,
        };
        command.validate()?;
        Ok(command)
    }

    /// Returns the fixed create-or-call action.
    pub const fn action(&self) -> &EvmTransactionAction {
        &self.action
    }

    /// Returns the complete public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }

    /// Returns the exact nonzero gas limit.
    pub const fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    /// Returns the maximum fee per gas.
    pub const fn max_fee_per_gas(&self) -> &EvmU256 {
        &self.max_fee_per_gas
    }

    /// Returns the maximum priority fee per gas.
    pub const fn max_priority_fee_per_gas(&self) -> &EvmU256 {
        &self.max_priority_fee_per_gas
    }

    /// Returns the transferred value.
    pub const fn value(&self) -> &EvmU256 {
        &self.value
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.gas_limit == 0
            || self.action.validate().is_err()
            || self
                .max_fee_per_gas
                .numeric_cmp(&self.max_priority_fee_per_gas)
                == Ordering::Less
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

checked_deserialize!(Eip1559TransactionCommand {
    action: EvmTransactionAction,
    binding: EvmTransactionBinding,
    gas_limit: u64,
    max_fee_per_gas: EvmU256,
    max_priority_fee_per_gas: EvmU256,
    value: EvmU256,
});

/// Caller-owned context paired with one complete transaction command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-context",
    version = "1",
    schema = "mfm.evm-transaction-context"
)]
pub struct EvmTransactionContext<K: MfmValueTrait> {
    caller_context: K,
    command: Eip1559TransactionCommand,
}

impl<K: MfmValueTrait> EvmTransactionContext<K> {
    /// Constructs a context-preserving transaction input.
    pub fn new(caller_context: K, command: Eip1559TransactionCommand) -> Self {
        Self {
            caller_context,
            command,
        }
    }

    /// Returns the caller-owned context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the complete command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        &self.command
    }
}

/// Confirmed transaction evidence exposed directly to workflows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-confirmation",
    version = "1",
    schema = "mfm.evm-transaction-confirmation"
)]
pub enum EvmTransactionConfirmation {
    /// A contract creation succeeded.
    Created {
        /// Canonical receipt block anchor.
        block_anchor: crate::EvmBlockAnchor,
        /// Receipt-derived created address.
        created_address: EvmAddress,
        /// Exact signed transaction hash.
        transaction_hash: EvmHash,
    },
    /// An ordinary call succeeded.
    Called {
        /// Canonical receipt block anchor.
        block_anchor: crate::EvmBlockAnchor,
        /// Exact signed transaction hash.
        transaction_hash: EvmHash,
    },
}

impl EvmTransactionConfirmation {
    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        match self {
            Self::Created { block_anchor, .. } | Self::Called { block_anchor, .. } => block_anchor,
        }
    }

    /// Returns the exact signed transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        match self {
            Self::Created {
                transaction_hash, ..
            }
            | Self::Called {
                transaction_hash, ..
            } => transaction_hash,
        }
    }
}

/// Minimal reverted transaction projection exposed to workflows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-revert",
    version = "1",
    schema = "mfm.evm-transaction-revert"
)]
pub struct EvmTransactionRevert {
    block_anchor: crate::EvmBlockAnchor,
    transaction_hash: EvmHash,
}

impl EvmTransactionRevert {
    /// Constructs one reverted receipt projection.
    pub fn new(block_anchor: crate::EvmBlockAnchor, transaction_hash: EvmHash) -> Self {
        Self {
            block_anchor,
            transaction_hash,
        }
    }

    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns the transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
}

/// Complete durable settlement evidence for one Effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-settlement",
    version = "1",
    schema = "mfm.evm-transaction-settlement"
)]
pub enum EvmTransactionSettlement {
    /// A transaction was confirmed successfully.
    Confirmed {
        /// Settled Effect identity.
        effect_id: EffectId,
        /// Reserved account nonce.
        nonce: u64,
        /// Complete confirmation evidence.
        confirmation: EvmTransactionConfirmation,
    },
    /// A transaction was confirmed reverted.
    Reverted {
        /// Settled Effect identity.
        effect_id: EffectId,
        /// Reserved account nonce.
        nonce: u64,
        /// Complete revert evidence.
        revert: EvmTransactionRevert,
    },
}

impl EvmTransactionSettlement {
    /// Constructs a confirmed settlement.
    pub fn confirmed(
        effect_id: EffectId,
        nonce: u64,
        confirmation: EvmTransactionConfirmation,
    ) -> Self {
        Self::Confirmed {
            effect_id,
            nonce,
            confirmation,
        }
    }

    /// Constructs a reverted settlement.
    pub fn reverted(effect_id: EffectId, nonce: u64, revert: EvmTransactionRevert) -> Self {
        Self::Reverted {
            effect_id,
            nonce,
            revert,
        }
    }

    /// Returns the settled Effect identity.
    pub const fn effect_id(&self) -> &EffectId {
        match self {
            Self::Confirmed { effect_id, .. } | Self::Reverted { effect_id, .. } => effect_id,
        }
    }

    /// Returns the reserved transaction nonce.
    pub const fn nonce(&self) -> u64 {
        match self {
            Self::Confirmed { nonce, .. } | Self::Reverted { nonce, .. } => *nonce,
        }
    }

    /// Returns the transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        match self {
            Self::Confirmed { confirmation, .. } => confirmation.transaction_hash(),
            Self::Reverted { revert, .. } => revert.transaction_hash(),
        }
    }

    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        match self {
            Self::Confirmed { confirmation, .. } => confirmation.block_anchor(),
            Self::Reverted { revert, .. } => revert.block_anchor(),
        }
    }
}

/// Caller context preserved through successful transaction execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-completion",
    version = "1",
    schema = "mfm.evm-transaction-completion"
)]
pub struct EvmTransactionCompletion<K: MfmValueTrait> {
    caller_context: K,
    confirmed: EvmTransactionConfirmation,
}

impl<K: MfmValueTrait> EvmTransactionCompletion<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the minimal confirmation.
    pub const fn confirmed(&self) -> &EvmTransactionConfirmation {
        &self.confirmed
    }
}

/// Caller context preserved through reverted transaction execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-reversion",
    version = "1",
    schema = "mfm.evm-transaction-reversion"
)]
pub struct EvmTransactionReversion<K: MfmValueTrait> {
    caller_context: K,
    reverted: EvmTransactionRevert,
}

impl<K: MfmValueTrait> EvmTransactionReversion<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the minimal revert projection.
    pub const fn reverted(&self) -> &EvmTransactionRevert {
        &self.reverted
    }
}

/// Mutating EVM transaction Effect capability.
pub struct EvmTransactionEffect;

impl EffectCapabilityContract for EvmTransactionEffect {
    type Command = Eip1559TransactionCommand;
    type Evidence = EvmTransactionSettlement;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new(EVM_TRANSACTION_EFFECT_CAPABILITY_ID)
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        let action_matches = matches!(
            (command.action(), evidence),
            (
                EvmTransactionAction::Create { .. },
                EvmTransactionSettlement::Confirmed {
                    confirmation: EvmTransactionConfirmation::Created { .. },
                    ..
                } | EvmTransactionSettlement::Reverted { .. }
            ) | (
                EvmTransactionAction::Call { .. },
                EvmTransactionSettlement::Confirmed {
                    confirmation: EvmTransactionConfirmation::Called { .. },
                    ..
                } | EvmTransactionSettlement::Reverted { .. }
            )
        );
        (evidence.effect_id() == effect_id && action_matches)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

/// Context-preserving transaction execution State.
pub struct ExecuteEvmTransaction<K: MfmValueTrait>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for ExecuteEvmTransaction<K> {
    type Input = EvmTransactionContext<K>;
    type Output = EvmTransactionCompletion<K>;
    type Failure = EvmTransactionReversion<K>;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new(EXECUTE_EVM_TRANSACTION_STATE_ID).map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> EffectState<EvmTransactionEffect> for ExecuteEvmTransaction<K> {
    fn prepare(input: &Self::Input) -> Result<Eip1559TransactionCommand, PreparationError> {
        input
            .command
            .validate()
            .map(|_| input.command.clone())
            .map_err(|_| PreparationError)
    }

    fn interpret(
        input: Self::Input,
        evidence: &EvmTransactionSettlement,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        match evidence {
            EvmTransactionSettlement::Confirmed { confirmation, .. } => {
                ProposedStateOutcome::Success {
                    output: EvmTransactionCompletion {
                        caller_context: input.caller_context,
                        confirmed: confirmation.clone(),
                    },
                }
            }
            EvmTransactionSettlement::Reverted { revert, .. } => ProposedStateOutcome::Failure {
                failure: EvmTransactionReversion {
                    caller_context: input.caller_context,
                    reverted: revert.clone(),
                },
            },
        }
    }
}

impl<K: MfmValueTrait> CapabilityInjection<ExecuteEvmTransaction<K>> for EvmTransactionEffect {
    type Setup = EvmTransactionBinding;
    type ExpandedInput = EvmTransactionContext<K>;
    type ExpandedOutput = EvmTransactionCompletion<K>;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

fn encode_bytes(bytes: Vec<u8>) -> String {
    CanonicalBytes::new(bytes).encoded().to_owned()
}

pub(crate) fn decode_bounded_bytes(value: &str, maximum: usize) -> Result<Vec<u8>, EvmDomainError> {
    let bytes = CanonicalBytes::from_base64url_no_pad(value.to_owned())
        .map_err(|_| EvmDomainError::InvalidValue)?
        .into_bytes();
    (bytes.len() <= maximum)
        .then_some(bytes)
        .ok_or(EvmDomainError::InvalidValue)
}

fn decode_exact_bytes<const N: usize>(value: &str) -> Result<[u8; N], EvmDomainError> {
    decode_bounded_bytes(value, N)?
        .try_into()
        .map_err(|_| EvmDomainError::InvalidValue)
}
