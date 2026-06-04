//! Deploy/configure/validate model and ABI preparation helpers.
//!
//! This crate owns the pure data-shaping layer behind EVM deploy/configure/validate workflows:
//!
//! - contract artifact parsing
//! - ABI-based calldata encoding
//! - validation assertion preparation
//! - small normalization helpers for addresses, blocks, and values
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_dcv_model::{
//!     prepare_validate_assertions, AbiJson, BlockTag, EventAssertionConfig, ExpectedValue,
//!     ReadAssertionConfig,
//! };
//!
//! let abi_json = AbiJson::from_json_value(&serde_json::json!([
//!     {
//!         "type": "function",
//!         "name": "owner",
//!         "inputs": [],
//!         "outputs": [{ "name": "", "type": "address" }],
//!         "stateMutability": "view"
//!     },
//!     {
//!         "type": "event",
//!         "name": "Configured",
//!         "inputs": [],
//!         "anonymous": false
//!     }
//! ]))
//! .unwrap();
//! let abi = mfm_evm_dcv_model::parse_abi(&abi_json).unwrap();
//!
//! let (reads, events) = prepare_validate_assertions(
//!     &abi,
//!     &[ReadAssertionConfig {
//!         function: "owner".to_string(),
//!         args: vec![],
//!         expected: ExpectedValue::from_json_value(&serde_json::json!(
//!             "0x0000000000000000000000000000000000000000"
//!         ))
//!         .unwrap(),
//!     }],
//!     &[EventAssertionConfig {
//!         event: "Configured".to_string(),
//!         min_count: 1,
//!         from_block: Some(BlockTag::Number { number: 0 }),
//!         to_block: Some(BlockTag::Tag {
//!             tag: "latest".to_string(),
//!         }),
//!     }],
//! )
//! .unwrap();
//!
//! assert_eq!(reads.len(), 1);
//! assert_eq!(events[0].event, "Configured");
//! assert!(events[0].topic0_hex.starts_with("0x"));
//! ```

use alloy_primitives::keccak256;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

fn canonical_json_text_from_str(input: &str) -> Result<String, String> {
    PlainCanonicalJsonBytes::from_json_str(input)
        .map(|canonical| canonical.to_string())
        .map_err(|error| error.to_string())
}

fn canonical_json_text_from_value(value: &Value) -> Result<String, String> {
    canonical_json_text_from_str(&value.to_string())
}

fn legacy_or_typed_json_text(value: Value) -> Result<String, String> {
    match value {
        Value::Object(mut object) if object.len() == 1 && object.contains_key("json_text") => {
            match object.remove("json_text") {
                Some(Value::String(json_text)) => canonical_json_text_from_str(&json_text),
                Some(_) => Err("json_text must be a string".to_string()),
                None => unreachable!("object contains json_text"),
            }
        }
        other => canonical_json_text_from_value(&other),
    }
}

fn deserialize_json_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    legacy_or_typed_json_text(value).map_err(de::Error::custom)
}

fn json_value_from_text(json_text: &str) -> Result<Value, String> {
    serde_json::from_str(json_text).map_err(|error| error.to_string())
}

macro_rules! impl_json_text_value {
    ($ty:ty) => {
        impl $ty {
            /// Builds the wrapper from already-authored JSON text.
            pub fn from_json_text(json_text: impl AsRef<str>) -> Result<Self, String> {
                Ok(Self {
                    json_text: canonical_json_text_from_str(json_text.as_ref())?,
                })
            }

            /// Builds the wrapper from a JSON value.
            pub fn from_json_value(value: &Value) -> Result<Self, String> {
                Ok(Self {
                    json_text: canonical_json_text_from_value(value)?,
                })
            }

            /// Returns the canonical JSON text carried by this typed wrapper.
            pub fn json_text(&self) -> &str {
                &self.json_text
            }

            /// Parses the wrapper into a JSON value for ABI preparation.
            pub fn to_json_value(&self) -> Result<Value, String> {
                json_value_from_text(&self.json_text)
            }
        }

        impl TryFrom<Value> for $ty {
            type Error = String;

            fn try_from(value: Value) -> Result<Self, Self::Error> {
                Ok(Self {
                    json_text: legacy_or_typed_json_text(value)?,
                })
            }
        }
    };
}

