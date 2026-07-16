#![warn(missing_docs)]
//! Reusable EVM contract action configs.
//!
//! This module owns deterministic, non-secret action config types for
//! context-bound deploy, configure, and validate workflows. It does not
//! carry runtime endpoints, provider lookup details, signer material locations,
//! artifact store handles, machine ids, or workflow topology.
//!
//! ```rust
//! use mfm_state_evm_contracts::{DeployAction, EvmTransactionStyle};
//!
//! let deploy: DeployAction = serde_json::from_value(serde_json::json!({
//!     "signer": {
//!         "signer_ref": "deployer",
//!         "expected_signer_address": "0x000000000000000000000000000000000000dead"
//!     }
//! }))?;
//!
//! assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Eip1559);
//! # Ok::<(), serde_json::Error>(())
//! ```

use alloy_primitives::Address;
use mfm_evm_contract_model::{
    AbiArgumentValue, ContractCallConfig, EventAssertionConfig, EvmContractScalarError,
    ReadAssertionConfig, WeiAmount,
};
use mfm_evm_core::encoding::address_hex_lower;
use mfm_evm_core::tx::parse_address;
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_signing::SignerRef;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

fn default_poll_interval_ms() -> u64 {
    500
}

fn default_max_receipt_polls() -> u64 {
    120
}

/// Maximum delay accepted between transaction receipt polls.
pub const MAX_RECEIPT_POLL_INTERVAL_MS: u64 = 10_000;
/// Maximum receipt poll attempts accepted for one mutation phase.
pub const MAX_RECEIPT_POLLS: u64 = 600;
/// Maximum total receipt polling wait accepted for one mutation phase.
pub const MAX_RECEIPT_TOTAL_WAIT_MS: u64 = 600_000;

fn nonzero_u64(value: u64, field: &'static str) -> Result<NonZeroU64, String> {
    NonZeroU64::new(value).ok_or_else(|| format!("{field} must be non-zero"))
}

fn optional_nonzero_u64(
    value: Option<u64>,
    field: &'static str,
) -> Result<Option<NonZeroU64>, String> {
    value.map(|value| nonzero_u64(value, field)).transpose()
}

fn wei_amount(value: String, field: &'static str) -> Result<WeiAmount, String> {
    WeiAmount::new(value).map_err(|error| match error {
        EvmContractScalarError::InvalidQuantity { message, .. } => {
            format!("{field} must be a valid EVM quantity: {message}")
        }
        EvmContractScalarError::InvalidIdentity { message, .. } => {
            format!("{field} must be a valid typed identity: {message}")
        }
        EvmContractScalarError::InvalidString { message, .. } => {
            format!("{field} must be a valid checked string: {message}")
        }
        EvmContractScalarError::Empty { .. } => format!("{field} must be non-empty"),
    })
}

fn optional_wei_amount(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<WeiAmount>, String> {
    value.map(|value| wei_amount(value, field)).transpose()
}

/// Signer intent for EVM contract transaction phases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "signer-intent",
    schema = "mfm.evm.contract.config.signer_intent"
)]
pub struct EvmSignerIntent {
    signer_ref: String,
    expected_signer_address: String,
}

impl EvmSignerIntent {
    /// Creates signer intent from a process-local signer reference and expected EVM address.
    pub fn new(signer_ref: SignerRef, expected_signer_address: Address) -> Self {
        Self {
            signer_ref: signer_ref.to_string(),
            expected_signer_address: address_hex_lower(&expected_signer_address),
        }
    }

    /// Returns the signer reference string.
    pub fn signer_ref_str(&self) -> &str {
        &self.signer_ref
    }

    /// Parses and returns the typed signer reference.
    pub fn signer_ref(&self) -> Result<SignerRef, String> {
        SignerRef::new(&self.signer_ref).map_err(|error| error.to_string())
    }

    /// Returns the expected signer address string.
    pub fn expected_signer_address_str(&self) -> &str {
        &self.expected_signer_address
    }

    /// Parses and returns the expected signer address.
    pub fn expected_signer_address(&self) -> Result<Address, String> {
        parse_address(&self.expected_signer_address, "expected_signer_address")
            .map_err(|error| error.message)
    }
}

