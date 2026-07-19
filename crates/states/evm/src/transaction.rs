//! One-transaction EVM side-effect state and deterministic evidence reducers.

use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_effects::ApplySideEffect;
use mfm_evm_capabilities::{
    EvmBlockAnchor, EvmFeeInputs, EvmNetworkBinding,
    EvmObservedTransaction as CapabilityTransaction, EvmReceipt as CapabilityReceipt,
    EvmReceiptStatus as CapabilityReceiptStatus, EvmSessionEvidence, EvmTransactionEstimate,
};
use mfm_ids::LocalPublicId;
use mfm_program::{
    AdapterBindingSpec, NoContext, ResourceClaim, ResourceNamespace, SideEffectIntent,
    SideEffectState, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_signing::{SignerRef, SigningCapability, SECP256K1_RFC6979_LOW_S_PROFILE_ID};
use mfm_values::MfmValue as _;
use serde::{Deserialize, Serialize};

use crate::canonical::{
    block_anchor_hash, block_anchor_number, canonical_address, canonical_bytes, canonical_hash,
    invalid, parse_address, parse_bytes, parse_hash, parse_quantity, validate_block_anchor,
    validate_session,
};
use crate::identity::{adapter_binding, state_kind, state_version};
use crate::EvmStateError;

/// Maximum init-code or calldata size admitted into typed transaction intent.
pub const EVM_TRANSACTION_DATA_MAX_BYTES: usize = 128 * 1024;
/// The only admitted EIP-1559 fee policy.
pub const EVM_TRANSACTION_FEE_POLICY: &str = "base_fee_x2_plus_priority";
/// The only admitted transaction gas-limit policy.
pub const EVM_GAS_POLICY: &str = "exact_estimate";
/// Exclusive resource namespace for one semantic network, chain, and sender nonce lane.
pub const EVM_SENDER_LANE_NAMESPACE: &str = "mfm.evm.sender_nonce";

const MAX_ACCESS_LIST_ENTRIES: usize = 256;
const MAX_ACCESS_LIST_STORAGE_KEYS: usize = 4_096;
const MAX_RECEIPT_LOGS: usize = 4_096;
const MAX_LOG_TOPICS: usize = 4;
const MAX_RECEIPT_LOG_DATA_BYTES: usize = 4 * 1024 * 1024;

/// One checked EIP-2930 access-list entry retained in transaction authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_access_list_entry",
    version = "1",
    schema = "mfm.evm.transaction.access_list_entry"
)]
pub struct EvmAccessListEntry {
    address: String,
    storage_keys: Vec<String>,
}

impl EvmAccessListEntry {
    /// Creates a canonical access-list entry from Alloy primitives.
    pub fn new(address: Address, storage_keys: Vec<B256>) -> Self {
        Self {
            address: canonical_address(address),
            storage_keys: storage_keys.into_iter().map(canonical_hash).collect(),
        }
    }

    /// Returns the canonical account address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns canonical storage keys in transaction order.
    pub fn storage_keys(&self) -> &[String] {
        &self.storage_keys
    }

    pub(crate) fn validate(&self) -> Result<(), EvmStateError> {
        parse_address(&self.address)?;
        for key in &self.storage_keys {
            parse_hash(key)?;
        }
        Ok(())
    }

    pub(crate) fn to_alloy(&self) -> Result<AccessListItem, EvmStateError> {
        Ok(AccessListItem {
            address: parse_address(&self.address)?,
            storage_keys: self
                .storage_keys
                .iter()
                .map(|key| parse_hash(key))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

/// Closed action for one signed EVM transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_action",
    version = "1",
    schema = "mfm.evm.transaction.action"
)]
pub enum EvmTransactionAction {
    /// Direct EOA contract creation.
    Create {
        /// Canonical lower-case `0x`-prefixed init code.
        init_code: String,
        /// Canonical decimal wei value.
        value: String,
    },
    /// Ordinary call, including native transfers when calldata is empty.
    Call {
        /// Canonical lower-case destination address.
        to: String,
        /// Canonical lower-case `0x`-prefixed calldata.
        calldata: String,
        /// Canonical decimal wei value.
        value: String,
    },
}

impl EvmTransactionAction {
    /// Creates one direct contract-creation action.
    pub fn create(init_code: impl AsRef<[u8]>, value: U256) -> Result<Self, EvmStateError> {
        let action = Self::Create {
            init_code: canonical_bytes(init_code.as_ref()),
            value: value.to_string(),
        };
        action.validate()?;
        Ok(action)
    }

    /// Creates one ordinary call action. Empty calldata is a native transfer.
    pub fn call(
        to: Address,
        calldata: impl AsRef<[u8]>,
        value: U256,
    ) -> Result<Self, EvmStateError> {
        let action = Self::Call {
            to: canonical_address(to),
            calldata: canonical_bytes(calldata.as_ref()),
            value: value.to_string(),
        };
        action.validate()?;
        Ok(action)
    }

    /// Returns the action category.
    pub const fn action_kind(&self) -> EvmTransactionActionKind {
        match self {
            Self::Create { .. } => EvmTransactionActionKind::Create,
            Self::Call { .. } => EvmTransactionActionKind::Call,
        }
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        match self {
            Self::Create { init_code, value } => {
                let bytes = parse_bytes(init_code, EVM_TRANSACTION_DATA_MAX_BYTES)?;
                if bytes.is_empty() {
                    return Err(invalid("create init_code must not be empty"));
                }
                parse_quantity(value)?;
            }
            Self::Call {
                to,
                calldata,
                value,
            } => {
                parse_address(to)?;
                parse_bytes(calldata, EVM_TRANSACTION_DATA_MAX_BYTES)?;
                parse_quantity(value)?;
            }
        }
        Ok(())
    }

    fn transaction_fields(&self) -> Result<(TxKind, U256, Bytes), EvmStateError> {
        match self {
            Self::Create { init_code, value } => Ok((
                TxKind::Create,
                parse_quantity(value)?,
                parse_bytes(init_code, EVM_TRANSACTION_DATA_MAX_BYTES)?.into(),
            )),
            Self::Call {
                to,
                calldata,
                value,
            } => Ok((
                TxKind::Call(parse_address(to)?),
                parse_quantity(value)?,
                parse_bytes(calldata, EVM_TRANSACTION_DATA_MAX_BYTES)?.into(),
            )),
        }
    }

    fn call_destination(&self) -> Result<Option<Address>, EvmStateError> {
        match self {
            Self::Create { .. } => Ok(None),
            Self::Call { to, .. } => parse_address(to).map(Some),
        }
    }
}

/// Action category retained for explicit reverted outcomes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_action_kind",
    version = "1",
    schema = "mfm.evm.transaction.action_kind"
)]
pub enum EvmTransactionActionKind {
    /// Direct creation.
    Create,
    /// Ordinary call.
    Call,
}