/// Typed wrapper for contract ABI JSON payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "abi-json",
    schema = "mfm.evm.dcv.value.abi_json"
)]
pub struct AbiJson {
    /// Canonical JSON text for the contract ABI.
    pub json_text: String,
}

impl<'de> Deserialize<'de> for AbiJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            json_text: deserialize_json_text(deserializer)?,
        })
    }
}

impl_json_text_value!(AbiJson);

impl AsRef<AbiJson> for AbiJson {
    fn as_ref(&self) -> &AbiJson {
        self
    }
}

/// Typed wrapper for contract bytecode JSON payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "bytecode-json",
    schema = "mfm.evm.dcv.value.bytecode_json"
)]
pub struct BytecodeJson {
    /// Canonical JSON text for the contract bytecode payload.
    pub json_text: String,
}

impl<'de> Deserialize<'de> for BytecodeJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            json_text: deserialize_json_text(deserializer)?,
        })
    }
}

impl_json_text_value!(BytecodeJson);

/// Typed wrapper for ABI-encoded function and constructor arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "abi-argument-value",
    schema = "mfm.evm.dcv.value.abi_argument"
)]
pub struct AbiArgumentValue {
    /// Canonical JSON text for the ABI argument value.
    pub json_text: String,
}

impl<'de> Deserialize<'de> for AbiArgumentValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            json_text: deserialize_json_text(deserializer)?,
        })
    }
}

impl_json_text_value!(AbiArgumentValue);

impl AsRef<AbiArgumentValue> for AbiArgumentValue {
    fn as_ref(&self) -> &AbiArgumentValue {
        self
    }
}

/// Typed wrapper for validation assertion expected values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "expected-value",
    schema = "mfm.evm.dcv.value.expected"
)]
pub struct ExpectedValue {
    /// Canonical JSON text for the expected validation value.
    pub json_text: String,
}

impl<'de> Deserialize<'de> for ExpectedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            json_text: deserialize_json_text(deserializer)?,
        })
    }
}

impl_json_text_value!(ExpectedValue);

/// JSON contract artifact used by deploy/configure/validate flows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "contract-artifact-config",
    schema = "mfm.evm.dcv.config.contract_artifact"
)]
pub struct ContractArtifactConfig {
    /// Contract ABI JSON payload.
    pub abi: AbiJson,
    /// Contract bytecode JSON payload.
    pub bytecode: BytecodeJson,
}

/// Runtime configuration for a single on-chain function call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-call-config",
    schema = "mfm.evm.dcv.config.configure_call"
)]
pub struct ConfigureCallConfig {
    /// Function name to invoke.
    pub function: String,

    #[serde(default)]
    /// Positional arguments passed to the function call.
    pub args: Vec<AbiArgumentValue>,

    #[serde(default)]
    /// Optional call value expressed in wei.
    pub value_wei: Option<String>,
}

fn default_min_count() -> u64 {
    1
}

/// Block selector used by validation reads and event queries.
///
/// Numeric blocks are rendered as hex quantities; string tags are passed through as-is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "block-tag",
    schema = "mfm.evm.dcv.value.block_tag"
)]
pub enum BlockTag {
    /// Explicit block number.
    Number {
        /// Block number.
        number: u64,
    },
    /// Symbolic tag such as `latest` or `earliest`.
    Tag {
        /// Symbolic block tag.
        tag: String,
    },
}

