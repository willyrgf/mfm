#![warn(missing_docs)]
//! Reusable EVM contract lifecycle model and ABI preparation helpers.
//!
//! This crate owns pure contract data shaping:
//!
//! - contract artifact parsing
//! - ABI calldata encoding
//! - validation assertion preparation
//! - lifecycle typestate values
//! - typed artifact evidence references
//!
//! ```rust
//! use mfm_evm_contract_model::{
//!     prepare_validate_assertions, AbiJson, BlockSelector, EventAssertionConfig, ExpectedValue,
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
//! ]))?;
//! let abi = mfm_evm_contract_model::parse_abi(&abi_json)?;
//!
//! let (reads, events) = prepare_validate_assertions(
//!     &abi,
//!     &[ReadAssertionConfig {
//!         function: "owner".to_string(),
//!         args: vec![],
//!         expected: ExpectedValue::from_json_value(&serde_json::json!(
//!             "0x0000000000000000000000000000000000000000"
//!         ))?,
//!     }],
//!     &[EventAssertionConfig {
//!         event: "Configured".to_string(),
//!         min_count: 1,
//!         from_block: Some(BlockSelector::Number { number: 0 }),
//!         to_block: Some(BlockSelector::Tag {
//!             tag: "latest".to_string(),
//!         }),
//!     }],
//! )?;
//!
//! assert_eq!(reads.len(), 1);
//! assert_eq!(events[0].event, "Configured");
//! assert!(events[0].topic0_hex.starts_with("0x"));
//! # Ok::<(), String>(())
//! ```

use alloy_primitives::keccak256;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;
use mfm_ids::{ArtifactId, ContentDigest, SchemaId, SemanticTypeId};
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

fn canonical_json_text_from_str(input: &str) -> Result<String, String> {
    PlainCanonicalJsonBytes::from_json_str(input)
        .map(|canonical| canonical.to_string())
        .map_err(|error| error.to_string())
}

fn canonical_json_text_from_value(value: &Value) -> Result<String, String> {
    canonical_json_text_from_str(&value.to_string())
}

fn strict_json_text_value(value: Value) -> Result<String, String> {
    match value {
        Value::Object(mut object) if object.len() == 1 && object.contains_key("json_text") => {
            match object.remove("json_text") {
                Some(Value::String(json_text)) => canonical_json_text_from_str(&json_text),
                Some(_) => Err("json_text must be a string".to_string()),
                None => unreachable!("object contains json_text"),
            }
        }
        _ => Err("json wrapper requires an object with json_text".to_string()),
    }
}

fn deserialize_json_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    strict_json_text_value(value).map_err(de::Error::custom)
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
    };
}

/// Typed wrapper for contract ABI JSON payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.abi",
    name = "json",
    schema = "mfm.evm.abi.value.json"
)]
pub struct AbiJson {
    json_text: String,
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
    namespace = "mfm.evm.contract",
    name = "bytecode-json",
    schema = "mfm.evm.contract.value.bytecode_json"
)]
pub struct BytecodeJson {
    json_text: String,
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
    namespace = "mfm.evm.abi",
    name = "argument-value",
    schema = "mfm.evm.abi.value.argument"
)]
pub struct AbiArgumentValue {
    json_text: String,
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
    namespace = "mfm.evm.contract",
    name = "expected-value",
    schema = "mfm.evm.contract.value.expected"
)]
pub struct ExpectedValue {
    json_text: String,
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

/// Typed artifact evidence reference used by contract lifecycle values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "artifact-evidence-ref",
    schema = "mfm.evm.contract.value.artifact_evidence_ref"
)]
pub struct LifecycleArtifactEvidenceRef {
    artifact_id: String,
    content_digest: String,
    byte_len: u64,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
}