impl<'de> Deserialize<'de> for EvmSignerIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawSignerIntent {
            signer_ref: String,
            expected_signer_address: String,
        }

        let raw = RawSignerIntent::deserialize(deserializer)?;
        let signer_ref = SignerRef::new(&raw.signer_ref).map_err(de::Error::custom)?;
        let expected_signer_address =
            parse_address(&raw.expected_signer_address, "expected_signer_address")
                .map_err(|error| de::Error::custom(error.message))?;
        Ok(Self::new(signer_ref, expected_signer_address))
    }
}

/// EVM transaction fee style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-style",
    schema = "mfm.evm.contract.config.transaction_style"
)]
pub enum EvmTransactionStyle {
    /// EIP-1559 dynamic fee transaction style.
    #[default]
    Eip1559,
    /// Legacy gas-price transaction style.
    Legacy,
}

/// Transaction fee and gas policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[serde(tag = "style", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-policy",
    schema = "mfm.evm.contract.config.transaction_policy"
)]
pub enum EvmTransactionPolicy {
    /// EIP-1559 dynamic fee transaction policy.
    Eip1559 {
        /// Optional explicit gas limit.
        gas_limit: Option<NonZeroU64>,
        /// Optional maximum fee per gas in wei.
        max_fee_per_gas: Option<WeiAmount>,
        /// Optional maximum priority fee per gas in wei.
        max_priority_fee_per_gas: Option<WeiAmount>,
    },
    /// Legacy gas-price transaction policy.
    Legacy {
        /// Optional explicit gas limit.
        gas_limit: Option<NonZeroU64>,
        /// Optional gas price in wei.
        gas_price: Option<WeiAmount>,
    },
}

impl Default for EvmTransactionPolicy {
    fn default() -> Self {
        Self::Eip1559 {
            gas_limit: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        }
    }
}

impl EvmTransactionPolicy {
    /// Creates a validated transaction policy.
    pub fn new(
        style: EvmTransactionStyle,
        gas_limit: Option<u64>,
        max_fee_per_gas: Option<String>,
        max_priority_fee_per_gas: Option<String>,
        gas_price: Option<String>,
    ) -> Result<Self, String> {
        match style {
            EvmTransactionStyle::Eip1559 => {
                if gas_price.is_some() {
                    return Err("gas_price is only valid for legacy transactions".to_string());
                }
                Ok(Self::Eip1559 {
                    gas_limit: optional_nonzero_u64(gas_limit, "gas_limit")?,
                    max_fee_per_gas: optional_wei_amount(max_fee_per_gas, "max_fee_per_gas")?,
                    max_priority_fee_per_gas: optional_wei_amount(
                        max_priority_fee_per_gas,
                        "max_priority_fee_per_gas",
                    )?,
                })
            }
            EvmTransactionStyle::Legacy => {
                if max_fee_per_gas.is_some() || max_priority_fee_per_gas.is_some() {
                    return Err(
                        "EIP-1559 fee fields are not valid for legacy transactions".to_string()
                    );
                }
                Ok(Self::Legacy {
                    gas_limit: optional_nonzero_u64(gas_limit, "gas_limit")?,
                    gas_price: optional_wei_amount(gas_price, "gas_price")?,
                })
            }
        }
    }

    /// Returns the configured transaction style.
    pub const fn style(&self) -> EvmTransactionStyle {
        match self {
            Self::Eip1559 { .. } => EvmTransactionStyle::Eip1559,
            Self::Legacy { .. } => EvmTransactionStyle::Legacy,
        }
    }

    /// Returns the optional gas limit.
    pub fn gas_limit(&self) -> Option<u64> {
        match self {
            Self::Eip1559 { gas_limit, .. } | Self::Legacy { gas_limit, .. } => {
                gas_limit.map(NonZeroU64::get)
            }
        }
    }

    /// Returns the optional EIP-1559 maximum fee per gas.
    pub fn max_fee_per_gas(&self) -> Option<&str> {
        match self {
            Self::Eip1559 {
                max_fee_per_gas, ..
            } => max_fee_per_gas.as_ref().map(WeiAmount::as_str),
            Self::Legacy { .. } => None,
        }
    }