/// Certified configuration for one reusable transaction state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submit_transaction_config",
    version = "1",
    schema = "mfm.evm.state.config.submit_transaction",
    validate = "validate_transaction_config"
)]
pub struct EvmTransactionConfig {
    network_id: String,
    chain_id: u64,
    expected_sender: String,
    signer_ref: String,
    signing_profile: String,
    fee_policy: String,
    gas_policy: String,
    access_list: Vec<EvmAccessListEntry>,
}

impl EvmTransactionConfig {
    /// Creates the one admitted deterministic EIP-1559 transaction configuration.
    pub fn new(
        network_id: impl Into<String>,
        chain_id: u64,
        expected_sender: Address,
        signer_ref: SignerRef,
        access_list: Vec<EvmAccessListEntry>,
    ) -> Result<Self, EvmStateError> {
        let config = Self {
            network_id: network_id.into(),
            chain_id,
            expected_sender: canonical_address(expected_sender),
            signer_ref: signer_ref.to_string(),
            signing_profile: SECP256K1_RFC6979_LOW_S_PROFILE_ID.to_owned(),
            fee_policy: EVM_TRANSACTION_FEE_POLICY.to_owned(),
            gas_policy: EVM_GAS_POLICY.to_owned(),
            access_list,
        };
        validate_transaction_config(&config).map_err(EvmStateError::invalid)?;
        Ok(config)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the exact EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the expected sender.
    pub fn expected_sender(&self) -> &str {
        &self.expected_sender
    }

    /// Returns the process-local signer reference.
    pub fn signer_ref(&self) -> &str {
        &self.signer_ref
    }

    /// Returns the deterministic signing profile.
    pub fn signing_profile(&self) -> &str {
        &self.signing_profile
    }

    /// Returns the exact access list.
    pub fn access_list(&self) -> &[EvmAccessListEntry] {
        &self.access_list
    }

    /// Returns the checked semantic network binding.
    pub fn network_binding(&self) -> Result<EvmNetworkBinding, EvmStateError> {
        EvmNetworkBinding::new(
            LocalPublicId::new(&self.network_id).map_err(|_| invalid("network_id was invalid"))?,
            self.chain_id,
        )
        .map_err(|_| invalid("network binding was invalid"))
    }

    /// Returns the checked signer reference.
    pub fn signer_reference(&self) -> Result<SignerRef, EvmStateError> {
        SignerRef::new(&self.signer_ref).map_err(|_| invalid("signer_ref was invalid"))
    }
}

fn validate_transaction_config(config: &EvmTransactionConfig) -> Result<(), String> {
    LocalPublicId::new(&config.network_id).map_err(|_| "network_id was invalid".to_owned())?;
    if config.chain_id == 0 {
        return Err("chain_id must be non-zero".to_owned());
    }
    parse_address(&config.expected_sender).map_err(|error| error.to_string())?;
    SignerRef::new(&config.signer_ref).map_err(|_| "signer_ref was invalid".to_owned())?;
    if config.signing_profile != SECP256K1_RFC6979_LOW_S_PROFILE_ID {
        return Err("signing_profile is not the deterministic EVM profile".to_owned());
    }
    if config.fee_policy != EVM_TRANSACTION_FEE_POLICY {
        return Err("fee_policy is not supported".to_owned());
    }
    if config.gas_policy != EVM_GAS_POLICY {
        return Err("gas_policy is not supported".to_owned());
    }
    validate_access_list(&config.access_list).map_err(|error| error.to_string())
}

/// Immutable authored transaction intent and idempotency authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_intent",
    version = "1",
    schema = "mfm.evm.transaction.intent"
)]
pub struct EvmTransactionIntent {
    network_id: String,
    chain_id: u64,
    expected_sender: String,
    signer_ref: String,
    signing_profile: String,
    fee_policy: String,
    gas_policy: String,
    action: EvmTransactionAction,
    access_list: Vec<EvmAccessListEntry>,
}