impl<'de> Deserialize<'de> for BlockTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::Number(number) => number
                .as_u64()
                .map(|number| Self::Number { number })
                .ok_or_else(|| de::Error::custom("block number must be an unsigned integer")),
            Value::String(tag) => Ok(Self::Tag { tag }),
            Value::Object(mut object) => {
                let Some(Value::String(kind)) = object.remove("kind") else {
                    return Err(de::Error::custom("block tag object requires string kind"));
                };
                match kind.as_str() {
                    "number" => match object.remove("number") {
                        Some(Value::Number(number)) => {
                            number.as_u64().map(|number| Self::Number { number })
                        }
                        _ => None,
                    }
                    .ok_or_else(|| de::Error::custom("number block tag requires number")),
                    "tag" => match object.remove("tag") {
                        Some(Value::String(tag)) => Ok(Self::Tag { tag }),
                        _ => Err(de::Error::custom("tag block tag requires tag string")),
                    },
                    _ => Err(de::Error::custom("unsupported block tag kind")),
                }
            }
            _ => Err(de::Error::custom(
                "block tag must be a number, string, or typed block-tag object",
            )),
        }
    }
}

/// Read assertion to evaluate through `eth_call`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "read-assertion-config",
    schema = "mfm.evm.dcv.config.read_assertion"
)]
pub struct ReadAssertionConfig {
    /// Function name to call.
    pub function: String,

    #[serde(default)]
    /// Positional arguments passed to the function call.
    pub args: Vec<AbiArgumentValue>,

    /// Expected decoded value.
    pub expected: ExpectedValue,
}

/// Event assertion to evaluate with `eth_getLogs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "event-assertion-config",
    schema = "mfm.evm.dcv.config.event_assertion"
)]
pub struct EventAssertionConfig {
    /// Event name expected in the ABI.
    pub event: String,

    /// Minimum matching log count required for success.
    pub min_count: u64,

    /// Optional lower bound for the log query.
    pub from_block: Option<BlockTag>,

    /// Optional upper bound for the log query.
    pub to_block: Option<BlockTag>,
}

impl<'de> Deserialize<'de> for EventAssertionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawEventAssertionConfig {
            event: String,
            #[serde(default = "default_min_count")]
            min_count: u64,
            #[serde(default)]
            from_block: Option<BlockTag>,
            #[serde(default)]
            to_block: Option<BlockTag>,
        }

        let raw = RawEventAssertionConfig::deserialize(deserializer)?;
        Ok(Self {
            event: raw.event,
            min_count: raw.min_count,
            from_block: raw.from_block,
            to_block: raw.to_block,
        })
    }
}

/// Typestate value emitted after a contract deployment has been confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deployed-contract",
    schema = "mfm.evm.dcv.value.deployed_contract"
)]
pub struct DeployedContract {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Stable network identifier targeted by the deploy state.
    pub network_id: String,
    /// Stable control-plane scope used for managed EVM source state.
    pub control_scope: String,
    /// Normalized deployed contract address.
    pub contract_address: String,
    /// Transaction hash for the deployment transaction.
    pub deploy_tx_hash: String,
    /// Optional content-addressed artifact id for the deployment receipt.
    pub deploy_receipt_artifact_id: Option<String>,
    /// Optional block number that confirmed the deployment.
    pub deployed_block_number: Option<u64>,
}

/// Typestate value emitted after deployment configuration has been confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configured-contract",
    schema = "mfm.evm.dcv.value.configured_contract"
)]
pub struct ConfiguredContract {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Deployment stage value consumed by the configure state.
    pub deployed: DeployedContract,
    /// Configuration calls requested against the deployed contract.
    pub configure_calls: Vec<ConfigureCallConfig>,
    /// Read confirmations required to prove the intended configuration is live on-chain.
    pub confirmation_read_assertions: Vec<ReadAssertionConfig>,
    /// Event confirmations required to prove the intended configuration was observed on-chain.
    pub confirmation_event_assertions: Vec<EventAssertionConfig>,
    /// Transaction hashes for configuration calls.
    pub configure_tx_hashes: Vec<String>,
    /// Optional content-addressed artifact ids for configuration receipts.
    pub configure_receipt_artifact_ids: Vec<String>,
    /// Optional highest block number that confirmed configuration.
    pub configured_block_number: Option<u64>,
}

