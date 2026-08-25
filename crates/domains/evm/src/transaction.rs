use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;

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
/// Exact State identity for EVM contract creation.
pub const CREATE_EVM_CONTRACT_STATE_ID: &str = "mfm.evm.state.create-contract@1";
/// Exact State identity for EVM contract calls.
pub const CALL_EVM_CONTRACT_STATE_ID: &str = "mfm.evm.state.call-contract@1";

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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm",
    name = "address",
    version = "1",
    schema = "mfm.evm-address"
)]
pub struct EvmAddress(#[mfm(minimum_bytes = 42, maximum_bytes = 42)] [u8; 20]);

impl EvmAddress {
    /// Parses exact lowercase `0x` plus 40 lowercase hexadecimal digits.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EvmDomainError> {
        decode_fixed_hex(value.as_ref()).map(Self)
    }

    /// Constructs one address from exactly 20 bytes.
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Returns the exact address bytes.
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl fmt::Display for EvmAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_fixed_hex(&self.0, formatter)
    }
}

impl TryFrom<String> for EvmAddress {
    type Error = EvmDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<EvmAddress> for String {
    fn from(value: EvmAddress) -> Self {
        encode_fixed_hex(&value.0)
    }
}

/// Exact lowercase 32-byte EVM hash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm",
    name = "hash",
    version = "1",
    schema = "mfm.evm-hash"
)]
pub struct EvmHash(#[mfm(minimum_bytes = 66, maximum_bytes = 66)] [u8; 32]);

impl EvmHash {
    /// Parses exact lowercase `0x` plus 64 lowercase hexadecimal digits.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EvmDomainError> {
        decode_fixed_hex(value.as_ref()).map(Self)
    }

    /// Constructs one hash from exactly 32 bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the exact hash bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for EvmHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_fixed_hex(&self.0, formatter)
    }
}

impl TryFrom<String> for EvmHash {
    type Error = EvmDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<EvmHash> for String {
    fn from(value: EvmHash) -> Self {
        encode_fixed_hex(&value.0)
    }
}

/// Canonical decimal unsigned EVM word in the inclusive range `0..=2^256-1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.evm",
    name = "uint256",
    version = "1",
    schema = "mfm.evm-uint256"
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

    /// Returns this value as `u128` when it fits that checked range.
    pub fn to_u128(&self) -> Option<u128> {
        self.value.parse().ok()
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
    schema = "mfm.evm-transaction-authority-epoch"
)]
pub struct EvmAuthorityEpoch {
    #[mfm(minimum_bytes = 32, maximum_bytes = 32)]
    value: CanonicalBytes,
}

impl EvmAuthorityEpoch {
    /// Constructs an epoch from exactly 32 public bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self {
            value: CanonicalBytes::new(bytes),
        }
    }

    /// Returns the exact 32 public epoch bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.value.as_bytes()
    }
}

impl<'de> Deserialize<'de> for EvmAuthorityEpoch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = CanonicalBytes::deserialize(deserializer)?;
        if value.as_bytes().len() != 32 {
            return Err(de::Error::custom(EvmDomainError::InvalidValue));
        }
        Ok(Self { value })
    }
}

/// Chain identity pinned by chain ID and expected genesis block hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-instance",
    version = "1",
    schema = "mfm.evm-chain-instance"
)]
pub struct EvmChainInstance {
    chain_id: NonZeroU64,
    expected_genesis_hash: EvmHash,
}

impl EvmChainInstance {
    /// Constructs one exact nonzero chain instance.
    pub fn new(chain_id: NonZeroU64, expected_genesis_hash: EvmHash) -> Self {
        Self {
            chain_id,
            expected_genesis_hash,
        }
    }

    /// Returns the nonzero chain ID.
    pub const fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }

    /// Returns the expected genesis block hash.
    pub const fn expected_genesis_hash(&self) -> &EvmHash {
        &self.expected_genesis_hash
    }
}

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

/// Private fixed EIP-1559 action retained inside a complete command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
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
enum TransactionAction {
    Create {
        #[mfm(minimum_bytes = 0, maximum_bytes = 49152)]
        initcode: CanonicalBytes,
    },
    Call {
        to: EvmAddress,
        #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
        calldata: CanonicalBytes,
    },
}

impl TransactionAction {
    fn input(&self) -> &[u8] {
        match self {
            Self::Create { initcode } => initcode.as_bytes(),
            Self::Call { calldata, .. } => calldata.as_bytes(),
        }
    }