impl EvmTransactionIntent {
    /// Builds immutable authored intent from checked config and one action.
    pub fn from_config(
        config: &EvmTransactionConfig,
        action: &EvmTransactionAction,
    ) -> Result<Self, EvmStateError> {
        validate_transaction_config(config).map_err(EvmStateError::invalid)?;
        action.validate()?;
        let intent = Self {
            network_id: config.network_id.clone(),
            chain_id: config.chain_id,
            expected_sender: config.expected_sender.clone(),
            signer_ref: config.signer_ref.clone(),
            signing_profile: config.signing_profile.clone(),
            fee_policy: config.fee_policy.clone(),
            gas_policy: config.gas_policy.clone(),
            action: action.clone(),
            access_list: config.access_list.clone(),
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the exact chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical expected sender.
    pub fn expected_sender(&self) -> &str {
        &self.expected_sender
    }

    /// Returns the exact signer reference.
    pub fn signer_ref(&self) -> &str {
        &self.signer_ref
    }

    /// Returns the certified deterministic signing profile.
    pub fn signing_profile(&self) -> &str {
        &self.signing_profile
    }

    /// Returns the closed transaction action.
    pub const fn action(&self) -> &EvmTransactionAction {
        &self.action
    }

    /// Returns the exact access list.
    pub fn access_list(&self) -> &[EvmAccessListEntry] {
        &self.access_list
    }

    /// Returns the exclusive sender lane authority.
    pub fn sender_lane(&self) -> EvmSenderLane {
        EvmSenderLane {
            network_id: self.network_id.clone(),
            chain_id: self.chain_id,
            expected_sender: self.expected_sender.clone(),
        }
    }

    /// Validates every authored field without live resources.
    pub fn validate(&self) -> Result<(), EvmStateError> {
        let config = EvmTransactionConfig {
            network_id: self.network_id.clone(),
            chain_id: self.chain_id,
            expected_sender: self.expected_sender.clone(),
            signer_ref: self.signer_ref.clone(),
            signing_profile: self.signing_profile.clone(),
            fee_policy: self.fee_policy.clone(),
            gas_policy: self.gas_policy.clone(),
            access_list: self.access_list.clone(),
        };
        validate_transaction_config(&config).map_err(EvmStateError::invalid)?;
        self.action.validate()
    }

    /// Converts the retained access list to Alloy transaction authority.
    pub fn alloy_access_list(&self) -> Result<AccessList, EvmStateError> {
        Ok(AccessList(
            self.access_list
                .iter()
                .map(EvmAccessListEntry::to_alloy)
                .collect::<Result<Vec<_>, _>>()?,
        ))
    }

    /// Returns the checked semantic network binding.
    pub fn network_binding(&self) -> Result<EvmNetworkBinding, EvmStateError> {
        self.validate()?;
        EvmNetworkBinding::new(
            LocalPublicId::new(&self.network_id).map_err(|_| invalid("network_id was invalid"))?,
            self.chain_id,
        )
        .map_err(|_| invalid("network binding was invalid"))
    }

    /// Returns the checked expected sender.
    pub fn expected_sender_address(&self) -> Result<Address, EvmStateError> {
        parse_address(&self.expected_sender)
    }

    /// Returns the checked signer reference.
    pub fn signer_reference(&self) -> Result<SignerRef, EvmStateError> {
        SignerRef::new(&self.signer_ref).map_err(|_| invalid("signer_ref was invalid"))
    }

    /// Admits intent and exact nonce/fee observations into one pre-gas transaction description.
    pub fn transaction_estimate(
        &self,
        pending_nonce: U256,
        fees: &EvmFeeInputs,
    ) -> Result<EvmTransactionEstimate, EvmStateError> {
        self.validate()?;
        let recomputed_fees = EvmFeeInputs::from_base_and_priority(
            fees.base_fee_per_gas,
            fees.max_priority_fee_per_gas,
        )
        .map_err(|_| invalid("fee observations were invalid"))?;
        if &recomputed_fees != fees {
            return Err(invalid(
                "fee observations did not match the certified policy",
            ));
        }
        let (to, value, input) = self.action.transaction_fields()?;
        EvmTransactionEstimate::new(
            U256::from(self.chain_id),
            pending_nonce,
            self.expected_sender_address()?,
            to,
            value,
            input,
            self.alloy_access_list()?,
            fees.max_fee_per_gas,
            fees.max_priority_fee_per_gas,
        )
        .map_err(|_| invalid("transaction observations exceeded EIP-1559 bounds"))
    }
}

/// Typed exclusive lane key for a semantic network, chain, and sender.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "sender_nonce_lane",
    version = "1",
    schema = "mfm.evm.transaction.sender_nonce_lane"
)]
pub struct EvmSenderLane {
    network_id: String,
    chain_id: u64,
    expected_sender: String,
}

impl EvmSenderLane {
    /// Returns the stable store-comparable key for this typed tuple.
    pub fn resource_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.network_id, self.chain_id, self.expected_sender
        )
    }
}

/// Returns the required exclusive sender-nonce lane declaration for transaction nodes.
pub fn evm_sender_lane_resource_claim() -> mfm_program::Result<ResourceClaim> {
    let namespace = ResourceNamespace::new(EVM_SENDER_LANE_NAMESPACE)
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    let schema = EvmSenderLane::schema_id()
        .map_err(|error| mfm_program::PlanError::Value(error.to_string()))?;
    Ok(ResourceClaim::exclusive(namespace, schema))
}

/// Canonical unsigned EIP-1559 envelope retained as public prepared authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "unsigned_transaction",
    version = "1",
    schema = "mfm.evm.transaction.unsigned"
)]
pub struct EvmUnsignedTransaction {
    chain_id: u64,
    nonce: String,
    max_priority_fee_per_gas: String,
    max_fee_per_gas: String,
    gas_limit: String,
    to: Option<String>,
    value: String,
    access_list: Vec<EvmAccessListEntry>,
    input: String,
}

impl EvmUnsignedTransaction {
    /// Adds the checked gas-limit policy to the exact estimated transaction description.
    pub fn from_estimate(
        estimate: &EvmTransactionEstimate,
        gas_estimate: U256,
    ) -> Result<Self, EvmStateError> {
        if gas_estimate.is_zero() {
            return Err(invalid("gas estimate must be non-zero"));
        }
        let to = match estimate.to() {
            TxKind::Create => None,
            TxKind::Call(address) => Some(canonical_address(address)),
        };
        let unsigned = Self {
            chain_id: u64::try_from(estimate.chain_id())
                .map_err(|_| invalid("chain id exceeded EIP-1559 range"))?,
            nonce: estimate.nonce().to_string(),
            max_priority_fee_per_gas: estimate.max_priority_fee_per_gas().to_string(),
            max_fee_per_gas: estimate.max_fee_per_gas().to_string(),
            gas_limit: gas_estimate.to_string(),
            to,
            value: estimate.value().to_string(),
            access_list: access_list_from_alloy(estimate.access_list()),
            input: canonical_bytes(estimate.input()),
        };
        unsigned.validate()?;
        Ok(unsigned)
    }

    /// Returns the EIP-2718 transaction type.
    pub const fn transaction_type(&self) -> u8 {
        mfm_evm_capabilities::EVM_EIP1559_TRANSACTION_TYPE
    }

    /// Returns the exact chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical nonce.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// Returns the canonical maximum priority fee per gas.
    pub fn max_priority_fee_per_gas(&self) -> &str {
        &self.max_priority_fee_per_gas
    }

    /// Returns the canonical maximum total fee per gas.
    pub fn max_fee_per_gas(&self) -> &str {
        &self.max_fee_per_gas
    }

    /// Returns the canonical gas limit.
    pub fn gas_limit(&self) -> &str {
        &self.gas_limit
    }

