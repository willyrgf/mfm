#![warn(missing_docs)]
//! Reusable EVM contract lifecycle phase configs.
//!
//! This crate owns deterministic, non-secret phase config types for deploy,
//! configure, and validate phases. It does not carry runtime endpoints, provider
//! lookup details, signer material locations, artifact store handles, machine ids,
//! or workflow topology.
//!
//! ```rust
//! use mfm_evm_contract_config::{DeployPhaseConfig, EvmTransactionStyle};
//!
//! let deploy: DeployPhaseConfig = serde_json::from_value(serde_json::json!({
//!     "network": {
//!         "network_id": "ethereum-mainnet",
//!         "expected_chain_id": 1
//!     },
//!     "signer": {
//!         "signer_ref": "deployer",
//!         "expected_signer_address": "0x000000000000000000000000000000000000dead"
//!     }
//! }))?;
//!
//! assert_eq!(deploy.network().expected_chain_id(), 1);
//! assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Eip1559);
//! # Ok::<(), serde_json::Error>(())
//! ```

use alloy_primitives::Address;
use mfm_evm_contract_model::{
    AbiArgumentValue, ContractArtifactConfig, ContractCallConfig, EventAssertionConfig,
    ReadAssertionConfig,
};
use mfm_evm_core::encoding::address_hex_lower;
use mfm_evm_core::tx::{parse_address, parse_u128_quantity};
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_signing::SignerRef;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

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

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must be non-empty"))
    } else {
        Ok(())
    }
}

fn validate_quantity(value: &Option<String>, field: &'static str) -> Result<(), String> {
    if let Some(value) = value {
        parse_u128_quantity(value, field).map_err(|error| error.message)?;
    }
    Ok(())
}

/// Semantic EVM network intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "network-intent",
    schema = "mfm.evm.contract.config.network_intent"
)]
pub struct EvmNetworkIntent {
    network_id: String,
    expected_chain_id: u64,
}

impl EvmNetworkIntent {
    /// Creates validated semantic network intent.
    pub fn new(network_id: impl AsRef<str>, expected_chain_id: u64) -> Result<Self, String> {
        validate_nonempty(network_id.as_ref(), "network_id")?;
        if expected_chain_id == 0 {
            return Err("expected_chain_id must be non-zero".to_string());
        }
        Ok(Self {
            network_id: network_id.as_ref().to_owned(),
            expected_chain_id,
        })
    }

    /// Returns the stable semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the expected EVM chain id.
    pub const fn expected_chain_id(&self) -> u64 {
        self.expected_chain_id
    }
}

impl<'de> Deserialize<'de> for EvmNetworkIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawNetworkIntent {
            network_id: String,
            expected_chain_id: u64,
        }

        let raw = RawNetworkIntent::deserialize(deserializer)?;
        Self::new(raw.network_id, raw.expected_chain_id).map_err(de::Error::custom)
    }
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
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-policy",
    schema = "mfm.evm.contract.config.transaction_policy"
)]
pub struct EvmTransactionPolicy {
    style: EvmTransactionStyle,
    gas_limit: Option<u64>,
    max_fee_per_gas: Option<String>,
    max_priority_fee_per_gas: Option<String>,
    gas_price: Option<String>,
}