/// Typed reference to a contract known to have completed configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configured-contract-ref",
    schema = "mfm.evm.dcv.value.configured_contract_ref"
)]
pub struct ConfiguredContractRef {
    /// Reference contract version.
    pub ref_version: u64,
    /// Stable network identifier for the configured contract.
    pub network_id: String,
    /// Stable control-plane scope used for managed EVM source state.
    pub control_scope: String,
    /// Normalized configured contract address.
    pub contract_address: String,
    /// Optional artifact id containing evidence for the configured state.
    pub evidence_artifact_id: Option<String>,
}

impl ConfiguredContractRef {
    /// Builds a typed reference from a configured contract lifecycle value.
    pub fn from_configured(configured: &ConfiguredContract) -> Self {
        Self {
            ref_version: 1,
            network_id: configured.deployed.network_id.clone(),
            control_scope: configured.deployed.control_scope.clone(),
            contract_address: configured.deployed.contract_address.clone(),
            evidence_artifact_id: None,
        }
    }
}

/// Typed reference for validating a pre-existing configured deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "existing-configured-contract-ref",
    schema = "mfm.evm.dcv.value.existing_configured_contract_ref"
)]
pub struct ExistingConfiguredContractRef {
    /// Reference contract version.
    pub ref_version: u64,
    /// Configured contract being imported into a validation-only workflow.
    pub configured_contract: ConfiguredContractRef,
    /// Redaction-safe source label for the imported reference.
    pub source: String,
}

/// Result for one validation read assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "validation-read-result",
    schema = "mfm.evm.dcv.value.validation_read_result"
)]
pub struct ValidationReadResult {
    /// Function evaluated by the read assertion.
    pub function: String,
    /// ABI argument values supplied to the read.
    pub args: Vec<AbiArgumentValue>,
    /// Expected decoded value.
    pub expected: ExpectedValue,
    /// Actual decoded value.
    pub actual: ExpectedValue,
    /// Whether the assertion passed.
    pub passed: bool,
}

/// Result for one validation event assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "validation-event-result",
    schema = "mfm.evm.dcv.value.validation_event_result"
)]
pub struct ValidationEventResult {
    /// Event evaluated by the event assertion.
    pub event: String,
    /// Minimum expected matching log count.
    pub min_count: u64,
    /// Observed matching log count.
    pub observed_count: u64,
    /// Whether the assertion passed.
    pub passed: bool,
}

/// Typestate value emitted after validating a configured contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "validation-report",
    schema = "mfm.evm.dcv.value.validation_report"
)]
pub struct ValidationReport {
    /// Validation report contract version.
    pub report_version: u64,
    /// Configured contract that was validated.
    pub configured_contract: ConfiguredContractRef,
    /// Expected chain id from validation config.
    pub expected_chain_id: u64,
    /// Observed chain id returned by the EVM RPC endpoint.
    pub observed_chain_id: u64,
    /// Redaction-safe client version returned by the EVM RPC endpoint.
    pub client_version: String,
    /// Results for configuration-intent read confirmations stored on the configured contract.
    pub configuration_read_results: Vec<ValidationReadResult>,
    /// Results for configuration-intent event confirmations stored on the configured contract.
    pub configuration_event_results: Vec<ValidationEventResult>,
    /// Additional read assertion results from validation config.
    pub read_results: Vec<ValidationReadResult>,
    /// Additional event assertion results from validation config.
    pub event_results: Vec<ValidationEventResult>,
    /// Whether all validation checks passed.
    pub valid: bool,
}

/// Prepared read assertion ready for runtime execution.
#[derive(Clone, Debug)]
pub struct PreparedReadAssertion {
    /// Encoded calldata for the asserted function call.
    pub data_hex: String,
    /// Expected decoded value.
    pub expected: ExpectedValue,
    /// ABI output types used during decoding.
    pub outputs: Vec<String>,
}