    /// Returns the call destination, or `None` for direct creation.
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }

    /// Returns the canonical transferred value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the exact access list.
    pub fn access_list(&self) -> &[EvmAccessListEntry] {
        &self.access_list
    }

    /// Returns the exact canonical transaction input.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Reconstructs the canonical Alloy signing envelope.
    pub fn to_signing_envelope(
        &self,
    ) -> Result<mfm_evm_signing::UnsignedEip1559Envelope, EvmStateError> {
        self.validate()?;
        let to = match &self.to {
            Some(to) => TxKind::Call(parse_address(to)?),
            None => TxKind::Create,
        };
        mfm_evm_signing::UnsignedEip1559Envelope::new(
            U256::from(self.chain_id),
            parse_quantity(&self.nonce)?,
            parse_quantity(&self.max_priority_fee_per_gas)?,
            parse_quantity(&self.max_fee_per_gas)?,
            parse_quantity(&self.gas_limit)?,
            to,
            parse_quantity(&self.value)?,
            AccessList(
                self.access_list
                    .iter()
                    .map(EvmAccessListEntry::to_alloy)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            parse_bytes(&self.input, EVM_TRANSACTION_DATA_MAX_BYTES)?.into(),
        )
        .map_err(|_| invalid("unsigned EIP-1559 envelope was invalid"))
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        if self.chain_id == 0 {
            return Err(invalid("unsigned chain_id must be non-zero"));
        }
        parse_quantity(&self.nonce)?;
        let priority = parse_quantity(&self.max_priority_fee_per_gas)?;
        let maximum = parse_quantity(&self.max_fee_per_gas)?;
        if priority > maximum {
            return Err(invalid("priority fee exceeded maximum fee"));
        }
        if parse_quantity(&self.gas_limit)?.is_zero() {
            return Err(invalid("gas limit must be non-zero"));
        }
        if let Some(to) = &self.to {
            parse_address(to)?;
        }
        parse_quantity(&self.value)?;
        validate_access_list(&self.access_list)?;
        parse_bytes(&self.input, EVM_TRANSACTION_DATA_MAX_BYTES)?;
        self.to_signing_envelope_shape_only()
    }

    fn to_signing_envelope_shape_only(&self) -> Result<(), EvmStateError> {
        u64::try_from(parse_quantity(&self.nonce)?)
            .map_err(|_| invalid("nonce exceeded EIP-1559 range"))?;
        u64::try_from(parse_quantity(&self.gas_limit)?)
            .map_err(|_| invalid("gas limit exceeded EIP-1559 range"))?;
        u128::try_from(parse_quantity(&self.max_priority_fee_per_gas)?)
            .map_err(|_| invalid("priority fee exceeded EIP-1559 range"))?;
        u128::try_from(parse_quantity(&self.max_fee_per_gas)?)
            .map_err(|_| invalid("maximum fee exceeded EIP-1559 range"))?;
        Ok(())
    }
}

/// Immutable prepared invocation authority fixed before submission starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "prepared_transaction",
    version = "1",
    schema = "mfm.evm.transaction.prepared"
)]
pub struct EvmPreparedTransaction {
    intent: EvmTransactionIntent,
    pending_nonce: String,
    base_fee_per_gas: String,
    max_priority_fee_per_gas: String,
    max_fee_per_gas: String,
    gas_estimate: String,
    unsigned: EvmUnsignedTransaction,
    signing_digest: String,
    expected_transaction_hash: String,
    expected_create_address: Option<String>,
    preparation_session: EvmSessionEvidence,
}

impl EvmPreparedTransaction {
    /// Constructs prepared public authority after one deterministic signature fixed the hash.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        intent: EvmTransactionIntent,
        pending_nonce: U256,
        fees: EvmFeeInputs,
        gas_estimate: U256,
        unsigned: EvmUnsignedTransaction,
        expected_transaction_hash: B256,
        preparation_session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        let envelope = unsigned.to_signing_envelope()?;
        let expected_create_address = match intent.action {
            EvmTransactionAction::Create { .. } => Some(canonical_address(
                parse_address(&intent.expected_sender)?.create(
                    u64::try_from(pending_nonce)
                        .map_err(|_| invalid("creation nonce exceeded EIP-1559 range"))?,
                ),
            )),
            EvmTransactionAction::Call { .. } => None,
        };
        let prepared = Self {
            intent,
            pending_nonce: pending_nonce.to_string(),
            base_fee_per_gas: fees.base_fee_per_gas.to_string(),
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas.to_string(),
            max_fee_per_gas: fees.max_fee_per_gas.to_string(),
            gas_estimate: gas_estimate.to_string(),
            unsigned,
            signing_digest: canonical_hash(envelope.signing_digest()),
            expected_transaction_hash: canonical_hash(expected_transaction_hash),
            expected_create_address,
            preparation_session: preparation_session.clone(),
        };
        prepared.validate()?;
        Ok(prepared)
    }

    /// Returns the immutable authored intent.
    pub const fn intent(&self) -> &EvmTransactionIntent {
        &self.intent
    }

    /// Returns the retained unsigned envelope.
    pub const fn unsigned(&self) -> &EvmUnsignedTransaction {
        &self.unsigned
    }

    /// Returns the canonical signing digest.
    pub fn signing_digest(&self) -> &str {
        &self.signing_digest
    }

    /// Returns the locally computed transaction hash.
    pub fn expected_transaction_hash(&self) -> &str {
        &self.expected_transaction_hash
    }

    /// Returns the direct CREATE address, when the action is creation.
    pub fn expected_create_address(&self) -> Option<&str> {
        self.expected_create_address.as_deref()
    }

    /// Reconstructs the exact unsigned signing envelope.
    pub fn signing_envelope(
        &self,
    ) -> Result<mfm_evm_signing::UnsignedEip1559Envelope, EvmStateError> {
        self.validate()?;
        self.unsigned.to_signing_envelope()
    }

    /// Parses the expected transaction hash.
    pub fn expected_hash(&self) -> Result<B256, EvmStateError> {
        parse_hash(&self.expected_transaction_hash)
    }

    /// Validates all prepared relations without signer or network access.
    pub fn validate(&self) -> Result<(), EvmStateError> {
        self.intent.validate()?;
        validate_session(
            &self.preparation_session,
            self.intent.network_id(),
            self.intent.chain_id(),
        )?;
        let nonce = parse_quantity(&self.pending_nonce)?;
        let fees = EvmFeeInputs::from_base_and_priority(
            parse_quantity(&self.base_fee_per_gas)?,
            parse_quantity(&self.max_priority_fee_per_gas)?,
        )
        .map_err(|_| invalid("prepared fee observations were invalid"))?;
        if fees.max_fee_per_gas != parse_quantity(&self.max_fee_per_gas)? {
            return Err(invalid(
                "prepared maximum fee did not match fee observations",
            ));
        }
        let gas = parse_quantity(&self.gas_estimate)?;
        let estimate = self.intent.transaction_estimate(nonce, &fees)?;
        let recomputed = EvmUnsignedTransaction::from_estimate(&estimate, gas)?;
        if recomputed != self.unsigned {
            return Err(invalid(
                "prepared unsigned envelope did not match observations",
            ));
        }
        let envelope = self.unsigned.to_signing_envelope()?;
        if canonical_hash(envelope.signing_digest()) != self.signing_digest {
            return Err(invalid(
                "prepared signing digest did not match unsigned envelope",
            ));
        }
        parse_hash(&self.expected_transaction_hash)?;
        let expected_create_address = match self.intent.action {
            EvmTransactionAction::Create { .. } => Some(canonical_address(
                parse_address(&self.intent.expected_sender)?.create(
                    u64::try_from(nonce)
                        .map_err(|_| invalid("creation nonce exceeded EIP-1559 range"))?,
                ),
            )),
            EvmTransactionAction::Call { .. } => None,
        };
        if self.expected_create_address != expected_create_address {
            return Err(invalid(
                "prepared CREATE address did not match sender and nonce",
            ));
        }
        Ok(())
    }
}

