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

/// Lifecycle stage required or produced by an import/transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "lifecycle-stage",
    schema = "mfm.evm.contract.value.lifecycle_stage"
)]
pub enum ContractLifecycleStage {
    /// Deployed contract instance stage.
    Deployed,
    /// Configured contract instance stage.
    Configured,
}

/// Source cell or public output selected for an MFM-run import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "source-cell-or-output-ref",
    schema = "mfm.evm.contract.value.source_cell_or_output_ref"
)]
pub enum SourceCellOrOutputRef {
    /// Source cell id inside the source run authority.
    Cell {
        /// Source cell id.
        cell_id: LifecycleCellIdRef,
    },
    /// Source terminal public-output binding key.
    PublicOutput {
        /// Source public output key.
        output_key: ArtifactPort,
    },
}

/// Certified policy for accepting a source-run context during import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "accepted-context-policy",
    schema = "mfm.evm.contract.value.accepted_context_policy"
)]
pub enum AcceptedContextPolicy {
    /// Source context must equal the importing context.
    ExactContext {},
    /// Source context must be one of these explicit context refs.
    AcceptedContextRefs {
        /// Accepted source context refs.
        context_refs: Vec<ContextRefValue>,
    },
}

impl Default for AcceptedContextPolicy {
    fn default() -> Self {
        Self::ExactContext {}
    }
}

/// Import request for a lifecycle value produced by another verified MFM run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "import-from-mfm-run",
    schema = "mfm.evm.contract.value.import_from_mfm_run"
)]
pub struct ImportFromMfmRun {
    /// Source run id.
    pub source_run_id: LifecycleRunIdRef,
    /// Source certified spec hash.
    pub source_spec_hash: LifecycleSpecHashRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Required source lifecycle stage.
    pub required_stage: ContractLifecycleStage,
    /// Certified context acceptance policy.
    pub accepted_context_policy: AcceptedContextPolicy,
}

impl<'de> Deserialize<'de> for ImportFromMfmRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawImportFromMfmRun {
            source_run_id: LifecycleRunIdRef,
            source_spec_hash: LifecycleSpecHashRef,
            source_cell_or_output_id: SourceCellOrOutputRef,
            source_value_digest: ContractProfileDigestRef,
            source_context_ref: ContextRefValue,
            required_stage: ContractLifecycleStage,
            #[serde(default)]
            accepted_context_policy: AcceptedContextPolicy,
        }

        let raw = RawImportFromMfmRun::deserialize(deserializer)?;
        Ok(Self {
            source_run_id: raw.source_run_id,
            source_spec_hash: raw.source_spec_hash,
            source_cell_or_output_id: raw.source_cell_or_output_id,
            source_value_digest: raw.source_value_digest,
            source_context_ref: raw.source_context_ref,
            required_stage: raw.required_stage,
            accepted_context_policy: raw.accepted_context_policy,
        })
    }
}

/// Replayable evidence for importing a lifecycle value from another MFM run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "import-from-mfm-run-evidence",
    schema = "mfm.evm.contract.value.import_from_mfm_run_evidence"
)]
pub struct ImportFromMfmRunEvidence {
    /// Source certified spec hash.
    pub source_spec_hash: LifecycleSpecHashRef,
    /// Retained source typed-spec artifact verified against the source stream admission event.
    pub source_spec_artifact_ref: LifecycleArtifactEvidenceRef,
    /// Retained source typed-spec certificate verified against the compiled registry.
    pub source_spec_certificate_ref: LifecycleArtifactEvidenceRef,
    /// Retained canonical committed source run stream.
    pub source_run_stream_ref: LifecycleArtifactEvidenceRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value schema id.
    pub source_cell_schema_id: ArtifactEvidenceSchemaId,
    /// Source value semantic type id.
    pub source_cell_semantic_type_id: ArtifactEvidenceSemanticTypeId,
    /// Source producer descriptor id.
    pub source_producer_descriptor_id: LifecycleDescriptorIdRef,
    /// Source lifecycle stage.
    pub source_stage: ContractLifecycleStage,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Source context descriptor id.
    pub source_context_descriptor_id: LifecycleContextDescriptorIdRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
    /// Retained source value artifact or inline canonical value evidence.
    pub source_value_artifact_ref_or_inline_canonical_value: LifecycleArtifactEvidenceRef,
    /// Source terminal cell or output event id.
    pub source_terminal_cell_or_output_event_ref: LifecycleEventIdRef,
    /// Certified import policy digest.
    pub import_policy_digest: ContractProfileDigestRef,
}