    const fn to(&self) -> Option<&EvmAddress> {
        match self {
            Self::Create { .. } => None,
            Self::Call { to, .. } => Some(to),
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        match self {
            Self::Create { initcode } if initcode.as_bytes().len() <= MAX_EVM_INITCODE_BYTES => {
                Ok(())
            }
            Self::Call { calldata, .. } if calldata.as_bytes().len() <= MAX_EVM_CALLDATA_BYTES => {
                Ok(())
            }
            _ => Err(EvmDomainError::InvalidValue),
        }
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
    action: TransactionAction,
    binding: EvmTransactionBinding,
    gas_limit: NonZeroU64,
    max_fee_per_gas: EvmU256,
    max_priority_fee_per_gas: EvmU256,
    value: EvmU256,
}

impl Eip1559TransactionCommand {
    /// Constructs one complete contract-creation command.
    pub fn create(
        binding: EvmTransactionBinding,
        initcode: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        if initcode.len() > MAX_EVM_INITCODE_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Self::new(
            binding,
            TransactionAction::Create {
                initcode: CanonicalBytes::new(initcode),
            },
            value,
            gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        )
    }

    /// Constructs one complete contract-call command.
    pub fn call(
        binding: EvmTransactionBinding,
        to: EvmAddress,
        calldata: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        if calldata.len() > MAX_EVM_CALLDATA_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Self::new(
            binding,
            TransactionAction::Call {
                to,
                calldata: CanonicalBytes::new(calldata),
            },
            value,
            gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        )
    }

    fn new(
        binding: EvmTransactionBinding,
        action: TransactionAction,
        value: EvmU256,
        gas_limit: NonZeroU64,
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

    /// Returns the call target, or `None` for contract creation.
    pub const fn to(&self) -> Option<&EvmAddress> {
        self.action.to()
    }

    /// Returns the checked creation initcode or call calldata.
    pub fn input(&self) -> &[u8] {
        self.action.input()
    }

    /// Returns the complete public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }

    /// Returns the exact nonzero gas limit.
    pub const fn gas_limit(&self) -> NonZeroU64 {
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
        if self.action.validate().is_err()
            || self.max_fee_per_gas.to_u128().is_none()
            || self.max_priority_fee_per_gas.to_u128().is_none()
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
    action: TransactionAction,
    binding: EvmTransactionBinding,
    gas_limit: NonZeroU64,
    max_fee_per_gas: EvmU256,
    max_priority_fee_per_gas: EvmU256,
    value: EvmU256,
});

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
struct TransactionContextWire<K> {
    caller_context: K,
    command: Eip1559TransactionCommand,
}

/// Caller-owned context paired with one checked contract-creation command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-creation-context",
    version = "1",
    schema = "mfm.evm-contract-creation-context"
)]
pub struct EvmContractCreationContext<K: MfmValueTrait> {
    caller_context: K,
    command: Eip1559TransactionCommand,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmContractCreationContext<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TransactionContextWire::<K>::deserialize(deserializer)?;
        Self::new(wire.caller_context, wire.command).map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmContractCreationContext<K> {
    /// Constructs a context-preserving contract-creation input.
    pub fn new(
        caller_context: K,
        command: Eip1559TransactionCommand,
    ) -> Result<Self, EvmDomainError> {
        let context = Self {
            caller_context,
            command,
        };
        context.validate().map(|_| context)
    }

    /// Returns the caller-owned context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the complete command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        &self.command
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.command.validate()?;
        self.command
            .to()
            .is_none()
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

/// Caller-owned context paired with one checked contract-call command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-call-context",
    version = "1",
    schema = "mfm.evm-contract-call-context"
)]
pub struct EvmContractCallContext<K: MfmValueTrait> {
    caller_context: K,
    command: Eip1559TransactionCommand,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmContractCallContext<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TransactionContextWire::<K>::deserialize(deserializer)?;
        Self::new(wire.caller_context, wire.command).map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmContractCallContext<K> {
    /// Constructs a context-preserving contract-call input.
    pub fn new(
        caller_context: K,
        command: Eip1559TransactionCommand,
    ) -> Result<Self, EvmDomainError> {
        let context = Self {
            caller_context,
            command,
        };
        context.validate().map(|_| context)
    }

    /// Returns the caller-owned context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the complete command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        &self.command
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.command.validate()?;
        self.command
            .to()
            .is_some()
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

impl<K: MfmValueTrait> EvmContractCallContext<EvmContractCreationCompletion<K>> {
    /// Constructs a call to the contract created by the preceding completion.
    pub fn for_created_contract(
        deployment: EvmContractCreationCompletion<K>,
        calldata: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        let binding = deployment.binding().clone();
        let target = deployment.created_address().clone();
        let command = Eip1559TransactionCommand::call(
            binding,
            target,
            calldata,
            value,
            gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        )?;
        Self::new(deployment, command)
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

/// Caller context and action-specific facts from successful contract creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-creation-completion",
    version = "1",
    schema = "mfm.evm-contract-creation-completion"
)]
pub struct EvmContractCreationCompletion<K: MfmValueTrait> {
    block_anchor: crate::EvmBlockAnchor,
    binding: EvmTransactionBinding,
    caller_context: K,
    created_address: EvmAddress,
    transaction_hash: EvmHash,
}

impl<K: MfmValueTrait> EvmContractCreationCompletion<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the execution binding selected by the creation command.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }

    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns the receipt-derived contract address.
    pub const fn created_address(&self) -> &EvmAddress {
        &self.created_address
    }