impl Default for EvmTransactionPolicy {
    fn default() -> Self {
        Self {
            style: EvmTransactionStyle::Eip1559,
            gas_limit: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas_price: None,
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
        let policy = Self {
            style,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas_price,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Returns the configured transaction style.
    pub const fn style(&self) -> EvmTransactionStyle {
        self.style
    }

    /// Returns the optional gas limit.
    pub const fn gas_limit(&self) -> Option<u64> {
        self.gas_limit
    }

    /// Returns the optional EIP-1559 maximum fee per gas.
    pub fn max_fee_per_gas(&self) -> Option<&str> {
        self.max_fee_per_gas.as_deref()
    }

    /// Returns the optional EIP-1559 priority fee per gas.
    pub fn max_priority_fee_per_gas(&self) -> Option<&str> {
        self.max_priority_fee_per_gas.as_deref()
    }

    /// Returns the optional legacy gas price.
    pub fn gas_price(&self) -> Option<&str> {
        self.gas_price.as_deref()
    }

    fn validate(&self) -> Result<(), String> {
        if self.gas_limit == Some(0) {
            return Err("gas_limit must be non-zero when provided".to_string());
        }
        validate_quantity(&self.max_fee_per_gas, "max_fee_per_gas")?;
        validate_quantity(&self.max_priority_fee_per_gas, "max_priority_fee_per_gas")?;
        validate_quantity(&self.gas_price, "gas_price")?;

        match self.style {
            EvmTransactionStyle::Eip1559 if self.gas_price.is_some() => {
                Err("gas_price is only valid for legacy transactions".to_string())
            }
            EvmTransactionStyle::Legacy
                if self.max_fee_per_gas.is_some() || self.max_priority_fee_per_gas.is_some() =>
            {
                Err("EIP-1559 fee fields are not valid for legacy transactions".to_string())
            }
            _ => Ok(()),
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
    poll_interval_ms: u64,
    max_receipt_polls: u64,
}

impl Default for ReceiptRetryPolicy {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval_ms(),
            max_receipt_polls: default_max_receipt_polls(),
        }
    }
}

impl ReceiptRetryPolicy {
    /// Creates a validated receipt retry policy.
    pub fn new(poll_interval_ms: u64, max_receipt_polls: u64) -> Result<Self, String> {
        if poll_interval_ms == 0 {
            return Err("poll_interval_ms must be non-zero".to_string());
        }
        if poll_interval_ms > MAX_RECEIPT_POLL_INTERVAL_MS {
            return Err(format!(
                "poll_interval_ms must be <= {MAX_RECEIPT_POLL_INTERVAL_MS}"
            ));
        }
        if max_receipt_polls == 0 {
            return Err("max_receipt_polls must be non-zero".to_string());
        }
        if max_receipt_polls > MAX_RECEIPT_POLLS {
            return Err(format!("max_receipt_polls must be <= {MAX_RECEIPT_POLLS}"));
        }
        let total_wait_ms = poll_interval_ms
            .checked_mul(max_receipt_polls.saturating_sub(1))
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
    pub const fn poll_interval_ms(&self) -> u64 {
        self.poll_interval_ms
    }

    /// Returns maximum receipt poll attempts.
    pub const fn max_receipt_polls(&self) -> u64 {
        self.max_receipt_polls
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

/// Validation assertion policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-policy",
    schema = "mfm.evm.contract.config.validation_policy"
)]
pub struct ContractValidationPolicy {
    read_assertions: Vec<ReadAssertionConfig>,
    event_assertions: Vec<EventAssertionConfig>,
}

impl ContractValidationPolicy {
    /// Returns read assertions evaluated with EVM calls.
    pub fn read_assertions(&self) -> &[ReadAssertionConfig] {
        &self.read_assertions
    }

    /// Returns event assertions evaluated with EVM log queries.
    pub fn event_assertions(&self) -> &[EventAssertionConfig] {
        &self.event_assertions
    }
}

impl<'de> Deserialize<'de> for ContractValidationPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawValidationPolicy {
            #[serde(default)]
            read_assertions: Vec<ReadAssertionConfig>,
            #[serde(default)]
            event_assertions: Vec<EventAssertionConfig>,
        }

        let raw = RawValidationPolicy::deserialize(deserializer)?;
        Ok(Self {
            read_assertions: raw.read_assertions,
            event_assertions: raw.event_assertions,
        })
    }
}

/// Deploy phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-phase-config",
    schema = "mfm.evm.contract.config.deploy_phase"
)]
pub struct DeployPhaseConfig {
    artifact: Option<ContractArtifactConfig>,
    network: EvmNetworkIntent,
    signer: EvmSignerIntent,
    constructor_args: Vec<AbiArgumentValue>,
    value_wei: Option<String>,
    transaction: EvmTransactionPolicy,
    receipt: ReceiptRetryPolicy,
}

impl DeployPhaseConfig {
    /// Returns the optional inline contract artifact.
    pub const fn artifact(&self) -> Option<&ContractArtifactConfig> {
        self.artifact.as_ref()
    }

    /// Returns semantic network intent.
    pub const fn network(&self) -> &EvmNetworkIntent {
        &self.network
    }

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
        self.value_wei.as_deref()
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

impl<'de> Deserialize<'de> for DeployPhaseConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDeployPhaseConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network: EvmNetworkIntent,
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