/// Typed observed submission with no signature scalars or raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_submission",
    version = "1",
    schema = "mfm.evm.transaction.submission"
)]
pub struct EvmTransactionSubmission {
    transaction_hash: String,
    from: String,
    unsigned: EvmUnsignedTransaction,
    session: EvmSessionEvidence,
}

impl EvmTransactionSubmission {
    /// Converts a checked provider acknowledgement into submission evidence.
    ///
    /// The submitted bytes are already bound to [`EvmPreparedTransaction`] by their locally
    /// computed hash. A provider acknowledgement of that exact hash therefore proves the public
    /// transaction fields without requiring a second RPC observation.
    pub fn from_acknowledgement(
        prepared: &EvmPreparedTransaction,
        acknowledged_hash: B256,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        let submission = Self {
            transaction_hash: canonical_hash(acknowledged_hash),
            from: prepared.intent.expected_sender.clone(),
            unsigned: prepared.unsigned.clone(),
            session: session.clone(),
        };
        submission.validate_against(prepared)?;
        Ok(submission)
    }

    /// Converts and validates a transaction lookup against prepared authority.
    pub fn from_observation(
        prepared: &EvmPreparedTransaction,
        observation: &CapabilityTransaction,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        let chain_id = u64::try_from(observation.chain_id)
            .map_err(|_| invalid("observed chain id exceeded supported range"))?;
        let unsigned = EvmUnsignedTransaction {
            chain_id,
            nonce: observation.nonce.to_string(),
            max_priority_fee_per_gas: observation.max_priority_fee_per_gas.to_string(),
            max_fee_per_gas: observation.max_fee_per_gas.to_string(),
            gas_limit: observation.gas_limit.to_string(),
            to: match observation.to {
                TxKind::Create => None,
                TxKind::Call(address) => Some(canonical_address(address)),
            },
            value: observation.value.to_string(),
            access_list: access_list_from_alloy(&observation.access_list),
            input: canonical_bytes(&observation.input),
        };
        let submission = Self {
            transaction_hash: canonical_hash(observation.transaction_hash),
            from: canonical_address(observation.from),
            unsigned,
            session: session.clone(),
        };
        submission.validate_against(prepared)?;
        Ok(submission)
    }

    /// Returns the observed transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Validates transaction lookup fields against immutable prepared authority.
    pub fn validate_against(&self, prepared: &EvmPreparedTransaction) -> Result<(), EvmStateError> {
        prepared.validate()?;
        validate_session(
            &self.session,
            prepared.intent.network_id(),
            prepared.intent.chain_id(),
        )?;
        parse_hash(&self.transaction_hash)?;
        parse_address(&self.from)?;
        self.unsigned.validate()?;
        if self.transaction_hash != prepared.expected_transaction_hash
            || self.from != prepared.intent.expected_sender
            || self.unsigned != prepared.unsigned
        {
            return Err(invalid(
                "transaction lookup did not match prepared authority",
            ));
        }
        Ok(())
    }
}

/// Public uncertainty evidence that never claims non-submission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_recovery_evidence",
    version = "1",
    schema = "mfm.evm.transaction.recovery_evidence"
)]
pub struct EvmTransactionRecoveryEvidence {
    transaction_hash: String,
    session: EvmSessionEvidence,
}

impl EvmTransactionRecoveryEvidence {
    /// Creates redacted uncertainty evidence for the exact prepared hash.
    pub fn new(
        prepared: &EvmPreparedTransaction,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        prepared.validate()?;
        let evidence = Self {
            transaction_hash: prepared.expected_transaction_hash.clone(),
            session: session.clone(),
        };
        evidence.validate_against(prepared)?;
        Ok(evidence)
    }

    /// Validates uncertainty evidence against the immutable prepared invocation.
    pub fn validate_against(&self, prepared: &EvmPreparedTransaction) -> Result<(), EvmStateError> {
        prepared.validate()?;
        if self.transaction_hash != prepared.expected_transaction_hash {
            return Err(invalid(
                "recovery evidence did not match the prepared transaction hash",
            ));
        }
        parse_hash(&self.transaction_hash)?;
        validate_session(
            &self.session,
            prepared.intent.network_id(),
            prepared.intent.chain_id(),
        )?;
        Ok(())
    }
}

/// Strict receipt execution status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_execution_status",
    version = "1",
    schema = "mfm.evm.transaction.execution_status"
)]
pub enum EvmExecutionStatus {
    /// Execution succeeded.
    Success,
    /// Execution reverted after consuming nonce and gas.
    Reverted,
}

/// Complete canonical receipt log with lossless read-only access for operation consumers.
///
/// Deterministic event identification does not require serializing internal
/// evidence or knowing its private field layout:
///
/// ```rust
/// use mfm_states_evm::{EvmTransactionLog, EvmTransactionReceipt};
///
/// fn matching_event<'a>(
///     receipt: &'a EvmTransactionReceipt,
///     emitting_address: &str,
///     signature_topic: &str,
/// ) -> Option<&'a EvmTransactionLog> {
///     receipt.logs().iter().find(|log| {
///         let complete_identity = (
///             log.address(),
///             log.topics(),
///             log.data(),
///             log.block_anchor().number(),
///             log.block_anchor().hash(),
///             log.transaction_hash(),
///             log.transaction_index(),
///             log.log_index(),
///             log.removed(),
///         );
///         let _ = complete_identity;
///         log.address() == emitting_address
///             && log.topics().first().is_some_and(|topic| topic == signature_topic)
///     })
/// }
/// ```
///
/// The read surface cannot mutate retained evidence:
///
/// ```compile_fail
/// use mfm_states_evm::EvmTransactionLog;
///
/// fn mark_removed(log: &mut EvmTransactionLog) {
///     log.removed = true;
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_log",
    version = "1",
    schema = "mfm.evm.transaction.log"
)]
pub struct EvmTransactionLog {
    address: String,
    topics: Vec<String>,
    data: String,
    block: EvmBlockAnchor,
    transaction_hash: String,
    transaction_index: String,
    log_index: String,
    removed: bool,
}