/// Prepared event assertion ready for runtime execution.
#[derive(Clone, Debug)]
pub struct PreparedEventAssertion {
    /// Event name referenced by the assertion.
    pub event: String,
    /// Keccak event signature topic.
    pub topic0_hex: String,
    /// Minimum matching log count required for success.
    pub min_count: u64,
    /// Lower bound for the log query.
    pub from_block: serde_json::Value,
    /// Upper bound for the log query.
    pub to_block: serde_json::Value,
}

/// Parses typed ABI JSON into the shared ABI representation.
pub fn parse_abi(abi: &AbiJson) -> Result<ParsedAbi, String> {
    let abi = abi.to_json_value()?;
    common_abi::parse_abi(&abi).map_err(|e| e.message)
}

/// Normalizes a hex string to lowercase `0x`-prefixed form.
pub fn normalize_hex_str(s: &str) -> Result<String, String> {
    common_hex::normalize_hex_str(s).map_err(|e| e.message)
}

/// Decodes a hex string into bytes.
pub fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    common_hex::hex_to_bytes(s).map_err(|e| e.message)
}

/// Encodes bytes as a lowercase `0x`-prefixed hex string.
pub fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    common_hex::bytes_to_hex_prefixed(bytes)
}

/// Resolves and encodes a function call from ABI name plus typed JSON arguments.
pub fn resolve_function_call<T>(
    abi: &ParsedAbi,
    function_name: &str,
    args: &[T],
) -> Result<(Vec<u8>, Vec<String>), String>
where
    T: AsRef<AbiArgumentValue>,
{
    let args = args
        .iter()
        .map(|arg| arg.as_ref().to_json_value())
        .collect::<Result<Vec<_>, _>>()?;
    common_abi::resolve_function_call(abi, function_name, &args).map_err(|e| e.message)
}

/// Encodes constructor bytecode plus typed constructor arguments.
pub fn constructor_data<T>(
    abi: &ParsedAbi,
    bytecode: &[u8],
    constructor_args: &[T],
) -> Result<Vec<u8>, String>
where
    T: AsRef<AbiArgumentValue>,
{
    let constructor_args = constructor_args
        .iter()
        .map(|arg| arg.as_ref().to_json_value())
        .collect::<Result<Vec<_>, _>>()?;
    common_abi::constructor_data(abi, bytecode, &constructor_args).map_err(|e| e.message)
}

/// Parses a contract artifact into validated ABI and bytecode components.
pub fn parse_artifact(cfg: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&cfg.abi)?;
    let bytecode_json = cfg.bytecode.to_json_value()?;
    let bytecode = common_abi::parse_bytecode(&bytecode_json).map_err(|e| e.message)?;
    Ok((abi, bytecode))
}

fn block_tag_to_rpc_value(block: &Option<BlockTag>, default_latest: bool) -> serde_json::Value {
    match block {
        Some(BlockTag::Number { number }) => serde_json::json!(format!("0x{:x}", number)),
        Some(BlockTag::Tag { tag }) => serde_json::json!(tag),
        None if default_latest => serde_json::json!("latest"),
        None => serde_json::json!("earliest"),
    }
}

/// Prepares read and event assertions for runtime validation.
pub fn prepare_validate_assertions(
    abi: &ParsedAbi,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<(Vec<PreparedReadAssertion>, Vec<PreparedEventAssertion>), String> {
    let mut reads = Vec::with_capacity(read_assertions.len());
    for ra in read_assertions {
        let (call_data, outputs) = resolve_function_call(abi, &ra.function, &ra.args)
            .map_err(|_| "read assertion did not match ABI".to_string())?;
        reads.push(PreparedReadAssertion {
            data_hex: bytes_to_hex_prefixed(&call_data),
            expected: ra.expected.clone(),
            outputs,
        });
    }

    let mut events = Vec::with_capacity(event_assertions.len());
    for ea in event_assertions {
        let ev = abi
            .events
            .iter()
            .find(|e| e.name == ea.event)
            .ok_or_else(|| "event assertion referenced unknown event".to_string())?;
        if ev.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }
        let sig = format!("{}({})", ev.name, ev.inputs.join(","));
        let topic0 = keccak256(sig.as_bytes());
        let topic0_hex = bytes_to_hex_prefixed(topic0.as_slice());
        let from_block = block_tag_to_rpc_value(&ea.from_block, false);
        let to_block = block_tag_to_rpc_value(&ea.to_block, true);
        events.push(PreparedEventAssertion {
            event: ea.event.clone(),
            topic0_hex,
            min_count: ea.min_count,
            from_block,
            to_block,
        });
    }

    Ok((reads, events))
}