        let raw = RawDeployPhaseConfig::deserialize(deserializer)?;
        validate_quantity(&raw.value_wei, "value_wei").map_err(de::Error::custom)?;
        Ok(Self {
            artifact: raw.artifact,
            network: raw.network,
            signer: raw.signer,
            constructor_args: raw.constructor_args,
            value_wei: raw.value_wei,
            transaction: raw.transaction,
            receipt: raw.receipt,
        })
    }
}

/// Configure phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-phase-config",
    schema = "mfm.evm.contract.config.configure_phase"
)]
pub struct ConfigurePhaseConfig {
    artifact: Option<ContractArtifactConfig>,
    network: EvmNetworkIntent,
    signer: EvmSignerIntent,
    calls: Vec<ContractCallConfig>,
    confirmation_read_assertions: Vec<ReadAssertionConfig>,
    confirmation_event_assertions: Vec<EventAssertionConfig>,
    transaction: EvmTransactionPolicy,
    receipt: ReceiptRetryPolicy,
}

impl ConfigurePhaseConfig {
    /// Returns the optional inline contract artifact.
    pub const fn artifact(&self) -> Option<&ContractArtifactConfig> {
        self.artifact.as_ref()
    }

    /// Returns semantic network intent.
    pub const fn network(&self) -> &EvmNetworkIntent {
        &self.network
    }

    /// Returns signer intent.
    pub const fn signer(&self) -> &EvmSignerIntent {
        &self.signer
    }

    /// Returns configured contract calls.
    pub fn calls(&self) -> &[ContractCallConfig] {
        &self.calls
    }

    /// Returns read confirmation assertions.
    pub fn confirmation_read_assertions(&self) -> &[ReadAssertionConfig] {
        &self.confirmation_read_assertions
    }

    /// Returns event confirmation assertions.
    pub fn confirmation_event_assertions(&self) -> &[EventAssertionConfig] {
        &self.confirmation_event_assertions
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

impl<'de> Deserialize<'de> for ConfigurePhaseConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawConfigurePhaseConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network: EvmNetworkIntent,
            signer: EvmSignerIntent,
            #[serde(default)]
            calls: Vec<ContractCallConfig>,
            #[serde(default)]
            confirmation_read_assertions: Vec<ReadAssertionConfig>,
            #[serde(default)]
            confirmation_event_assertions: Vec<EventAssertionConfig>,
            #[serde(default)]
            transaction: EvmTransactionPolicy,
            #[serde(default)]
            receipt: ReceiptRetryPolicy,
        }

        let raw = RawConfigurePhaseConfig::deserialize(deserializer)?;
        Ok(Self {
            artifact: raw.artifact,
            network: raw.network,
            signer: raw.signer,
            calls: raw.calls,
            confirmation_read_assertions: raw.confirmation_read_assertions,
            confirmation_event_assertions: raw.confirmation_event_assertions,
            transaction: raw.transaction,
            receipt: raw.receipt,
        })
    }
}

/// Validate phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validate-phase-config",
    schema = "mfm.evm.contract.config.validate_phase"
)]
pub struct ValidatePhaseConfig {
    artifact: Option<ContractArtifactConfig>,
    network: EvmNetworkIntent,
    validation: ContractValidationPolicy,
}

impl ValidatePhaseConfig {
    /// Returns the optional inline contract artifact.
    pub const fn artifact(&self) -> Option<&ContractArtifactConfig> {
        self.artifact.as_ref()
    }

    /// Returns semantic network intent.
    pub const fn network(&self) -> &EvmNetworkIntent {
        &self.network
    }

    /// Returns validation assertion policy.
    pub const fn validation(&self) -> &ContractValidationPolicy {
        &self.validation
    }
}