    /// Returns the exact signed transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
}

/// Caller context and action-specific facts from a successful contract call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-call-completion",
    version = "1",
    schema = "mfm.evm-contract-call-completion"
)]
pub struct EvmContractCallCompletion<K: MfmValueTrait> {
    block_anchor: crate::EvmBlockAnchor,
    binding: EvmTransactionBinding,
    caller_context: K,
    target: EvmAddress,
    transaction_hash: EvmHash,
}

impl<K: MfmValueTrait> EvmContractCallCompletion<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the execution binding selected by the call command.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }

    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns the target selected by the checked call command.
    pub const fn target(&self) -> &EvmAddress {
        &self.target
    }

    /// Returns the exact signed transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
}

/// Context-preserving failure from contract creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-creation-failure",
    version = "1",
    schema = "mfm.evm-contract-creation-failure"
)]
pub enum EvmContractCreationFailure<K: MfmValueTrait> {
    /// The creation transaction was confirmed reverted.
    Reverted {
        /// Unchanged caller-owned context.
        caller_context: K,
        /// Minimal receipt-derived revert projection.
        revert: EvmTransactionRevert,
    },
    /// Direct interpretation received a successful settlement for the opposite action.
    InconsistentSettlement {
        /// Unchanged caller-owned context.
        caller_context: K,
    },
}