/// Strict typed transaction receipt retained as terminal external-effect evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_receipt",
    version = "1",
    schema = "mfm.evm.transaction.receipt"
)]
pub struct EvmTransactionReceipt {
    transaction_hash: String,
    transaction_index: String,
    block: EvmBlockAnchor,
    from: String,
    to: Option<String>,
    contract_address: Option<String>,
    status: EvmExecutionStatus,
    gas_used: String,
    cumulative_gas_used: String,
    logs: Vec<EvmTransactionLog>,
    session: EvmSessionEvidence,
}

impl EvmTransactionReceipt {
    /// Converts and validates one strict capability receipt.
    pub fn from_observation(
        prepared: &EvmPreparedTransaction,
        receipt: &CapabilityReceipt,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        receipt
            .validate()
            .map_err(|_| invalid("receipt log identities were incoherent"))?;
        let status = match receipt.status {
            CapabilityReceiptStatus::Success => EvmExecutionStatus::Success,
            CapabilityReceiptStatus::Reverted => EvmExecutionStatus::Reverted,
        };
        let converted = Self {
            transaction_hash: canonical_hash(receipt.transaction_hash),
            transaction_index: receipt.transaction_index.to_string(),
            block: receipt.block.clone(),
            from: canonical_address(receipt.from),
            to: receipt.to.map(canonical_address),
            contract_address: receipt.contract_address.map(canonical_address),
            status,
            gas_used: receipt.gas_used.to_string(),
            cumulative_gas_used: receipt.cumulative_gas_used.to_string(),
            logs: receipt
                .logs
                .iter()
                .map(|log| EvmTransactionLog {
                    address: canonical_address(log.address),
                    topics: log.topics.iter().copied().map(canonical_hash).collect(),
                    data: canonical_bytes(&log.data),
                    block: log.block.clone(),
                    transaction_hash: canonical_hash(log.transaction_hash),
                    transaction_index: log.transaction_index.to_string(),
                    log_index: log.log_index.to_string(),
                    removed: log.removed,
                })
                .collect(),
            session: session.clone(),
        };
        converted.validate_against(prepared)?;
        Ok(converted)
    }

    /// Returns the expected transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the canonical transaction index within the block.
    pub fn transaction_index(&self) -> &str {
        &self.transaction_index
    }

    /// Returns the canonical sender address.
    pub fn sender(&self) -> &str {
        &self.from
    }

    /// Returns the canonical destination address, or none for direct creation.
    pub fn destination(&self) -> Option<&str> {
        self.to.as_deref()
    }

    /// Returns the strict execution status.
    pub const fn status(&self) -> EvmExecutionStatus {
        self.status
    }

    /// Returns the created contract address, when valid for a successful direct creation.
    pub fn contract_address(&self) -> Option<&str> {
        self.contract_address.as_deref()
    }

    /// Returns the canonical gas used by this transaction.
    pub fn gas_used(&self) -> &str {
        &self.gas_used
    }

    /// Returns the canonical cumulative gas used in the block.
    pub fn cumulative_gas_used(&self) -> &str {
        &self.cumulative_gas_used
    }

    /// Returns complete canonical logs.
    pub fn logs(&self) -> &[EvmTransactionLog] {
        &self.logs
    }

    /// Returns the inclusion anchor.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block
    }

    /// Returns the inclusion block number as U256.
    pub fn block_number_quantity(&self) -> Result<U256, EvmStateError> {
        block_anchor_number(&self.block)
    }

    /// Returns the inclusion block hash.
    pub fn block_hash_value(&self) -> Result<B256, EvmStateError> {
        block_anchor_hash(&self.block)
    }

    /// Returns the checked source-bound session provenance for this observation.
    pub const fn session_evidence(&self) -> &EvmSessionEvidence {
        &self.session
    }

    /// Returns whether another observation has identical on-chain receipt fields.
    pub fn matches_chain_receipt(&self, other: &Self) -> bool {
        self.same_chain_receipt(other)
    }

    /// Validates receipt, action, creation-address, and log relations.
    pub fn validate_against(&self, prepared: &EvmPreparedTransaction) -> Result<(), EvmStateError> {
        prepared.validate()?;
        validate_session(
            &self.session,
            prepared.intent.network_id(),
            prepared.intent.chain_id(),
        )?;
        parse_hash(&self.transaction_hash)?;
        parse_quantity(&self.transaction_index)?;
        validate_block_anchor(&self.block)?;
        parse_address(&self.from)?;
        if let Some(to) = &self.to {
            parse_address(to)?;
        }
        if let Some(address) = &self.contract_address {
            parse_address(address)?;
        }
        parse_quantity(&self.gas_used)?;
        parse_quantity(&self.cumulative_gas_used)?;
        if parse_quantity(&self.cumulative_gas_used)? < parse_quantity(&self.gas_used)? {
            return Err(invalid("receipt cumulative gas was below transaction gas"));
        }
        if self.transaction_hash != prepared.expected_transaction_hash
            || self.from != prepared.intent.expected_sender
        {
            return Err(invalid("receipt identity did not match prepared authority"));
        }
        let expected_to = prepared
            .intent
            .action
            .call_destination()?
            .map(canonical_address);
        if self.to != expected_to {
            return Err(invalid(
                "receipt destination did not match transaction action",
            ));
        }
        match (prepared.intent.action.action_kind(), self.status) {
            (EvmTransactionActionKind::Create, EvmExecutionStatus::Success) => {
                if self.contract_address.as_deref() != prepared.expected_create_address.as_deref() {
                    return Err(invalid(
                        "successful CREATE receipt did not contain the derived address",
                    ));
                }
            }
            _ if self.contract_address.is_some() => {
                return Err(invalid(
                    "receipt contract address was forbidden for this action or status",
                ));
            }
            _ => {}
        }
        if matches!(self.status, EvmExecutionStatus::Reverted) && !self.logs.is_empty() {
            return Err(invalid("reverted receipt contained logs"));
        }
        if self.logs.len() > MAX_RECEIPT_LOGS {
            return Err(invalid("receipt contained too many logs"));
        }
        let mut previous_log_index = None;
        let mut total_log_data_bytes = 0usize;
        for log in &self.logs {
            log.validate_against(self)?;
            let log_index = parse_quantity(&log.log_index)?;
            if previous_log_index.is_some_and(|previous| previous >= log_index) {
                return Err(invalid(
                    "receipt log indexes were duplicate or out of order",
                ));
            }
            previous_log_index = Some(log_index);
            total_log_data_bytes = total_log_data_bytes
                .checked_add(parse_bytes(&log.data, EVM_TRANSACTION_DATA_MAX_BYTES)?.len())
                .ok_or_else(|| invalid("receipt log data size overflowed"))?;
            if total_log_data_bytes > MAX_RECEIPT_LOG_DATA_BYTES {
                return Err(invalid("receipt log data exceeded the retained bound"));
            }
        }
        Ok(())
    }

    /// Validates receipt fields against both prepared authority and the retained transaction
    /// observation.
    pub fn validate_with_submission(
        &self,
        prepared: &EvmPreparedTransaction,
        submission: &EvmTransactionSubmission,
    ) -> Result<(), EvmStateError> {
        self.validate_against(prepared)?;
        submission.validate_against(prepared)?;
        if parse_quantity(&self.gas_used)? > parse_quantity(&prepared.unsigned.gas_limit)? {
            return Err(invalid("receipt gas used exceeded the prepared gas limit"));
        }
        Ok(())
    }

    fn same_chain_receipt(&self, other: &Self) -> bool {
        self.transaction_hash == other.transaction_hash
            && self.transaction_index == other.transaction_index
            && self.block == other.block
            && self.from == other.from
            && self.to == other.to
            && self.contract_address == other.contract_address
            && self.status == other.status
            && self.gas_used == other.gas_used
            && self.cumulative_gas_used == other.cumulative_gas_used
            && self.logs == other.logs
    }
}