    /// Returns the optional EIP-1559 priority fee per gas.
    pub fn max_priority_fee_per_gas(&self) -> Option<&str> {
        match self {
            Self::Eip1559 {
                max_priority_fee_per_gas,
                ..
            } => max_priority_fee_per_gas.as_ref().map(WeiAmount::as_str),
            Self::Legacy { .. } => None,
        }
    }

    /// Returns the optional legacy gas price.
    pub fn gas_price(&self) -> Option<&str> {
        match self {
            Self::Legacy { gas_price, .. } => gas_price.as_ref().map(WeiAmount::as_str),
            Self::Eip1559 { .. } => None,
        }
    }
}

impl<'de> Deserialize<'de> for EvmTransactionPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawTransactionPolicy {
            #[serde(default)]
            style: EvmTransactionStyle,
            #[serde(default)]
            gas_limit: Option<u64>,
            #[serde(default)]
            max_fee_per_gas: Option<String>,
            #[serde(default)]
            max_priority_fee_per_gas: Option<String>,
            #[serde(default)]
            gas_price: Option<String>,
        }

        let raw = RawTransactionPolicy::deserialize(deserializer)?;
        Self::new(
            raw.style,
            raw.gas_limit,
            raw.max_fee_per_gas,
            raw.max_priority_fee_per_gas,
            raw.gas_price,
        )
        .map_err(de::Error::custom)
    }
}

/// Receipt polling and retry policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "receipt-policy",
    schema = "mfm.evm.contract.config.receipt_policy"
)]
pub struct ReceiptRetryPolicy {
    poll_interval_ms: NonZeroU64,
    max_receipt_polls: NonZeroU64,
}

impl Default for ReceiptRetryPolicy {
    fn default() -> Self {
        Self {
            poll_interval_ms: nonzero_u64(default_poll_interval_ms(), "poll_interval_ms")
                .expect("default poll interval is non-zero"),
            max_receipt_polls: nonzero_u64(default_max_receipt_polls(), "max_receipt_polls")
                .expect("default receipt polls is non-zero"),
        }
    }
}

impl ReceiptRetryPolicy {
    /// Creates a validated receipt retry policy.
    pub fn new(poll_interval_ms: u64, max_receipt_polls: u64) -> Result<Self, String> {
        let poll_interval_ms = nonzero_u64(poll_interval_ms, "poll_interval_ms")?;
        let max_receipt_polls = nonzero_u64(max_receipt_polls, "max_receipt_polls")?;

        if poll_interval_ms.get() > MAX_RECEIPT_POLL_INTERVAL_MS {
            return Err(format!(
                "poll_interval_ms must be <= {MAX_RECEIPT_POLL_INTERVAL_MS}"
            ));
        }
        if max_receipt_polls.get() > MAX_RECEIPT_POLLS {
            return Err(format!("max_receipt_polls must be <= {MAX_RECEIPT_POLLS}"));
        }
        let total_wait_ms = poll_interval_ms
            .get()
            .checked_mul(max_receipt_polls.get().saturating_sub(1))
            .ok_or_else(|| "receipt polling wait budget overflowed".to_string())?;
        if total_wait_ms > MAX_RECEIPT_TOTAL_WAIT_MS {
            return Err(format!(
                "receipt polling wait budget must be <= {MAX_RECEIPT_TOTAL_WAIT_MS} ms"
            ));
        }
        Ok(Self {
            poll_interval_ms,
            max_receipt_polls,
        })
    }

    /// Returns delay between receipt polls in milliseconds.
    pub fn poll_interval_ms(&self) -> u64 {
        self.poll_interval_ms.get()
    }

    /// Returns maximum receipt poll attempts.
    pub fn max_receipt_polls(&self) -> u64 {
        self.max_receipt_polls.get()
    }
}

impl<'de> Deserialize<'de> for ReceiptRetryPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawReceiptRetryPolicy {
            #[serde(default = "default_poll_interval_ms")]
            poll_interval_ms: u64,
            #[serde(default = "default_max_receipt_polls")]
            max_receipt_polls: u64,
        }

        let raw = RawReceiptRetryPolicy::deserialize(deserializer)?;
        Self::new(raw.poll_interval_ms, raw.max_receipt_polls).map_err(de::Error::custom)
    }
}