/// One terminal source-run cell or output event derived from a committed source stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "source-run-terminal-event",
    schema = "mfm.evm.contract.value.source_run_terminal_event"
)]
pub struct SourceRunTerminalEvent {
    /// Source terminal cell or output event id.
    pub source_terminal_cell_or_output_event_ref: LifecycleEventIdRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value schema id.
    pub source_cell_schema_id: ArtifactEvidenceSchemaId,
    /// Source value semantic type id.
    pub source_cell_semantic_type_id: ArtifactEvidenceSemanticTypeId,
    /// Source producer descriptor id.
    pub source_producer_descriptor_id: LifecycleDescriptorIdRef,
    /// Source lifecycle stage.
    pub source_stage: ContractLifecycleStage,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Source context descriptor id.
    pub source_context_descriptor_id: LifecycleContextDescriptorIdRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
}

/// Certified evidence policy for adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-adoption-evidence-policy",
    schema = "mfm.evm.contract.value.external_adoption_evidence_policy"
)]
pub struct ExternalAdoptionEvidencePolicy {
    /// Whether code must be observed at the adopted address.
    pub require_code: bool,
    /// Optional expected deployed code hash.
    pub expected_code_hash: Option<EvmCodeHash>,
    /// Optional observation block anchor.
    pub block_anchor: Option<BlockSelector>,
    /// Initial read assertions required before adoption.
    pub initial_read_assertions: Vec<ReadAssertionConfig>,
    /// Initial event assertions required before adoption.
    pub initial_event_assertions: Vec<EventAssertionConfig>,
    /// Whether an unverified configured claim is allowed.
    pub allow_external_claimed_configured: bool,
}

fn default_require_code() -> bool {
    true
}

impl Default for ExternalAdoptionEvidencePolicy {
    fn default() -> Self {
        Self {
            require_code: true,
            expected_code_hash: None,
            block_anchor: None,
            initial_read_assertions: Vec::new(),
            initial_event_assertions: Vec::new(),
            allow_external_claimed_configured: false,
        }
    }
}

impl<'de> Deserialize<'de> for ExternalAdoptionEvidencePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawExternalAdoptionEvidencePolicy {
            #[serde(default = "default_require_code")]
            require_code: bool,
            #[serde(default)]
            expected_code_hash: Option<EvmCodeHash>,
            #[serde(default)]
            block_anchor: Option<BlockSelector>,
            #[serde(default)]
            initial_read_assertions: Vec<ReadAssertionConfig>,
            #[serde(default)]
            initial_event_assertions: Vec<EventAssertionConfig>,
            #[serde(default)]
            allow_external_claimed_configured: bool,
        }

        let raw = RawExternalAdoptionEvidencePolicy::deserialize(deserializer)?;
        Ok(Self {
            require_code: raw.require_code,
            expected_code_hash: raw.expected_code_hash,
            block_anchor: raw.block_anchor,
            initial_read_assertions: raw.initial_read_assertions,
            initial_event_assertions: raw.initial_event_assertions,
            allow_external_claimed_configured: raw.allow_external_claimed_configured,
        })
    }
}

/// Request to adopt an external EVM address under a certified lifecycle context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "adopt-external-address",
    schema = "mfm.evm.contract.value.adopt_external_address"
)]
pub struct AdoptExternalAddress {
    /// Normalized address being adopted.
    pub address: ContractAddress,
    /// Redaction-safe provenance label.
    pub provenance_label: ProvenanceLabel,
    /// Certified evidence policy.
    pub evidence_policy: ExternalAdoptionEvidencePolicy,
}

impl<'de> Deserialize<'de> for AdoptExternalAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawAdoptExternalAddress {
            address: ContractAddress,
            provenance_label: ProvenanceLabel,
            #[serde(default)]
            evidence_policy: ExternalAdoptionEvidencePolicy,
        }

        let raw = RawAdoptExternalAddress::deserialize(deserializer)?;
        Ok(Self {
            address: raw.address,
            provenance_label: raw.provenance_label,
            evidence_policy: raw.evidence_policy,
        })
    }
}

/// Redacted EVM source evidence captured with an external adoption read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-evm-source-evidence",
    schema = "mfm.evm.contract.value.external_evm_source_evidence"
)]
pub struct ExternalEvmSourceEvidence {
    /// Semantic network id captured from the bound provider evidence.
    pub network_id: String,
    /// Expected EVM chain id captured from the certified network context.
    pub expected_chain_id: u64,
    /// Observed EVM chain id reported by the provider.
    pub observed_chain_id: u64,
    /// Redacted provider source reference.
    pub source_ref: String,
    /// Redacted provider source policy id.
    pub policy_id: String,
}