impl LifecycleArtifactEvidenceRef {
    /// Creates a typed lifecycle artifact evidence reference.
    pub fn new(
        artifact_id: ArtifactId,
        content_digest: ContentDigest,
        byte_len: u64,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<SemanticTypeId>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.to_string(),
            content_digest: content_digest.to_string(),
            byte_len,
            schema_id: schema_id.map(|schema_id| schema_id.to_string()),
            semantic_type_id: semantic_type_id.map(|semantic_type_id| semantic_type_id.to_string()),
        }
    }

    /// Returns the artifact id string.
    pub fn artifact_id_str(&self) -> &str {
        &self.artifact_id
    }

    /// Parses and returns the typed artifact id.
    pub fn artifact_id(&self) -> Result<ArtifactId, String> {
        ArtifactId::parse(&self.artifact_id).map_err(|error| error.to_string())
    }

    /// Returns the content digest string.
    pub fn content_digest_str(&self) -> &str {
        &self.content_digest
    }

    /// Parses and returns the typed content digest.
    pub fn content_digest(&self) -> Result<ContentDigest, String> {
        ContentDigest::parse(&self.content_digest).map_err(|error| error.to_string())
    }

    /// Returns the artifact byte length.
    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Returns the optional schema id string.
    pub fn schema_id_str(&self) -> Option<&str> {
        self.schema_id.as_deref()
    }

    /// Parses and returns the optional typed schema id.
    pub fn schema_id(&self) -> Result<Option<SchemaId>, String> {
        self.schema_id
            .as_deref()
            .map(SchemaId::parse)
            .transpose()
            .map_err(|error| error.to_string())
    }

    /// Returns the optional semantic type id string.
    pub fn semantic_type_id_str(&self) -> Option<&str> {
        self.semantic_type_id.as_deref()
    }

    /// Parses and returns the optional typed semantic type id.
    pub fn semantic_type_id(&self) -> Result<Option<SemanticTypeId>, String> {
        self.semantic_type_id
            .as_deref()
            .map(SemanticTypeId::parse)
            .transpose()
            .map_err(|error| error.to_string())
    }

    fn validate(&self) -> Result<(), String> {
        self.artifact_id()?;
        self.content_digest()?;
        self.schema_id()?;
        self.semantic_type_id()?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for LifecycleArtifactEvidenceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawLifecycleArtifactEvidenceRef {
            artifact_id: String,
            content_digest: String,
            byte_len: u64,
            #[serde(default)]
            schema_id: Option<String>,
            #[serde(default)]
            semantic_type_id: Option<String>,
        }

        let raw = RawLifecycleArtifactEvidenceRef::deserialize(deserializer)?;
        let evidence = Self {
            artifact_id: raw.artifact_id,
            content_digest: raw.content_digest,
            byte_len: raw.byte_len,
            schema_id: raw.schema_id,
            semantic_type_id: raw.semantic_type_id,
        };
        evidence.validate().map_err(de::Error::custom)?;
        Ok(evidence)
    }
}

/// JSON contract artifact used by lifecycle phases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "artifact-config",
    schema = "mfm.evm.contract.config.artifact"
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
    namespace = "mfm.evm.contract",
    name = "call-config",
    schema = "mfm.evm.contract.config.call"
)]
pub struct ContractCallConfig {
    /// Function name to invoke.
    pub function: String,

    /// Positional arguments passed to the function call.
    #[serde(default)]
    pub args: Vec<AbiArgumentValue>,

    /// Optional call value expressed in wei.
    #[serde(default)]
    pub value_wei: Option<String>,
}

fn default_min_count() -> u64 {
    1
}

/// Block selector used by validation reads and event queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "block-selector",
    schema = "mfm.evm.contract.value.block_selector"
)]
pub enum BlockSelector {
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

impl<'de> Deserialize<'de> for BlockSelector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum RawBlockSelector {
            Number { number: u64 },
            Tag { tag: String },
        }

        match RawBlockSelector::deserialize(deserializer)? {
            RawBlockSelector::Number { number } => Ok(Self::Number { number }),
            RawBlockSelector::Tag { tag } => Ok(Self::Tag { tag }),
        }
    }
}

/// Read assertion to evaluate through an EVM call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "read-assertion-config",
    schema = "mfm.evm.contract.config.read_assertion"
)]
pub struct ReadAssertionConfig {
    /// Function name to call.
    pub function: String,

    /// Positional arguments passed to the function call.
    #[serde(default)]
    pub args: Vec<AbiArgumentValue>,

    /// Expected decoded value.
    pub expected: ExpectedValue,
}

/// Event assertion to evaluate with an EVM log query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "event-assertion-config",
    schema = "mfm.evm.contract.config.event_assertion"
)]
pub struct EventAssertionConfig {
    /// Event name expected in the ABI.
    pub event: String,

    /// Minimum matching log count required for success.
    pub min_count: u64,

    /// Optional lower bound for the log query.
    pub from_block: Option<BlockSelector>,