/// Context-preserving failure from a contract call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-call-failure",
    version = "1",
    schema = "mfm.evm-contract-call-failure"
)]
pub enum EvmContractCallFailure<K: MfmValueTrait> {
    /// The call transaction was confirmed reverted.
    Reverted {
        /// Unchanged caller-owned context.
        caller_context: K,
        /// Minimal receipt-derived revert projection.
        revert: EvmTransactionRevert,
    },
    /// Direct interpretation received a successful settlement for the opposite action.
    InconsistentSettlement {
        /// Unchanged caller-owned context.
        caller_context: K,
    },
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
            (command.to(), evidence),
            (
                None,
                EvmTransactionSettlement::Confirmed {
                    confirmation: EvmTransactionConfirmation::Created { .. },
                    ..
                } | EvmTransactionSettlement::Reverted { .. }
            ) | (
                Some(_),
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

/// Context-preserving EVM contract-creation State.
pub struct CreateEvmContract<K: MfmValueTrait>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for CreateEvmContract<K> {
    type Input = EvmContractCreationContext<K>;
    type Output = EvmContractCreationCompletion<K>;
    type Failure = EvmContractCreationFailure<K>;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new(CREATE_EVM_CONTRACT_STATE_ID).map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> EffectState<EvmTransactionEffect> for CreateEvmContract<K> {
    fn prepare(input: &Self::Input) -> Result<Eip1559TransactionCommand, PreparationError> {
        input
            .validate()
            .map(|_| input.command.clone())
            .map_err(|_| PreparationError)
    }

    fn interpret(
        input: Self::Input,
        evidence: &EvmTransactionSettlement,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        match evidence {
            EvmTransactionSettlement::Confirmed {
                confirmation:
                    EvmTransactionConfirmation::Created {
                        block_anchor,
                        created_address,
                        transaction_hash,
                    },
                ..
            } => ProposedStateOutcome::Success {
                output: EvmContractCreationCompletion {
                    block_anchor: block_anchor.clone(),
                    binding: input.command.binding().clone(),
                    caller_context: input.caller_context,
                    created_address: created_address.clone(),
                    transaction_hash: transaction_hash.clone(),
                },
            },
            EvmTransactionSettlement::Confirmed {
                confirmation: EvmTransactionConfirmation::Called { .. },
                ..
            } => ProposedStateOutcome::Failure {
                failure: EvmContractCreationFailure::InconsistentSettlement {
                    caller_context: input.caller_context,
                },
            },
            EvmTransactionSettlement::Reverted { revert, .. } => ProposedStateOutcome::Failure {
                failure: EvmContractCreationFailure::Reverted {
                    caller_context: input.caller_context,
                    revert: revert.clone(),
                },
            },
        }
    }
}

impl<K: MfmValueTrait> CapabilityInjection<CreateEvmContract<K>> for EvmTransactionEffect {
    type Setup = EvmTransactionBinding;
    type ExpandedInput = EvmContractCreationContext<K>;
    type ExpandedOutput = EvmContractCreationCompletion<K>;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

/// Context-preserving EVM contract-call State.
pub struct CallEvmContract<K: MfmValueTrait>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for CallEvmContract<K> {
    type Input = EvmContractCallContext<K>;
    type Output = EvmContractCallCompletion<K>;
    type Failure = EvmContractCallFailure<K>;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new(CALL_EVM_CONTRACT_STATE_ID).map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> EffectState<EvmTransactionEffect> for CallEvmContract<K> {
    fn prepare(input: &Self::Input) -> Result<Eip1559TransactionCommand, PreparationError> {
        input
            .validate()
            .map(|_| input.command.clone())
            .map_err(|_| PreparationError)
    }

    fn interpret(
        input: Self::Input,
        evidence: &EvmTransactionSettlement,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        match evidence {
            EvmTransactionSettlement::Confirmed {
                confirmation:
                    EvmTransactionConfirmation::Called {
                        block_anchor,
                        transaction_hash,
                    },
                ..
            } => {
                let Some(to) = input.command.to() else {
                    return ProposedStateOutcome::Failure {
                        failure: EvmContractCallFailure::InconsistentSettlement {
                            caller_context: input.caller_context,
                        },
                    };
                };
                ProposedStateOutcome::Success {
                    output: EvmContractCallCompletion {
                        block_anchor: block_anchor.clone(),
                        binding: input.command.binding().clone(),
                        caller_context: input.caller_context,
                        target: to.clone(),
                        transaction_hash: transaction_hash.clone(),
                    },
                }
            }
            EvmTransactionSettlement::Confirmed {
                confirmation: EvmTransactionConfirmation::Created { .. },
                ..
            } => ProposedStateOutcome::Failure {
                failure: EvmContractCallFailure::InconsistentSettlement {
                    caller_context: input.caller_context,
                },
            },
            EvmTransactionSettlement::Reverted { revert, .. } => ProposedStateOutcome::Failure {
                failure: EvmContractCallFailure::Reverted {
                    caller_context: input.caller_context,
                    revert: revert.clone(),
                },
            },
        }
    }
}

impl<K: MfmValueTrait> CapabilityInjection<CallEvmContract<K>> for EvmTransactionEffect {
    type Setup = EvmTransactionBinding;
    type ExpandedInput = EvmContractCallContext<K>;
    type ExpandedOutput = EvmContractCallCompletion<K>;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

fn decode_fixed_hex<const N: usize>(encoded: &str) -> Result<[u8; N], EvmDomainError> {
    if encoded.len() != 2 + N * 2 || !encoded.starts_with("0x") {
        return Err(EvmDomainError::InvalidValue);
    }
    let mut decoded = [0_u8; N];
    for (output, pair) in decoded
        .iter_mut()
        .zip(encoded.as_bytes()[2..].chunks_exact(2))
    {
        let high = decode_lower_hex_nibble(pair[0]).ok_or(EvmDomainError::InvalidValue)?;
        let low = decode_lower_hex_nibble(pair[1]).ok_or(EvmDomainError::InvalidValue)?;
        *output = (high << 4) | low;
    }
    Ok(decoded)
}

fn decode_lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn encode_fixed_hex<const N: usize>(bytes: &[u8; N]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(2 + N * 2);
    encoded.push_str("0x");
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn write_fixed_hex<const N: usize>(
    bytes: &[u8; N],
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    formatter.write_str("0x")?;
    for byte in bytes {
        formatter.write_fmt(format_args!("{byte:02x}"))?;
    }
    Ok(())
}