/// Replayable code-read evidence captured while adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-code-read-evidence",
    schema = "mfm.evm.contract.value.external_code_read_evidence"
)]
pub struct ExternalCodeReadEvidence {
    /// Address whose deployed bytecode was read.
    pub address: ContractAddress,
    /// Block selector used for the code read.
    pub block: BlockSelector,
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Observed bytecode hash.
    pub observed_code_hash: EvmCodeHash,
    /// Observed bytecode length.
    pub observed_code_byte_len: u64,
}

/// Replayable read-assertion evidence captured during external configured adoption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-read-assertion-evidence",
    schema = "mfm.evm.contract.value.external_read_assertion_evidence"
)]
pub struct ExternalReadAssertionEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationReadResult,
}

/// Replayable event-assertion evidence captured during external configured adoption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-event-assertion-evidence",
    schema = "mfm.evm.contract.value.external_event_assertion_evidence"
)]
pub struct ExternalEventAssertionEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationEventResult,
}

/// Replayable evidence captured while adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-adoption-evidence",
    schema = "mfm.evm.contract.value.external_adoption_evidence"
)]
pub struct ExternalAdoptionEvidence {
    /// Certified evidence policy digest.
    pub evidence_policy_digest: ContractProfileDigestRef,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Lifecycle stage admitted by this external adoption.
    pub resource_stage: ContractLifecycleStage,
    /// Observed EVM chain id.
    pub observed_chain_id: u64,
    /// Replayable code-read evidence when the policy required a code observation.
    pub code_read_evidence: Option<ExternalCodeReadEvidence>,
    /// Replayable read assertion evidence.
    pub read_assertion_evidence: Vec<ExternalReadAssertionEvidence>,
    /// Replayable event assertion evidence.
    pub event_assertion_evidence: Vec<ExternalEventAssertionEvidence>,
}

/// Claim describing why an instance is considered configured.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configuration-claim",
    schema = "mfm.evm.contract.value.configuration_claim"
)]
pub enum ConfigurationClaim {
    /// Configuration was performed by an MFM configure transition.
    MfmConfigured {
        /// Configure node id.
        configure_node: LifecycleNodeIdRef,
        /// Digest of the configure action.
        configure_action_digest: ContractProfileDigestRef,
        /// Configuration call evidence refs.
        call_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
        /// Confirmation read/event evidence refs.
        confirmation_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    },
    /// Configuration provenance was imported from another verified MFM run.
    ImportedMfmConfigured {
        /// Source run id.
        source_run_id: LifecycleRunIdRef,
        /// Source spec hash.
        source_spec_hash: LifecycleSpecHashRef,
        /// Source cell or output id.
        source_cell_or_output_id: SourceCellOrOutputRef,
        /// Source value digest.
        source_value_digest: ContractProfileDigestRef,
        /// Source context ref.
        source_context_ref: ContextRefValue,
    },
    /// External adoption observed configured-state evidence.
    ExternalObservedConfigured {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest.
        evidence_policy_digest: ContractProfileDigestRef,
        /// Digest of the inline external adoption evidence that supports the observation.
        external_adoption_evidence_digest: ContractProfileDigestRef,
    },
    /// External adoption made an explicitly unverified configured claim.
    ExternalClaimedConfigured {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest permitting this claim.
        evidence_policy_digest: ContractProfileDigestRef,
    },
}

impl ConfigurationClaim {
    /// Returns true when this claim proves MFM configuration provenance.
    pub const fn proves_mfm_configuration(&self) -> bool {
        matches!(
            self,
            Self::MfmConfigured { .. } | Self::ImportedMfmConfigured { .. }
        )
    }
}

/// Provenance for a deployed contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-provenance",
    schema = "mfm.evm.contract.value.deploy_provenance"
)]
pub enum DeployProvenance {
    /// Contract was deployed by an MFM deploy transition.
    MfmDeploy {
        /// Deployment transaction hash.
        deploy_tx_hash: String,
    },
    /// Deployed instance was imported from another verified MFM run.
    ImportedMfmRun {
        /// Imported source context ref.
        source_context_ref: ContextRefValue,
        /// Imported source value digest.
        source_value_digest: ContractProfileDigestRef,
        /// Certified import policy digest.
        import_policy_digest: ContractProfileDigestRef,
    },
    /// Deployed instance was externally adopted.
    ExternalAdoption {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest.
        evidence_policy_digest: ContractProfileDigestRef,
    },
}

