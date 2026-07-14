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
//!     prepare_validate_assertions, AbiJson, BlockSelector, BlockTag, EventAssertionConfig,
//!     EventName, ExpectedValue, FunctionName, ReadAssertionConfig,
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
//!         function: FunctionName::new("owner").map_err(|error| error.to_string())?,
//!         args: vec![],
//!         expected: ExpectedValue::from_json_value(&serde_json::json!(
//!             "0x0000000000000000000000000000000000000000"
//!         ))?,
//!     }],
//!     &[EventAssertionConfig {
//!         event: EventName::new("Configured").map_err(|error| error.to_string())?,
//!         min_count: 1,
//!         from_block: Some(BlockSelector::Number { number: 0 }),
//!         to_block: Some(BlockSelector::Tag {
//!             tag: BlockTag::Latest,
//!         }),
//!     }],
//! )?;
//!
//! assert_eq!(reads.len(), 1);
//! assert_eq!(events[0].event, "Configured");
//! assert!(events[0].topic0_hex.starts_with("0x"));
//! # Ok::<(), String>(())
//! ```

use std::sync::OnceLock;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, ContextDescriptorId, ContextRef, ContextResourceKind,
    ContextStage, DescriptorId, EventId, NodeId, RunId, SchemaId, SemanticTypeId, SpecHash,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_values::{ContextBoundOutput, ContextRefValue};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

#[path = "scalars.rs"]
mod scalars;
pub use self::scalars::*;

#[path = "context.rs"]
mod context;
pub use self::context::{
    ContractProfile, EvmContractContext, EvmNetworkContext, FinalityOrObservationPolicy,
};

#[path = "lifecycle.rs"]
mod lifecycle;
pub use self::lifecycle::*;

#[path = "abi.rs"]
mod abi;
pub use self::abi::{
    constructor_data, decode_single_output_to_json, expected_matches, parse_abi, parse_artifact,
    prepare_validate_assertions, resolve_function_call,
};

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "artifact-evidence-ref",
    schema = "mfm.evm.contract.value.artifact_evidence_ref"
)]
pub struct LifecycleArtifactEvidenceRef {
    artifact_id: ArtifactEvidenceArtifactId,
    content_digest: ArtifactEvidenceContentDigest,
    /// Exact retained-artifact evidence identity used by store/read authority.
    evidence_hash: ArtifactEvidenceEvidenceHash,
    byte_len: u64,
    schema_id: Option<ArtifactEvidenceSchemaId>,
    semantic_type_id: Option<ArtifactEvidenceSemanticTypeId>,
}

impl LifecycleArtifactEvidenceRef {
    /// Creates a typed lifecycle artifact evidence reference.
    pub fn new(
        artifact_id: ArtifactId,
        content_digest: ContentDigest,
        evidence_hash: ContentDigest,
        byte_len: u64,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<SemanticTypeId>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            content_digest: content_digest.into(),
            evidence_hash: evidence_hash.into(),
            byte_len,
            schema_id: schema_id.map(Into::into),
            semantic_type_id: semantic_type_id.map(Into::into),
        }
    }

    /// Returns the artifact id string.
    pub fn artifact_id_str(&self) -> &str {
        self.artifact_id.as_str()
    }

    /// Parses and returns the typed artifact id.
    pub fn artifact_id(&self) -> Result<ArtifactId, String> {
        self.artifact_id.typed()
    }

    /// Returns the content digest string.
    pub fn content_digest_str(&self) -> &str {
        self.content_digest.as_str()
    }

    /// Parses and returns the typed content digest.
    pub fn content_digest(&self) -> Result<ContentDigest, String> {
        self.content_digest.typed()
    }

    /// Returns the exact retained-artifact evidence hash string.
    pub fn evidence_hash_str(&self) -> &str {
        self.evidence_hash.as_str()
    }

    /// Parses and returns the typed exact retained-artifact evidence hash.
    pub fn evidence_hash(&self) -> Result<ContentDigest, String> {
        self.evidence_hash.typed()
    }

    /// Returns the artifact byte length.
    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Returns the optional schema id string.
    pub fn schema_id_str(&self) -> Option<&str> {
        self.schema_id
            .as_ref()
            .map(ArtifactEvidenceSchemaId::as_str)
    }

    /// Parses and returns the optional typed schema id.
    pub fn schema_id(&self) -> Result<Option<SchemaId>, String> {
        self.schema_id
            .as_ref()
            .map(ArtifactEvidenceSchemaId::typed)
            .transpose()
    }

    /// Returns the optional semantic type id string.
    pub fn semantic_type_id_str(&self) -> Option<&str> {
        self.semantic_type_id
            .as_ref()
            .map(ArtifactEvidenceSemanticTypeId::as_str)
    }

    /// Parses and returns the optional typed semantic type id.
    pub fn semantic_type_id(&self) -> Result<Option<SemanticTypeId>, String> {
        self.semantic_type_id
            .as_ref()
            .map(ArtifactEvidenceSemanticTypeId::typed)
            .transpose()
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
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "call-config",
    schema = "mfm.evm.contract.config.call"
)]
pub struct ContractCallConfig {
    /// Function name to invoke.
    pub function: FunctionName,