/// Deploy action executed inside a certified EVM contract context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-action",
    schema = "mfm.evm.contract.config.deploy_action"
)]
pub struct DeployAction {
    signer: EvmSignerIntent,
    constructor_args: Vec<AbiArgumentValue>,
    value_wei: Option<WeiAmount>,
    transaction: EvmTransactionPolicy,
    receipt: ReceiptRetryPolicy,
}

impl DeployAction {
    /// Returns signer intent.
    pub const fn signer(&self) -> &EvmSignerIntent {
        &self.signer
    }

    /// Returns constructor arguments.
    pub fn constructor_args(&self) -> &[AbiArgumentValue] {
        &self.constructor_args
    }

    /// Returns optional deployment value in wei.
    pub fn value_wei(&self) -> Option<&str> {
        self.value_wei.as_ref().map(WeiAmount::as_str)
    }

    /// Returns transaction policy.
    pub const fn transaction(&self) -> &EvmTransactionPolicy {
        &self.transaction
    }

    /// Returns receipt retry policy.
    pub const fn receipt(&self) -> &ReceiptRetryPolicy {
        &self.receipt
    }
}

impl<'de> Deserialize<'de> for DeployAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDeployAction {
            signer: EvmSignerIntent,
            #[serde(default)]
            constructor_args: Vec<AbiArgumentValue>,
            #[serde(default)]
            value_wei: Option<String>,
            #[serde(default)]
            transaction: EvmTransactionPolicy,
            #[serde(default)]
            receipt: ReceiptRetryPolicy,
        }

        let raw = RawDeployAction::deserialize(deserializer)?;
        Ok(Self {
            signer: raw.signer,
            constructor_args: raw.constructor_args,
            value_wei: optional_wei_amount(raw.value_wei, "value_wei")
                .map_err(de::Error::custom)?,
            transaction: raw.transaction,
            receipt: raw.receipt,
        })
    }
}

/// Configure action executed inside a certified EVM contract context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-action",
    schema = "mfm.evm.contract.config.configure_action"
)]
pub struct ConfigureAction {
    signer: EvmSignerIntent,
    calls: Vec<ContractCallConfig>,
    transaction: EvmTransactionPolicy,
    receipt: ReceiptRetryPolicy,
}

impl ConfigureAction {
    /// Returns signer intent.
    pub const fn signer(&self) -> &EvmSignerIntent {
        &self.signer
    }

    /// Returns configured contract calls.
    pub fn calls(&self) -> &[ContractCallConfig] {
        &self.calls
    }

    /// Returns transaction policy.
    pub const fn transaction(&self) -> &EvmTransactionPolicy {
        &self.transaction
    }

    /// Returns receipt retry policy.
    pub const fn receipt(&self) -> &ReceiptRetryPolicy {
        &self.receipt
    }
}

impl<'de> Deserialize<'de> for ConfigureAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawConfigureAction {
            signer: EvmSignerIntent,
            #[serde(default)]
            calls: Vec<ContractCallConfig>,
            #[serde(default)]
            transaction: EvmTransactionPolicy,
            #[serde(default)]
            receipt: ReceiptRetryPolicy,
        }

        let raw = RawConfigureAction::deserialize(deserializer)?;
        Ok(Self {
            signer: raw.signer,
            calls: raw.calls,
            transaction: raw.transaction,
            receipt: raw.receipt,
        })
    }
}

/// Validate action executed inside a certified EVM contract context.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validate-action",
    schema = "mfm.evm.contract.config.validate_action"
)]
pub struct ValidateAction {
    #[serde(default)]
    read_assertions: Vec<ReadAssertionConfig>,
    #[serde(default)]
    event_assertions: Vec<EventAssertionConfig>,
}

impl ValidateAction {
    /// Returns read assertions evaluated with EVM calls.
    pub fn read_assertions(&self) -> &[ReadAssertionConfig] {
        &self.read_assertions
    }

    /// Returns event assertions evaluated with EVM log queries.
    pub fn event_assertions(&self) -> &[EventAssertionConfig] {
        &self.event_assertions
    }
}