/// Context-bound deployed contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deployed-contract-instance",
    schema = "mfm.evm.contract.value.deployed_contract_instance"
)]
pub struct DeployedContractInstance {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Deployment or import provenance.
    pub deploy_provenance: DeployProvenance,
    /// Deployment or import evidence refs.
    pub deploy_evidence: Vec<LifecycleArtifactEvidenceRef>,
    /// Replayable external-adoption evidence when this instance was adopted externally.
    pub external_adoption_evidence: Option<ExternalAdoptionEvidence>,
    /// Optional block number that confirmed deployment or adoption.
    pub deployed_block_number: Option<u64>,
}

/// Configured instance lineage back to a same-context deployed address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-from",
    schema = "mfm.evm.contract.value.configured_from"
)]
pub struct ConfiguredFrom {
    /// Deployed instance context ref.
    pub deployed_context_ref: ContextRefValue,
    /// Deployed instance address.
    pub deployed_address: ContractAddress,
}

/// Optional configured-state evidence snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configuration-snapshot",
    schema = "mfm.evm.contract.value.configuration_snapshot"
)]
pub struct ConfigurationSnapshot {
    /// Read assertion results proving configured state.
    pub read_results: Vec<ValidationReadResult>,
    /// Event assertion results proving configured state.
    pub event_results: Vec<ValidationEventResult>,
}

/// Context-bound configured contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance",
    schema = "mfm.evm.contract.value.configured_contract_instance"
)]
pub struct ConfiguredContractInstance {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Deployed instance identity that configuration builds from.
    pub configured_from: ConfiguredFrom,
    /// Honest configuration provenance claim.
    pub configuration_claim: ConfigurationClaim,
    /// Configuration or import evidence refs.
    pub configure_or_import_evidence: Vec<LifecycleArtifactEvidenceRef>,
    /// Replayable external-adoption evidence when this instance was adopted externally.
    pub external_adoption_evidence: Option<ExternalAdoptionEvidence>,
    /// Optional highest block number that confirmed configuration or adoption.
    pub configured_block_number: Option<u64>,
    /// Optional configured-state assertion snapshot.
    pub asserted_configuration_snapshot: Option<ConfigurationSnapshot>,
}

/// Configured instance identity consumed by validation reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance-ref",
    schema = "mfm.evm.contract.value.configured_contract_instance_ref"
)]
pub struct ConfiguredContractInstanceRef {
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized configured contract address.
    pub address: ContractAddress,
    /// Honest configuration provenance claim.
    pub configuration_claim: ConfigurationClaim,
}

impl ConfiguredContractInstanceRef {
    /// Builds a configured instance reference from a configured instance.
    pub fn from_configured(configured: &ConfiguredContractInstance) -> Self {
        Self {
            context_ref: configured.context_ref.clone(),
            address: configured.address.clone(),
            configuration_claim: configured.configuration_claim.clone(),
        }
    }
}

/// Context-bound terminal validation report for a configured contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-bound-validation-report",
    schema = "mfm.evm.contract.value.context_bound_validation_report"
)]
pub struct ContextBoundValidationReport {
    /// Validation report contract version.
    pub report_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Configured contract instance that was validated.
    pub configured_instance: ConfiguredContractInstanceRef,
    /// Observed EVM chain id from validation read evidence.
    pub observed_chain_id: u64,
    /// Results for configuration-intent read confirmations stored on the configured instance.
    pub configuration_read_results: Vec<ValidationReadResult>,
    /// Results for configuration-intent event confirmations stored on the configured instance.
    pub configuration_event_results: Vec<ValidationEventResult>,
    /// Additional read assertion results from validation action.
    pub read_results: Vec<ValidationReadResult>,
    /// Additional event assertion results from validation action.
    pub event_results: Vec<ValidationEventResult>,
    /// Replayable read assertion evidence from validation reads.
    pub validation_read_evidence: Vec<ExternalReadAssertionEvidence>,
    /// Replayable event assertion evidence from validation log reads.
    pub validation_event_evidence: Vec<ExternalEventAssertionEvidence>,
    /// Retained lifecycle evidence refs that support the configured input being validated.
    pub evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Whether all validation checks passed.
    pub valid: bool,
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
