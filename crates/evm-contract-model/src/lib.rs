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

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use alloy_primitives::keccak256;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;
use mfm_evm_core::tx::parse_u128_quantity;
use mfm_ids::{
    ArtifactId, ContentDigest, LocalPublicId, SchemaId, SemanticTypeId, StableAuthorKey,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

/// Error returned when constructing EVM contract scalar authorities.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvmContractScalarError {
    /// A string scalar was empty or whitespace-only.
    #[error("{kind} must be non-empty")]
    Empty {
        /// Human-readable scalar kind.
        kind: &'static str,
    },
    /// A wei quantity failed canonical quantity parsing.
    #[error("{kind} must be a valid EVM quantity: {message}")]
    InvalidQuantity {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
    /// A typed identity failed category-specific parsing.
    #[error("{kind} must be a valid typed identity: {message}")]
    InvalidIdentity {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
    /// A checked string failed its shared grammar.
    #[error("{kind} must be a valid checked string: {message}")]
    InvalidString {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
}

fn require_non_empty(kind: &'static str, value: &str) -> Result<(), EvmContractScalarError> {
    if value.trim().is_empty() {
        Err(EvmContractScalarError::Empty { kind })
    } else {
        Ok(())
    }
}

macro_rules! evm_string_scalar {
    ($ty:ident, $validator:ident, $kind:literal, $semantic:literal, $schema:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            MfmValue,
        )]
        #[serde(try_from = "String", into = "String")]
        #[mfm(namespace = "mfm.evm.contract", name = $semantic, schema = $schema, transparent_string)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            /// Creates a checked EVM contract scalar authority.
            pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
                let raw = value.into();
                $validator($kind, &raw)?;
                Ok(Self { raw })
            }

            /// Returns the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Consumes this authority into its canonical string representation.
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = EvmContractScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = EvmContractScalarError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}

fn require_stable_author_key(
    kind: &'static str,
    value: &str,
) -> Result<(), EvmContractScalarError> {
    StableAuthorKey::new(value)
        .map(|_| ())
        .map_err(|error| EvmContractScalarError::InvalidString {
            kind,
            message: error.to_string(),
        })
}

fn require_local_public_id(kind: &'static str, value: &str) -> Result<(), EvmContractScalarError> {
    LocalPublicId::new(value)
        .map(|_| ())
        .map_err(|error| EvmContractScalarError::InvalidString {
            kind,
            message: error.to_string(),
        })
}

evm_string_scalar!(
    EvmNetworkId,
    require_stable_author_key,
    "network_id",
    "network-id",
    "mfm.evm.contract.id.network",
    "Stable typed EVM network identifier."
);

evm_string_scalar!(
    FunctionName,
    require_non_empty,
    "function",
    "function-name",
    "mfm.evm.contract.id.function",
    "Checked EVM ABI function name."
);

evm_string_scalar!(
    EventName,
    require_non_empty,
    "event",
    "event-name",
    "mfm.evm.contract.id.event",
    "Checked EVM ABI event name."
);

evm_string_scalar!(
    ArtifactPort,
    require_local_public_id,
    "artifact_port",
    "artifact-port",
    "mfm.evm.contract.id.artifact_port",
    "Checked artifact context port name."
);

macro_rules! evidence_identity_scalar {
    (
        $ty:ident,
        $target:ty,
        $kind:literal,
        $semantic:literal,
        $schema:literal,
        $doc:literal
    ) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            MfmValue,
        )]
        #[serde(try_from = "String", into = "String")]
        #[mfm(namespace = "mfm.evm.contract", name = $semantic, schema = $schema, transparent_string)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            /// Creates a checked evidence identity reference.
            pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
                let raw = value.into();
                require_non_empty($kind, &raw)?;
                <$target>::parse(&raw).map_err(|error| EvmContractScalarError::InvalidIdentity {
                    kind: $kind,
                    message: error.to_string(),
                })?;
                Ok(Self { raw })
            }

            /// Returns the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Parses this checked reference into the corresponding kernel identity.
            pub fn typed(&self) -> Result<$target, String> {
                <$target>::parse(&self.raw).map_err(|error| error.to_string())
            }

            /// Consumes this authority into its canonical string representation.
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = EvmContractScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = EvmContractScalarError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$target> for $ty {
            fn from(value: $target) -> Self {
                Self {
                    raw: value.to_string(),
                }
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}

evidence_identity_scalar!(
    ArtifactEvidenceArtifactId,
    ArtifactId,
    "artifact_id",
    "artifact-evidence-artifact-id",
    "mfm.evm.contract.id.artifact_evidence_artifact",
    "Checked artifact id reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceContentDigest,
    ContentDigest,
    "content_digest",
    "artifact-evidence-content-digest",
    "mfm.evm.contract.id.artifact_evidence_content_digest",
    "Checked content digest reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceSchemaId,
    SchemaId,
    "schema_id",
    "artifact-evidence-schema-id",
    "mfm.evm.contract.id.artifact_evidence_schema",
    "Checked schema id reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceSemanticTypeId,
    SemanticTypeId,
    "semantic_type_id",
    "artifact-evidence-semantic-type-id",
    "mfm.evm.contract.id.artifact_evidence_semantic_type",
    "Checked semantic type id reference carried by lifecycle evidence."
);

/// Checked wei quantity rendered in the authored EVM quantity format.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "wei-amount",
    schema = "mfm.evm.contract.value.wei_amount",
    transparent_string
)]
pub struct WeiAmount {
    raw: String,
}

impl WeiAmount {
    /// Creates a checked wei amount.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
        let raw = value.into();
        parse_u128_quantity(&raw, "wei").map_err(|error| {
            EvmContractScalarError::InvalidQuantity {
                kind: "wei",
                message: error.message,
            }
        })?;
        Ok(Self { raw })
    }

    /// Returns the canonical string representation.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its canonical string representation.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for WeiAmount {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for WeiAmount {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for WeiAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WeiAmount {
    type Err = EvmContractScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for WeiAmount {
    type Error = EvmContractScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<WeiAmount> for String {
    fn from(value: WeiAmount) -> Self {
        value.raw
    }
}

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
#[mfm(
    namespace = "mfm.evm.contract",
    name = "artifact-evidence-ref",
    schema = "mfm.evm.contract.value.artifact_evidence_ref"
)]
pub struct LifecycleArtifactEvidenceRef {
    artifact_id: ArtifactEvidenceArtifactId,
    content_digest: ArtifactEvidenceContentDigest,
    byte_len: u64,
    schema_id: Option<ArtifactEvidenceSchemaId>,
    semantic_type_id: Option<ArtifactEvidenceSemanticTypeId>,
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
            artifact_id: artifact_id.into(),
            content_digest: content_digest.into(),
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
        #[serde(tag = "kind", rename_all = "snake_case")]
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
        Some(BlockSelector::Tag { tag }) => serde_json::json!(tag.as_str()),
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
        let (call_data, outputs) =
            resolve_function_call(abi, assertion.function.as_str(), &assertion.args)
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
            .find(|event| event.name == assertion.event.as_str())
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
            event: assertion.event.to_string(),
            topic0_hex,
            min_count: assertion.min_count,
            from_block,
            to_block,
        });
    }

    Ok((reads, events))
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
        (Value::String(actual), Value::String(expected))
            if actual.starts_with("0x") && expected.starts_with("0x") =>
        {
            normalize_hex_str(actual).ok() == normalize_hex_str(expected).ok()
        }
        _ => false,
    }
}