impl EvmTransactionLog {
    /// Returns the canonical emitting address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns ordered canonical topics without permitting mutation.
    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    /// Returns canonical hexadecimal encoding of the arbitrary log-data bytes.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Returns the exact inclusion block anchor.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block
    }

    /// Returns the canonical enclosing transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the canonical transaction index within the block.
    pub fn transaction_index(&self) -> &str {
        &self.transaction_index
    }

    /// Returns the canonical log index within the block.
    pub fn log_index(&self) -> &str {
        &self.log_index
    }

    /// Returns whether the provider marked this log removed.
    pub const fn removed(&self) -> bool {
        self.removed
    }

    fn validate_against(&self, receipt: &EvmTransactionReceipt) -> Result<(), EvmStateError> {
        parse_address(&self.address)?;
        if self.topics.len() > MAX_LOG_TOPICS {
            return Err(invalid("receipt log contained too many topics"));
        }
        for topic in &self.topics {
            parse_hash(topic)?;
        }
        parse_bytes(&self.data, EVM_TRANSACTION_DATA_MAX_BYTES)?;
        validate_block_anchor(&self.block)?;
        parse_hash(&self.transaction_hash)?;
        parse_quantity(&self.transaction_index)?;
        parse_quantity(&self.log_index)?;
        if self.removed
            || self.block != receipt.block
            || self.transaction_hash != receipt.transaction_hash
            || self.transaction_index != receipt.transaction_index
        {
            return Err(invalid("receipt log identity was incoherent"));
        }
        Ok(())
    }
}

/// Finality evidence from a fresh receipt, its canonical block, and a checked head.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_confirmation",
    version = "1",
    schema = "mfm.evm.transaction.confirmation"
)]
pub struct EvmTransactionConfirmation {
    fresh_receipt: EvmTransactionReceipt,
    canonical_block: EvmBlockAnchor,
    head: EvmBlockAnchor,
    required_depth: u64,
    confirmations: String,
    session: EvmSessionEvidence,
}

impl EvmTransactionConfirmation {
    /// Builds checked finality evidence from fresh live observations.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prepared: &EvmPreparedTransaction,
        retained_receipt: &EvmTransactionReceipt,
        fresh_receipt: EvmTransactionReceipt,
        canonical_block: &EvmBlockAnchor,
        head: &EvmBlockAnchor,
        required_depth: u64,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        let receipt_number = retained_receipt.block_number_quantity()?;
        let confirmations = head
            .number_quantity()
            .map_err(|_| invalid("confirmation head block number was invalid"))?
            .checked_sub(receipt_number)
            .and_then(|distance| distance.checked_add(U256::from(1)))
            .ok_or_else(|| invalid("confirmation head preceded receipt block"))?;
        let confirmation = Self {
            fresh_receipt,
            canonical_block: canonical_block.clone(),
            head: head.clone(),
            required_depth,
            confirmations: confirmations.to_string(),
            session: session.clone(),
        };
        confirmation.validate_against(prepared, retained_receipt)?;
        Ok(confirmation)
    }

    /// Returns the checked head that established the required depth.
    pub const fn head(&self) -> &EvmBlockAnchor {
        &self.head
    }

    /// Returns the certified required depth.
    pub const fn required_depth(&self) -> u64 {
        self.required_depth
    }

    /// Validates fresh receipt, canonical block, and depth relations without live IO.
    pub fn validate_against(
        &self,
        prepared: &EvmPreparedTransaction,
        retained_receipt: &EvmTransactionReceipt,
    ) -> Result<(), EvmStateError> {
        retained_receipt.validate_against(prepared)?;
        self.fresh_receipt.validate_against(prepared)?;
        validate_session(
            &self.session,
            prepared.intent.network_id(),
            prepared.intent.chain_id(),
        )?;
        validate_block_anchor(&self.canonical_block)?;
        validate_block_anchor(&self.head)?;
        if self.required_depth == 0 {
            return Err(invalid("confirmation depth must be positive"));
        }
        if !retained_receipt.same_chain_receipt(&self.fresh_receipt) {
            return Err(invalid(
                "fresh receipt moved or changed before confirmation",
            ));
        }
        if self.fresh_receipt.session != self.session {
            return Err(invalid(
                "fresh receipt provenance differed from confirmation session",
            ));
        }
        if &self.canonical_block != retained_receipt.block_anchor() {
            return Err(invalid("canonical block did not match receipt anchor"));
        }
        let receipt_number = retained_receipt.block_number_quantity()?;
        let head_number = block_anchor_number(&self.head)?;
        let recomputed = head_number
            .checked_sub(receipt_number)
            .and_then(|distance| distance.checked_add(U256::from(1)))
            .ok_or_else(|| invalid("confirmation head preceded receipt block"))?;
        if recomputed != parse_quantity(&self.confirmations)?
            || recomputed < U256::from(self.required_depth)
        {
            return Err(invalid(
                "confirmation depth was insufficient or inconsistent",
            ));
        }
        Ok(())
    }
}