    /// Optional upper bound for the log query.
    pub to_block: Option<BlockSelector>,
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
            from_block: Option<BlockSelector>,
            #[serde(default)]
            to_block: Option<BlockSelector>,
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
    namespace = "mfm.evm.contract",
    name = "deployed-contract",
    schema = "mfm.evm.contract.value.deployed_contract"
)]
pub struct DeployedContract {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Stable network identifier targeted by the deploy state.
    pub network_id: String,
    /// Expected chain id for the semantic network target.
    pub expected_chain_id: u64,
    /// Normalized deployed contract address.
    pub contract_address: String,
    /// Transaction hash for the deployment transaction.
    pub deploy_tx_hash: String,
    /// Optional evidence reference for the deployment receipt artifact.
    pub deploy_receipt_evidence: Option<LifecycleArtifactEvidenceRef>,
    /// Optional block number that confirmed the deployment.
    pub deployed_block_number: Option<u64>,
}

/// Typestate value emitted after deployment configuration has been confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract",
    schema = "mfm.evm.contract.value.configured_contract"
)]
pub struct ConfiguredContract {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Deployment stage value consumed by the configure state.
    pub deployed: DeployedContract,
    /// Configuration calls requested against the deployed contract.
    pub configure_calls: Vec<ContractCallConfig>,
    /// Read confirmations required to prove the intended configuration is live on-chain.
    pub confirmation_read_assertions: Vec<ReadAssertionConfig>,
    /// Event confirmations required to prove the intended configuration was observed on-chain.
    pub confirmation_event_assertions: Vec<EventAssertionConfig>,
    /// Transaction hashes for configuration calls.
    pub configure_tx_hashes: Vec<String>,
    /// Evidence references for configuration receipt artifacts.
    pub configure_receipt_evidence: Vec<LifecycleArtifactEvidenceRef>,
    /// Optional highest block number that confirmed configuration.
    pub configured_block_number: Option<u64>,
}

/// Typed reference to a contract known to have completed configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-ref",
    schema = "mfm.evm.contract.value.configured_contract_ref"
)]
pub struct ConfiguredContractRef {
    /// Reference contract version.
    pub ref_version: u64,
    /// Stable network identifier for the configured contract.
    pub network_id: String,
    /// Expected chain id for the semantic network target.
    pub expected_chain_id: u64,
    /// Normalized configured contract address.
    pub contract_address: String,
    /// Optional evidence for the configured state.
    pub evidence: Option<LifecycleArtifactEvidenceRef>,
}

impl ConfiguredContractRef {
    /// Builds a typed reference from a configured contract lifecycle value.
    pub fn from_configured(configured: &ConfiguredContract) -> Self {
        Self {
            ref_version: 1,
            network_id: configured.deployed.network_id.clone(),
            expected_chain_id: configured.deployed.expected_chain_id,
            contract_address: configured.deployed.contract_address.clone(),
            evidence: configured.configure_receipt_evidence.last().cloned(),
        }
    }
}

/// Typed reference for validating a pre-existing configured deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "existing-configured-contract-ref",
    schema = "mfm.evm.contract.value.existing_configured_contract_ref"
)]
pub struct ExistingConfiguredContractRef {
    /// Reference contract version.
    pub ref_version: u64,
    /// Configured contract being imported into a validation-only flow.
    pub configured_contract: ConfiguredContractRef,
    /// Redaction-safe source label for the imported reference.
    pub source: String,
}

/// Result for one validation read assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-read-result",
    schema = "mfm.evm.contract.value.validation_read_result"
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
    namespace = "mfm.evm.contract",
    name = "validation-event-result",
    schema = "mfm.evm.contract.value.validation_event_result"
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
    namespace = "mfm.evm.contract",
    name = "validation-report",
    schema = "mfm.evm.contract.value.validation_report"
)]
pub struct ValidationReport {
    /// Validation report contract version.
    pub report_version: u64,
    /// Configured contract that was validated.
    pub configured_contract: ConfiguredContractRef,
    /// Expected chain id from validation config.
    pub expected_chain_id: u64,
    /// Observed chain id returned by the selected EVM source.
    pub observed_chain_id: u64,
    /// Redaction-safe client version returned by the selected EVM source.
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
    common_abi::parse_abi(&abi).map_err(|error| error.message)
}

/// Normalizes a hex string to lowercase `0x`-prefixed form.
pub fn normalize_hex_str(value: &str) -> Result<String, String> {
    common_hex::normalize_hex_str(value).map_err(|error| error.message)
}