    /// Positional arguments passed to the function call.
    #[serde(default)]
    pub args: Vec<AbiArgumentValue>,

    /// Optional call value expressed in wei.
    #[serde(default)]
    pub value_wei: Option<WeiAmount>,
}

fn default_min_count() -> u64 {
    1
}

/// Symbolic EVM block tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "block-tag",
    schema = "mfm.evm.contract.value.block_tag"
)]
pub enum BlockTag {
    /// Earliest available block.
    Earliest,
    /// Latest available block.
    Latest,
    /// Pending block.
    Pending,
    /// Safe block tag when supported by the provider.
    Safe,
    /// Finalized block tag when supported by the provider.
    Finalized,
}

impl BlockTag {
    /// Returns the canonical JSON-RPC tag string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Earliest => "earliest",
            Self::Latest => "latest",
            Self::Pending => "pending",
            Self::Safe => "safe",
            Self::Finalized => "finalized",
        }
    }
}

/// Block selector used by validation reads and event queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
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
        tag: BlockTag,
    },
}

impl<'de> Deserialize<'de> for BlockSelector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum RawBlockSelector {
            Number { number: u64 },
            Tag { tag: BlockTag },
        }

        match RawBlockSelector::deserialize(deserializer)? {
            RawBlockSelector::Number { number } => Ok(Self::Number { number }),
            RawBlockSelector::Tag { tag } => Ok(Self::Tag { tag }),
        }
    }
}

/// Read assertion to evaluate through an EVM call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "read-assertion-config",
    schema = "mfm.evm.contract.config.read_assertion"
)]
pub struct ReadAssertionConfig {
    /// Function name to call.
    pub function: FunctionName,

    /// Positional arguments passed to the function call.
    #[serde(default)]
    pub args: Vec<AbiArgumentValue>,

    /// Expected decoded value.
    pub expected: ExpectedValue,
}

/// Event assertion to evaluate with an EVM log query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "event-assertion-config",
    schema = "mfm.evm.contract.config.event_assertion"
)]
pub struct EventAssertionConfig {
    /// Event name expected in the ABI.
    pub event: EventName,

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
        #[serde(deny_unknown_fields)]
        struct RawEventAssertionConfig {
            event: EventName,
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

/// Returns the certified resource kind for EVM contract instances.
pub fn contract_instance_resource_kind() -> &'static ContextResourceKind {
    static KIND: OnceLock<ContextResourceKind> = OnceLock::new();
    KIND.get_or_init(|| {
        ContextResourceKind::new("mfm.evm.contract.instance")
            .expect("hardcoded context resource kind is valid")
    })
}

/// Returns the certified resource kind for terminal EVM validation reports.
pub fn validation_report_resource_kind() -> &'static ContextResourceKind {
    static KIND: OnceLock<ContextResourceKind> = OnceLock::new();
    KIND.get_or_init(|| {
        ContextResourceKind::new("mfm.evm.contract.validation_report")
            .expect("hardcoded context resource kind is valid")
    })
}

/// Returns the certified lifecycle stage for deployed instances.
pub fn deployed_contract_stage() -> &'static ContextStage {
    static STAGE: OnceLock<ContextStage> = OnceLock::new();
    STAGE.get_or_init(|| ContextStage::new("deployed").expect("hardcoded context stage is valid"))
}

/// Returns the certified lifecycle stage for configured instances.
pub fn configured_contract_stage() -> &'static ContextStage {
    static STAGE: OnceLock<ContextStage> = OnceLock::new();
    STAGE.get_or_init(|| ContextStage::new("configured").expect("hardcoded context stage is valid"))
}

/// Returns the certified terminal stage for validation reports.
pub fn validation_report_stage() -> &'static ContextStage {
    static STAGE: OnceLock<ContextStage> = OnceLock::new();
    STAGE.get_or_init(|| ContextStage::new("validated").expect("hardcoded context stage is valid"))
}

impl ContextBoundOutput for DeployedContractInstance {
    fn context_ref(&self) -> &ContextRef {
        self.context_ref.as_context_ref()
    }

    fn context_resource_kind(&self) -> &ContextResourceKind {
        contract_instance_resource_kind()
    }

    fn context_stage(&self) -> &ContextStage {
        deployed_contract_stage()
    }
}

impl ContextBoundOutput for ConfiguredContractInstance {
    fn context_ref(&self) -> &ContextRef {
        self.context_ref.as_context_ref()
    }

    fn context_resource_kind(&self) -> &ContextResourceKind {
        contract_instance_resource_kind()
    }

    fn context_stage(&self) -> &ContextStage {
        configured_contract_stage()
    }
}

impl ContextBoundOutput for ContextBoundValidationReport {
    fn context_ref(&self) -> &ContextRef {
        self.context_ref.as_context_ref()
    }

    fn context_resource_kind(&self) -> &ContextResourceKind {
        validation_report_resource_kind()
    }

    fn context_stage(&self) -> &ContextStage {
        validation_report_stage()
    }
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
