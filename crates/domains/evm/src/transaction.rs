use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroU64;

use mfm_canonical::CanonicalBytes;
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::de;
use serde::{Deserialize, Serialize};

use crate::EvmDomainError;

/// Exact Effect capability identity for EIP-1559 transaction execution.
pub const EVM_TRANSACTION_EFFECT_CAPABILITY_ID: &str = "mfm.evm.capability.execute-transaction@2";

const MAX_U256_DECIMAL: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";
/// Maximum EVM contract-creation initcode bytes admitted by one command.
pub const MAX_EVM_INITCODE_BYTES: usize = 49_152;
/// Maximum EVM call calldata bytes admitted by one command or anchored Read.
pub const MAX_EVM_CALLDATA_BYTES: usize = 131_072;

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
    /// Chain id.
    pub chain_id: NonZeroU64,
    /// Expected genesis hash.
    pub expected_genesis_hash: EvmHash,
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
    /// Chain instance.
    pub chain_instance: EvmChainInstance,
    /// Endpoint ref.
    pub endpoint_ref: ContentRef,
}

impl EvmTransactionRoute {
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
    /// Authority epoch.
    pub authority_epoch: EvmAuthorityEpoch,
    /// Route.
    pub route: EvmTransactionRoute,
    /// Sender.
    pub sender: EvmAddress,
}

impl EvmTransactionBinding {
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
        let maximum = match &self.action {
            TransactionAction::Create { .. } => MAX_EVM_INITCODE_BYTES,
            TransactionAction::Call { .. } => MAX_EVM_CALLDATA_BYTES,
        };
        validate_transaction_parameters(
            self.input(),
            maximum,
            &self.max_priority_fee_per_gas,
            &self.max_fee_per_gas,
        )
    }
}

impl_checked_deserialize!(Eip1559TransactionCommand {
    action: TransactionAction,
    binding: EvmTransactionBinding,
    gas_limit: NonZeroU64,
    max_fee_per_gas: EvmU256,
    max_priority_fee_per_gas: EvmU256,
    value: EvmU256,
});

/// Shared receipt facts authenticated for one settled transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-receipt",
    version = "1",
    schema = "mfm.evm-transaction-receipt"
)]
pub struct EvmTransactionReceipt {
    /// Block anchor.
    pub block_anchor: crate::EvmBlockAnchor,
    /// Transaction hash.
    pub transaction_hash: EvmHash,
}

/// Closed outcome authenticated for one settled transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-outcome",
    version = "1",
    schema = "mfm.evm-transaction-outcome"
)]
pub enum EvmTransactionOutcome {
    /// A contract creation succeeded.
    Created {
        /// Receipt-derived created contract address.
        created_address: EvmAddress,
    },
    /// An ordinary contract call succeeded.
    Called,
    /// The transaction was mined with a reverted status.
    Reverted,
}

impl EvmTransactionOutcome {
    /// Returns the created contract address when this is a successful creation.
    pub const fn created_address(&self) -> Option<&EvmAddress> {
        match self {
            Self::Created { created_address } => Some(created_address),
            Self::Called | Self::Reverted => None,
        }
    }
}

/// Complete durable settlement evidence for one Effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-settlement",
    version = "1",
    schema = "mfm.evm-transaction-settlement"
)]
pub struct EvmTransactionSettlement {
    effect_id: EffectId,
    nonce: u64,
    receipt: EvmTransactionReceipt,
    outcome: EvmTransactionOutcome,
}

impl EvmTransactionSettlement {
    /// Constructs one successful contract-creation settlement.
    pub fn created(
        effect_id: EffectId,
        nonce: u64,
        receipt: EvmTransactionReceipt,
        created_address: EvmAddress,
    ) -> Self {
        Self {
            effect_id,
            nonce,
            receipt,
            outcome: EvmTransactionOutcome::Created { created_address },
        }
    }