/// Normalizes an optional wei value into RPC hex quantity form.
pub fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, String> {
    common_abi::parse_value_wei_to_hex(value_wei).map_err(|e| e.message)
}

/// Ensures the receipt-poll budget is non-zero.
pub fn ensure_nonzero_polls(max_receipt_polls: u64) -> Result<(), String> {
    if max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

/// Ensures the configured artifact context port is non-empty.
pub fn ensure_nonempty_artifact_port(artifact_port: &str) -> Result<(), String> {
    if artifact_port.trim().is_empty() {
        return Err("artifact_port must be non-empty".to_string());
    }
    Ok(())
}

/// Normalizes an EVM address to canonical lowercase `0x`-prefixed form.
pub fn normalize_address(s: &str) -> Result<String, String> {
    encoding::normalize_address(s).map_err(|e| e.message)
}

/// Decodes a single-output `eth_call` response into JSON.
pub fn decode_single_output_to_json(
    outputs: &[String],
    raw_hex: &str,
) -> Result<serde_json::Value, String> {
    if outputs.len() != 1 {
        return Err("only single-output assertions are supported".to_string());
    }
    let decoded =
        common_abi::decode_single_output_to_json(outputs, raw_hex).map_err(|e| e.message)?;
    if outputs[0] != "bool"
        && outputs[0] != "address"
        && outputs[0] != "uint256"
        && outputs[0] != "uint"
    {
        return Err(format!(
            "unsupported output type for assertion: {}",
            outputs[0]
        ));
    }
    Ok(decoded)
}

/// Returns whether `actual` satisfies the configured expected value.
pub fn expected_matches(actual: &ExpectedValue, expected: &ExpectedValue) -> bool {
    let Ok(actual) = actual.to_json_value() else {
        return false;
    };
    let Ok(expected) = expected.to_json_value() else {
        return false;
    };

    if expected == actual {
        return true;
    }

    match (&actual, &expected) {
        (Value::String(a), Value::String(e)) => {
            if a.starts_with("0x") && e.starts_with("0x") {
                normalize_hex_str(a).ok() == normalize_hex_str(e).ok()
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_values::MfmValue;

    #[test]
    fn expected_matches_normalizes_hex_wrappers() {
        let actual = ExpectedValue::from_json_value(&serde_json::json!("0xAA")).expect("actual");
        let expected =
            ExpectedValue::from_json_value(&serde_json::json!("0xaa")).expect("expected");

        assert!(expected_matches(&actual, &expected));
    }

    #[test]
    fn json_wrappers_reject_non_canonical_float_values() {
        assert!(AbiArgumentValue::from_json_value(&serde_json::json!(1.5)).is_err());
        assert!(ExpectedValue::from_json_value(&serde_json::json!(1.5)).is_err());
    }

    #[test]
    fn typed_json_wrappers_have_no_unchecked_value_conversion() {
        let source = include_str!("lib.rs");

        for forbidden in [
            ["impl ", "Dcv", "Json", "Input", " for ", "Value"].concat(),
            ["impl ", "From", "<Value> for ", "Abi", "Json"].concat(),
            ["impl ", "From", "<Value> for ", "Bytecode", "Json"].concat(),
            ["impl ", "From", "<Value> for ", "Abi", "Argument", "Value"].concat(),
            ["impl ", "From", "<Value> for ", "Expected", "Value"].concat(),
        ] {
            assert!(!source.contains(&forbidden), "found {forbidden}");
        }
    }

    #[test]
    fn prepare_validate_assertions_preserves_typed_expected_values() {
        let abi_json = AbiJson::from_json_value(&serde_json::json!([
            {
                "type": "function",
                "name": "owner",
                "inputs": [],
                "outputs": [{ "name": "", "type": "address" }],
                "stateMutability": "view"
            }
        ]))
        .expect("abi json");
        let abi = parse_abi(&abi_json).expect("abi");
        let expected = ExpectedValue::from_json_value(&serde_json::json!(
            "0x0000000000000000000000000000000000000000"
        ))
        .expect("expected");

        let (reads, events) = prepare_validate_assertions(
            &abi,
            &[ReadAssertionConfig {
                function: "owner".to_string(),
                args: vec![],
                expected: expected.clone(),
            }],
            &[],
        )
        .expect("prepare");

        assert_eq!(events.len(), 0);
        assert_eq!(reads[0].expected, expected);
    }

    #[test]
    fn configured_ref_is_derived_from_configured_lifecycle_value() {
        let deployed = DeployedContract {
            lifecycle_version: 1,
            network_id: "ethereum-mainnet".to_string(),
            control_scope: "shared".to_string(),
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            deploy_tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            deploy_receipt_artifact_id: None,
            deployed_block_number: Some(10),
        };
        let configured = ConfiguredContract {
            lifecycle_version: 1,
            deployed,
            configure_calls: Vec::new(),
            confirmation_read_assertions: Vec::new(),
            confirmation_event_assertions: Vec::new(),
            configure_tx_hashes: vec![
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ],
            configure_receipt_artifact_ids: Vec::new(),
            configured_block_number: Some(11),
        };

        let configured_ref = ConfiguredContractRef::from_configured(&configured);

        assert_eq!(configured_ref.network_id, configured.deployed.network_id);
        assert_eq!(
            configured_ref.contract_address,
            configured.deployed.contract_address
        );
    }

    #[test]
    fn validation_report_targets_configured_ref_not_raw_lifecycle_stage() {
        let source = include_str!("lib.rs");
        let report_struct = source
            .split("pub struct ValidationReport")
            .nth(1)
            .expect("report struct")
            .split("/// Prepared read assertion")
            .next()
            .expect("report body");

        assert!(report_struct.contains("configured_contract: ConfiguredContractRef"));
        assert!(!report_struct.contains("deployed: DeployedContract"));
    }

    #[test]
    fn json_wrappers_canonicalize_legacy_values() {
        let arg = AbiArgumentValue::from_json_value(&serde_json::json!({
            "b": 2,
            "a": 1
        }))
        .expect("arg");

        assert_eq!(arg.json_text(), r#"{"a":1,"b":2}"#);
        assert_eq!(
            arg.to_json_value().expect("json value"),
            serde_json::json!({"a": 1, "b": 2})
        );
    }

    #[test]
    fn block_tag_accepts_legacy_and_typed_shapes() {
        let legacy_number: BlockTag = serde_json::from_value(serde_json::json!(12)).expect("tag");
        let legacy_tag: BlockTag =
            serde_json::from_value(serde_json::json!("latest")).expect("tag");
        let typed_number: BlockTag =
            serde_json::from_value(serde_json::json!({"kind": "number", "number": 12}))
                .expect("tag");

        assert_eq!(legacy_number, BlockTag::Number { number: 12 });
        assert_eq!(
            legacy_tag,
            BlockTag::Tag {
                tag: "latest".to_string()
            }
        );
        assert_eq!(typed_number, legacy_number);
    }

    #[test]
    fn lifecycle_values_have_typed_schema_ids() {
        assert!(DeployedContract::schema_id().is_ok());
        assert!(ConfiguredContract::schema_id().is_ok());
        assert!(ConfiguredContractRef::schema_id().is_ok());
        assert!(ExistingConfiguredContractRef::schema_id().is_ok());
        assert!(ValidationReport::schema_id().is_ok());
    }
}