/// Successful transaction-specific terminal result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_success",
    version = "1",
    schema = "mfm.evm.transaction.success"
)]
pub enum EvmTransactionSuccess {
    /// A direct creation yielded the sender/nonce-derived address.
    Created {
        /// Created contract address.
        address: String,
    },
    /// An ordinary call targeted this address.
    Called {
        /// Called address.
        address: String,
    },
}

/// Explicit terminal execution result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_result",
    version = "1",
    schema = "mfm.evm.transaction.result"
)]
pub enum EvmTransactionResult {
    /// Execution succeeded.
    Succeeded {
        /// Action-specific success value.
        result: EvmTransactionSuccess,
    },
    /// Execution reverted after consuming nonce and gas.
    Reverted {
        /// Reverted action category.
        action: EvmTransactionActionKind,
    },
}

/// Terminal one-transaction output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction_outcome",
    version = "1",
    schema = "mfm.evm.transaction.outcome"
)]
pub struct EvmTransactionOutcome {
    transaction_hash: String,
    receipt: EvmTransactionReceipt,
    confirmation_head: Option<EvmBlockAnchor>,
    result: EvmTransactionResult,
}

impl EvmTransactionOutcome {
    /// Returns the expected transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    /// Returns the complete strict receipt.
    pub const fn receipt(&self) -> &EvmTransactionReceipt {
        &self.receipt
    }

    /// Returns the finality head when the node required confirmation depth.
    pub const fn confirmation_head(&self) -> Option<&EvmBlockAnchor> {
        self.confirmation_head.as_ref()
    }

    /// Returns the explicit success or revert classification.
    pub const fn result(&self) -> &EvmTransactionResult {
        &self.result
    }

    /// Reduces retained public transaction evidence into the one terminal outcome.
    pub fn from_evidence(
        prepared: &EvmPreparedTransaction,
        submission: &EvmTransactionSubmission,
        receipt: &EvmTransactionReceipt,
        confirmation: Option<&EvmTransactionConfirmation>,
    ) -> Result<Self, EvmStateError> {
        receipt.validate_with_submission(prepared, submission)?;
        if let Some(confirmation) = confirmation {
            confirmation.validate_against(prepared, receipt)?;
        }
        let result = match receipt.status {
            EvmExecutionStatus::Reverted => EvmTransactionResult::Reverted {
                action: prepared.intent.action.action_kind(),
            },
            EvmExecutionStatus::Success => match &prepared.intent.action {
                EvmTransactionAction::Create { .. } => EvmTransactionResult::Succeeded {
                    result: EvmTransactionSuccess::Created {
                        address: receipt
                            .contract_address
                            .clone()
                            .ok_or_else(|| invalid("successful CREATE address was missing"))?,
                    },
                },
                EvmTransactionAction::Call { to, .. } => EvmTransactionResult::Succeeded {
                    result: EvmTransactionSuccess::Called {
                        address: to.clone(),
                    },
                },
            },
        };
        Ok(Self {
            transaction_hash: prepared.expected_transaction_hash.clone(),
            receipt: receipt.clone(),
            confirmation_head: confirmation.map(|value| value.head.clone()),
            result,
        })
    }
}

/// Reusable one-transaction EVM side-effect state.
pub struct SubmitEvmTransactionState {
    config: EvmTransactionConfig,
}

impl StateSpec for SubmitEvmTransactionState {
    type Config = EvmTransactionConfig;
    type Context = NoContext;
    type Input = EvmTransactionAction;
    type Output = EvmTransactionOutcome;
    type Effect = ApplySideEffect;
    type Caps = (
        mfm_evm_capabilities::EvmTransactionCapability,
        SigningCapability,
    );

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("transaction.submit")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("transaction.submit")
    }

    fn name() -> &'static str {
        "mfm.evm.transaction.submit"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for SubmitEvmTransactionState {
    type Intent = EvmTransactionIntent;
    type IdempotencyInput = EvmTransactionIntent;
    type PreparedInvocation = EvmPreparedTransaction;
    type Submission = EvmTransactionSubmission;
    type RecoveryEvidence = EvmTransactionRecoveryEvidence;
    type Receipt = EvmTransactionReceipt;
    type Confirmation = EvmTransactionConfirmation;

    fn intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<SideEffectIntent<Self::Intent, Self::IdempotencyInput>> {
        let intent = EvmTransactionIntent::from_config(&self.config, input)?;
        Ok(SideEffectIntent::new(intent.clone(), intent))
    }

    fn output_from_receipt(
        &self,
        input: &Self::Input,
        prepared: &Self::PreparedInvocation,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let authored = self.intent(input, context)?;
        if authored.intent() != prepared.intent() {
            return Err(invalid("prepared intent differed from authored intent").into());
        }
        EvmTransactionOutcome::from_evidence(prepared, submission, receipt, None)
            .map_err(Into::into)
    }

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        prepared: &Self::PreparedInvocation,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let authored = self.intent(input, context)?;
        if authored.intent() != prepared.intent() {
            return Err(invalid("prepared intent differed from authored intent").into());
        }
        EvmTransactionOutcome::from_evidence(prepared, submission, receipt, Some(confirmation))
            .map_err(Into::into)
    }
}

fn validate_access_list(entries: &[EvmAccessListEntry]) -> Result<(), EvmStateError> {
    if entries.len() > MAX_ACCESS_LIST_ENTRIES {
        return Err(invalid("access list contained too many addresses"));
    }
    let mut storage_key_count = 0usize;
    for entry in entries {
        entry.validate()?;
        storage_key_count = storage_key_count
            .checked_add(entry.storage_keys.len())
            .ok_or_else(|| invalid("access list size overflowed"))?;
    }
    if storage_key_count > MAX_ACCESS_LIST_STORAGE_KEYS {
        return Err(invalid("access list contained too many storage keys"));
    }
    Ok(())
}

fn access_list_from_alloy(access_list: &AccessList) -> Vec<EvmAccessListEntry> {
    access_list
        .0
        .iter()
        .map(|entry| EvmAccessListEntry::new(entry.address, entry.storage_keys.clone()))
        .collect()
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