/// Decodes a hex string into bytes.
pub fn hex_to_bytes(value: &str) -> Result<Vec<u8>, String> {
    common_hex::hex_to_bytes(value).map_err(|error| error.message)
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
    common_abi::resolve_function_call(abi, function_name, &args).map_err(|error| error.message)
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
    common_abi::constructor_data(abi, bytecode, &constructor_args).map_err(|error| error.message)
}

/// Parses a contract artifact into validated ABI and bytecode components.
pub fn parse_artifact(config: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&config.abi)?;
    let bytecode_json = config.bytecode.to_json_value()?;
    let bytecode = common_abi::parse_bytecode(&bytecode_json).map_err(|error| error.message)?;
    Ok((abi, bytecode))
}

fn block_selector_to_rpc_value(
    block: &Option<BlockSelector>,
    default_latest: bool,
) -> serde_json::Value {
    match block {
        Some(BlockSelector::Number { number }) => serde_json::json!(format!("0x{:x}", number)),
        Some(BlockSelector::Tag { tag }) => serde_json::json!(tag),
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
    for assertion in read_assertions {
        let (call_data, outputs) = resolve_function_call(abi, &assertion.function, &assertion.args)
            .map_err(|_| "read assertion did not match ABI".to_string())?;
        reads.push(PreparedReadAssertion {
            data_hex: bytes_to_hex_prefixed(&call_data),
            expected: assertion.expected.clone(),
            outputs,
        });
    }

    let mut events = Vec::with_capacity(event_assertions.len());
    for assertion in event_assertions {
        let event = abi
            .events
            .iter()
            .find(|event| event.name == assertion.event)
            .ok_or_else(|| "event assertion referenced unknown event".to_string())?;
        if event.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }
        let signature = format!("{}({})", event.name, event.inputs.join(","));
        let topic0 = keccak256(signature.as_bytes());
        let topic0_hex = bytes_to_hex_prefixed(topic0.as_slice());
        let from_block = block_selector_to_rpc_value(&assertion.from_block, false);
        let to_block = block_selector_to_rpc_value(&assertion.to_block, true);
        events.push(PreparedEventAssertion {
            event: assertion.event.clone(),
            topic0_hex,
            min_count: assertion.min_count,
            from_block,
            to_block,
        });
    }

    Ok((reads, events))
}