    /// Constructs one successful contract-call settlement.
    pub fn called(effect_id: EffectId, nonce: u64, receipt: EvmTransactionReceipt) -> Self {
        Self {
            effect_id,
            nonce,
            receipt,
            outcome: EvmTransactionOutcome::Called,
        }
    }

    /// Constructs one reverted transaction settlement.
    pub fn reverted(effect_id: EffectId, nonce: u64, receipt: EvmTransactionReceipt) -> Self {
        Self {
            effect_id,
            nonce,
            receipt,
            outcome: EvmTransactionOutcome::Reverted,
        }
    }

    /// Returns the settled Effect identity.
    pub const fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }

    /// Returns the reserved transaction nonce.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Returns the shared checked receipt facts.
    pub const fn receipt(&self) -> &EvmTransactionReceipt {
        &self.receipt
    }

    /// Returns the closed settlement outcome.
    pub const fn outcome(&self) -> &EvmTransactionOutcome {
        &self.outcome
    }

    /// Returns the transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.receipt.transaction_hash
    }

    /// Returns the canonical receipt block anchor.
    pub const fn block_anchor(&self) -> &crate::EvmBlockAnchor {
        &self.receipt.block_anchor
    }
}

fn validate_transaction_parameters(
    input: &[u8],
    maximum: usize,
    priority_fee: &EvmU256,
    max_fee: &EvmU256,
) -> Result<(), EvmDomainError> {
    validate_input_bytes(input, maximum)?;
    if max_fee.to_u128().is_none()
        || priority_fee.to_u128().is_none()
        || max_fee.numeric_cmp(priority_fee) == Ordering::Less
    {
        return Err(EvmDomainError::InvalidValue);
    }
    Ok(())
}

pub(crate) fn validate_input_bytes(input: &[u8], maximum: usize) -> Result<(), EvmDomainError> {
    (input.len() <= maximum)
        .then_some(())
        .ok_or(EvmDomainError::InvalidValue)
}

pub(crate) mod recipes;
pub use recipes::*;

mod report;
pub use report::*;

mod facts;
pub use facts::*;

mod plans;
pub use plans::*;

mod stages;
pub use stages::*;

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

#[cfg(test)]
mod tests {
    use mfm_values::{canonicalize_mfm_value, MfmValue as _};

    use super::*;

    #[test]
    fn private_action_identity_and_wire_are_frozen() {
        let semantic = TransactionAction::semantic_id()
            .expect("action semantic id")
            .to_string();
        let schema = TransactionAction::schema_descriptor()
            .expect("action descriptor")
            .schema_id()
            .expect("action schema id")
            .to_string();
        assert_eq!(
            semantic,
            "semantic:mfm.evm:eip1559-transaction-action:1:sha256-jcs-v1:e9ea0b242e14471a3ed2996b657ec0e3b3d9b8aeaa00b215e2d866afd8af1bd9"
        );
        assert_eq!(
            schema,
            "schema:mfm.evm-eip1559-transaction-action:1:sha256-jcs-v1:071ba5027b04d00c5d85d91070c9f8d62ff02ec213bbcc94a55568ba73b9b05e"
        );
        let create = TransactionAction::Create {
            initcode: CanonicalBytes::new([1, 2, 3]),
        };
        assert_eq!(
            canonicalize_mfm_value(&create)
                .expect("canonical action")
                .0
                .as_str(),
            r#"{"kind":"create","value":{"initcode":"AQID"}}"#
        );
        let call = TransactionAction::Call {
            to: EvmAddress::from_bytes([0x22; 20]),
            calldata: CanonicalBytes::new([4, 5, 6]),
        };
        assert_eq!(
            canonicalize_mfm_value(&call)
                .expect("canonical action")
                .0
                .as_str(),
            r#"{"kind":"call","value":{"calldata":"BAUG","to":"0x2222222222222222222222222222222222222222"}}"#
        );
    }
}