impl<'de> Deserialize<'de> for ValidatePhaseConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawValidatePhaseConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network: EvmNetworkIntent,
            #[serde(default)]
            validation: ContractValidationPolicy,
        }

        let raw = RawValidatePhaseConfig::deserialize(deserializer)?;
        Ok(Self {
            artifact: raw.artifact,
            network: raw.network,
            validation: raw.validation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_values::MfmConfig;

    fn network_json(chain_id: u64) -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": chain_id,
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
        })
    }

    fn configure_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(1),
        })
    }

    #[test]
    fn phase_configs_require_expected_chain_id() {
        let network_without_chain = serde_json::json!({"network_id": "ethereum-mainnet"});

        assert!(
            serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
                "network": network_without_chain,
                "signer": signer_json(),
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ConfigurePhaseConfig>(serde_json::json!({
                "network": serde_json::json!({"network_id": "ethereum-mainnet"}),
                "signer": signer_json(),
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ValidatePhaseConfig>(serde_json::json!({
                "network": serde_json::json!({"network_id": "ethereum-mainnet"}),
            }))
            .is_err()
        );
    }

    #[test]
    fn eip1559_transaction_policy_is_default() {
        let deploy: DeployPhaseConfig = serde_json::from_value(deploy_json()).expect("deploy");

        assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Eip1559);
        assert_eq!(deploy.receipt().poll_interval_ms(), 500);
        assert_eq!(deploy.network().expected_chain_id(), 1);
        assert_eq!(deploy.signer().signer_ref_str(), "deployer");
    }

    #[test]
    fn receipt_policy_rejects_unbounded_waits() {
        assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS + 1, 1).is_err());
        assert!(ReceiptRetryPolicy::new(1, MAX_RECEIPT_POLLS + 1).is_err());
        assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 62).is_err());
        assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 61).is_ok());
        assert!(
            serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
                "network": network_json(1),
                "signer": signer_json(),
                "receipt": {
                    "poll_interval_ms": MAX_RECEIPT_POLL_INTERVAL_MS + 1,
                    "max_receipt_polls": 1
                }
            }))
            .is_err()
        );
    }

    #[test]
    fn legacy_transaction_style_remains_accepted() {
        let deploy: DeployPhaseConfig = serde_json::from_value(serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
            "transaction": {
                "style": "legacy",
                "gas_price": "1000000000",
            },
        }))
        .expect("legacy deploy config");

        assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Legacy);
        assert_eq!(deploy.transaction().gas_price(), Some("1000000000"));
    }

    #[test]
    fn eip1559_transaction_policy_rejects_legacy_fee_field() {
        assert!(
            serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
                "network": network_json(1),
                "signer": signer_json(),
                "transaction": {
                    "style": "eip1559",
                    "gas_price": "1000000000",
                },
            }))
            .is_err()
        );
    }

    #[test]
    fn configs_deny_provider_and_runtime_fields() {
        let mut deploy = deploy_json();
        let provider_key = ["keystore", "_path_env"].concat();
        deploy
            .get_mut("signer")
            .and_then(serde_json::Value::as_object_mut)
            .expect("signer object")
            .insert(provider_key, serde_json::json!("MFM_SIGNER_FILE"));
        assert!(serde_json::from_value::<DeployPhaseConfig>(deploy).is_err());

        let mut validate = validate_json();
        let routing_key = ["rpc", "_url"].concat();
        validate
            .as_object_mut()
            .expect("validate object")
            .insert(routing_key, serde_json::json!("http://127.0.0.1:8545"));
        assert!(serde_json::from_value::<ValidatePhaseConfig>(validate).is_err());
    }

    #[test]
    fn configure_and_validate_phase_configs_parse() {
        let configure: ConfigurePhaseConfig =
            serde_json::from_value(configure_json()).expect("configure");
        let validate: ValidatePhaseConfig =
            serde_json::from_value(validate_json()).expect("validate");

        assert_eq!(configure.calls().len(), 0);
        assert_eq!(validate.validation().read_assertions().len(), 0);
    }

    #[test]
    fn schema_ids_use_contract_config_namespace() {
        let schema_ids = [
            DeployPhaseConfig::schema_id().expect("schema").to_string(),
            ConfigurePhaseConfig::schema_id()
                .expect("schema")
                .to_string(),
            ValidatePhaseConfig::schema_id()
                .expect("schema")
                .to_string(),
            EvmNetworkIntent::schema_id().expect("schema").to_string(),
            EvmSignerIntent::schema_id().expect("schema").to_string(),
            EvmTransactionPolicy::schema_id()
                .expect("schema")
                .to_string(),
        ];

        assert!(schema_ids
            .iter()
            .all(|schema_id| schema_id.contains("mfm.evm.contract.config")));
    }
}