/// Normalizes an optional wei value into RPC hex quantity form.
pub fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, String> {
    common_abi::parse_value_wei_to_hex(value_wei).map_err(|error| error.message)
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
pub fn normalize_address(value: &str) -> Result<String, String> {
    encoding::normalize_address(value).map_err(|error| error.message)
}

/// Decodes a single-output EVM call response into JSON.
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
        (Value::String(actual), Value::String(expected)) => {
            if actual.starts_with("0x") && expected.starts_with("0x") {
                normalize_hex_str(actual).ok() == normalize_hex_str(expected).ok()
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
    use mfm_ids::{DigestAlgorithm, DigestBytes};
    use mfm_values::MfmValue;

    const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn digest() -> DigestBytes {
        DigestBytes::from_hex(DIGEST_HEX).expect("digest")
    }

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

        assert!(!source.contains(&["pub ", "json_text"].concat()));
        for forbidden in [
            ["impl ", "TryFrom", "<Value> for ", "Abi", "Json"].concat(),
            ["impl ", "TryFrom", "<Value> for ", "Bytecode", "Json"].concat(),
            [
                "impl ",
                "TryFrom",
                "<Value> for ",
                "Abi",
                "Argument",
                "Value",
            ]
            .concat(),
            ["impl ", "TryFrom", "<Value> for ", "Expected", "Value"].concat(),
            ["impl ", "From", "<Value> for ", "Abi", "Json"].concat(),
            ["impl ", "From", "<Value> for ", "Bytecode", "Json"].concat(),
            ["impl ", "From", "<Value> for ", "Abi", "Argument", "Value"].concat(),
            ["impl ", "From", "<Value> for ", "Expected", "Value"].concat(),
        ] {
            assert!(!source.contains(&forbidden), "found {forbidden}");
        }
    }

    #[test]
    fn json_wrappers_require_typed_deserialize_shape() {
        assert!(serde_json::from_value::<AbiJson>(serde_json::json!([
            {"type": "function", "name": "owner"}
        ]))
        .is_err());

        let typed = AbiJson::from_json_value(&serde_json::json!([
            {"type": "function", "name": "owner"}
        ]))
        .expect("typed abi");
        let serialized = serde_json::to_value(&typed).expect("serialized");
        let decoded: AbiJson = serde_json::from_value(serialized).expect("decoded");
        assert_eq!(decoded, typed);

        let abi = serde_json::from_value::<AbiJson>(serde_json::json!({
            "json_text": r#"[{"type":"function","name":"owner"}]"#
        }))
        .expect("typed wrapper");

        assert_eq!(abi.json_text(), r#"[{"name":"owner","type":"function"}]"#);
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
        let evidence = LifecycleArtifactEvidenceRef::new(
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
            128,
            None,
            None,
        );
        let deployed = DeployedContract {
            lifecycle_version: 1,
            network_id: "ethereum-mainnet".to_string(),
            expected_chain_id: 1,
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            deploy_tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            deploy_receipt_evidence: None,
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
            configure_receipt_evidence: vec![evidence.clone()],
            configured_block_number: Some(11),
        };

        let configured_ref = ConfiguredContractRef::from_configured(&configured);

        assert_eq!(configured_ref.network_id, configured.deployed.network_id);
        assert_eq!(
            configured_ref.contract_address,
            configured.deployed.contract_address
        );
        assert_eq!(configured_ref.evidence, Some(evidence));
    }

    #[test]
    fn validation_report_targets_configured_ref_not_raw_lifecycle_stage() {
        let report = ValidationReport {
            report_version: 1,
            configured_contract: ConfiguredContractRef {
                ref_version: 1,
                network_id: "ethereum-mainnet".to_string(),
                expected_chain_id: 1,
                contract_address: "0x1111111111111111111111111111111111111111".to_string(),
                evidence: None,
            },
            expected_chain_id: 1,
            observed_chain_id: 1,
            client_version: "reth-test".to_string(),
            configuration_read_results: Vec::new(),
            configuration_event_results: Vec::new(),
            read_results: Vec::new(),
            event_results: Vec::new(),
            valid: true,
        };

        assert!(report.valid);
        assert_eq!(report.configured_contract.expected_chain_id, 1);
    }

    #[test]
    fn json_wrappers_canonicalize_authored_values() {
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
    fn block_selector_uses_typed_shape_only() {
        let typed_number: BlockSelector =
            serde_json::from_value(serde_json::json!({"kind": "number", "number": 12}))
                .expect("selector");
        let typed_tag: BlockSelector =
            serde_json::from_value(serde_json::json!({"kind": "tag", "tag": "latest"}))
                .expect("selector");

        assert_eq!(typed_number, BlockSelector::Number { number: 12 });
        assert_eq!(
            typed_tag,
            BlockSelector::Tag {
                tag: "latest".to_string()
            }
        );
        assert!(serde_json::from_value::<BlockSelector>(serde_json::json!(12)).is_err());
    }

    #[test]
    fn lifecycle_values_have_contract_and_abi_schema_ids() {
        let schema_ids = [
            AbiJson::schema_id().expect("schema").to_string(),
            AbiArgumentValue::schema_id().expect("schema").to_string(),
            ContractArtifactConfig::schema_id()
                .expect("schema")
                .to_string(),
            DeployedContract::schema_id().expect("schema").to_string(),
            ConfiguredContract::schema_id().expect("schema").to_string(),
            ConfiguredContractRef::schema_id()
                .expect("schema")
                .to_string(),
            ExistingConfiguredContractRef::schema_id()
                .expect("schema")
                .to_string(),
            ValidationReport::schema_id().expect("schema").to_string(),
        ];

        assert!(schema_ids
            .iter()
            .all(|schema_id| schema_id.contains("mfm.evm.contract")
                || schema_id.contains("mfm.evm.abi")));
    }

    #[test]
    fn lifecycle_artifact_evidence_refs_round_trip_typed_ids() {
        let evidence = LifecycleArtifactEvidenceRef::new(
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
            128,
            Some(
                SchemaId::new(
                    "mfm.evm.contract.value.test",
                    "v1",
                    DigestAlgorithm::Sha256JcsV1,
                    digest(),
                )
                .expect("schema"),
            ),
            Some(
                SemanticTypeId::new(
                    "mfm.evm.contract",
                    "test",
                    "v1",
                    DigestAlgorithm::Sha256JcsV1,
                    digest(),
                )
                .expect("semantic"),
            ),
        );

        let json = serde_json::to_value(&evidence).expect("json");
        let decoded: LifecycleArtifactEvidenceRef =
            serde_json::from_value(json).expect("decoded evidence");

        assert_eq!(decoded, evidence);
    }
}
