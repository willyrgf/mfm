use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::OnceLock;

use mfm_ids::{
    AdmissionLogicalKey, ArtifactId, AttemptId, ContentDigest, ContentRef,
    CorrectionInvocationDigest, DigestAlgorithm, EffectKey, ExecutorFrontierDigest,
    ExecutorRecordDigest, FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest,
    GenesisDigest, JournalCandidateDigest, JournalCommitDigest, JournalRecordHash, NodeId,
    ObjectEvidenceDigest, OutputLogicalIdentityDigest, RecordId, RequestDigest, RunId,
    RunSemanticStateDigest, SchemaId, SemanticDigest, SourceClosureDigest, SpecHash,
    TerminalEffectEvidenceDigest,
};
use serde_json::{Map, Value};

use crate::{sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes};

const ANNEX_BYTES: &[u8] = include_bytes!("../../../../contracts/recoverability/v1/annex.json");
const MAX_ANNEX_BYTES: usize = 16_777_216;
const MAX_SCHEMA_DEPTH: usize = 256;

static EMBEDDED_CONTRACT: OnceLock<
    std::result::Result<RecoverabilityContractV1, RecoverabilityError>,
> = OnceLock::new();

/// Stable machine-readable failure code for the recoverability-v1 codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoverabilityErrorCode {
    /// The embedded annex is not canonical or violates its closed meta-contract.
    InvalidAnnex,
    /// Input is not valid float-free canonical JSON.
    InvalidCanonicalJson,
    /// A requested schema contract is not registered by the annex.
    UnknownSchema,
    /// A requested semantic domain is not registered by the annex.
    UnknownDomain,
    /// A semantic domain was paired with the wrong validated preimage schema.
    SchemaMismatch,
    /// A required object field is absent.
    MissingField,
    /// An object contains a field not admitted by its closed schema.
    UnknownField,
    /// A value has the wrong JSON type for its schema.
    WrongType,
    /// A value violates its closed grammar, literal, enum, or identity contract.
    InvalidValue,
    /// A bounded value or collection is outside its admitted range.
    OutOfBounds,
    /// A collection required to be unique contains a duplicate.
    DuplicateItem,
    /// A collection does not use its schema-selected canonical order.
    InvalidOrder,
    /// A checked identity could not be constructed from annex-derived material.
    IdentityConstruction,
}

impl RecoverabilityErrorCode {
    /// Returns the stable persisted spelling used by the conformance corpus.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidAnnex => "invalid_annex",
            Self::InvalidCanonicalJson => "invalid_canonical_json",
            Self::UnknownSchema => "unknown_schema",
            Self::UnknownDomain => "unknown_domain",
            Self::SchemaMismatch => "schema_mismatch",
            Self::MissingField => "missing_field",
            Self::UnknownField => "unknown_field",
            Self::WrongType => "wrong_type",
            Self::InvalidValue => "invalid_value",
            Self::OutOfBounds => "out_of_bounds",
            Self::DuplicateItem => "duplicate_item",
            Self::InvalidOrder => "invalid_order",
            Self::IdentityConstruction => "identity_construction",
        }
    }
}

impl fmt::Display for RecoverabilityErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Redaction-safe error returned by the recoverability-v1 contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct RecoverabilityError {
    code: RecoverabilityErrorCode,
    message: String,
}

impl RecoverabilityError {
    fn new(code: RecoverabilityErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable error code.
    pub const fn code(&self) -> RecoverabilityErrorCode {
        self.code
    }

    /// Returns a reviewed diagnostic that never includes rejected input bytes.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// One schema-checked canonical value from the recoverability-v1 annex.
///
/// Construction is private to [`RecoverabilityContractV1`], so callers cannot
/// pair claimed bytes with an unrelated schema identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCanonicalValueV1 {
    schema_contract: String,
    schema_id: SchemaId,
    canonical: PlainCanonicalJsonBytes,
    value: Value,
}

impl ValidatedCanonicalValueV1 {
    /// Returns the exact registered schema contract.
    pub fn schema_contract(&self) -> &str {
        &self.schema_contract
    }

    /// Returns the annex-derived schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the exact canonical JSON bytes that passed schema validation.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }

    /// Returns the exact canonical JSON text that passed schema validation.
    pub fn as_str(&self) -> &str {
        self.canonical.as_str()
    }
}

/// Opaque canonical location of one projected schema reference.
///
/// The path uses RFC 6901 JSON Pointer syntax. Construction remains private to
/// [`RecoverabilityContractV1`] so callers cannot pair an arbitrary path with a
/// validated reference value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalReferencePathV1 {
    pointer: String,
}

impl CanonicalReferencePathV1 {
    /// Returns the canonical RFC 6901 JSON Pointer.
    ///
    /// The empty string identifies the root value.
    pub fn as_str(&self) -> &str {
        &self.pointer
    }
}

/// Closed terminal authority kind reached through an annex schema reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceTerminalKindV1 {
    /// Lightweight content identity.
    ContentRef,
    /// Producer-bound retained-value authority.
    ValueRef,
}

impl ReferenceTerminalKindV1 {
    fn schema_contract(self) -> &'static str {
        match self {
            Self::ContentRef => "mfm.content-ref.v1",
            Self::ValueRef => "mfm.value-ref.v1",
        }
    }
}

/// One terminal schema-reference edge projected from an annex-validated value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaReferenceEdgeV1 {
    path: CanonicalReferencePathV1,
    declared_contract: String,
    terminal_kind: ReferenceTerminalKindV1,
    value: ValidatedCanonicalValueV1,
}

impl SchemaReferenceEdgeV1 {
    /// Returns the opaque canonical location of the encountered reference.
    pub const fn path(&self) -> &CanonicalReferencePathV1 {
        &self.path
    }

    /// Returns the exact outer reference contract declared by the annex shape.
    pub fn declared_contract(&self) -> &str {
        &self.declared_contract
    }

    /// Returns the resolved closed terminal authority kind.
    pub const fn terminal_kind(&self) -> ReferenceTerminalKindV1 {
        self.terminal_kind
    }

    /// Returns the referenced value revalidated under its resolved terminal schema.
    pub const fn value(&self) -> &ValidatedCanonicalValueV1 {
        &self.value
    }
}

#[derive(Debug, Clone)]
struct SchemaDefinition {
    schema_id: SchemaId,
    shape: Value,
}

#[derive(Debug, Clone)]
struct DomainDefinition {
    preimage_schema: String,
    result_kind: String,
}

macro_rules! branded_domain_derivation {
    ($method:ident, $domain:literal, $result_kind:literal, $identity:ty, $doc:literal) => {
        #[doc = $doc]
        pub fn $method(
            &self,
            value: &ValidatedCanonicalValueV1,
        ) -> std::result::Result<$identity, RecoverabilityError> {
            self.semantic_digest_for($domain, $result_kind, value)
                .map(<$identity>::from_semantic_digest)
        }
    };
}

macro_rules! digest_only_domain_derivation {
    ($method:ident, $domain:literal, $result_kind:literal, $identity:ty, $doc:literal) => {
        #[doc = $doc]
        pub fn $method(
            &self,
            value: &ValidatedCanonicalValueV1,
        ) -> std::result::Result<$identity, RecoverabilityError> {
            self.semantic_digest_for($domain, $result_kind, value)
                .map(|digest| {
                    <$identity>::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest())
                })
        }
    };
}

/// Embedded, closed recoverability-v1 schema, codec, and digest authority.
#[derive(Debug)]
pub struct RecoverabilityContractV1 {
    annex: PlainCanonicalJsonBytes,
    schemas: BTreeMap<String, SchemaDefinition>,
    schema_contracts_by_id: BTreeMap<SchemaId, String>,
    domains: BTreeMap<String, DomainDefinition>,
    domain_envelope_schema: String,
    maximum_canonical_bytes: usize,
}

impl RecoverabilityContractV1 {
    /// Loads and validates the embedded annex once for the process.
    pub fn embedded() -> std::result::Result<&'static Self, RecoverabilityError> {
        match EMBEDDED_CONTRACT.get_or_init(|| Self::from_annex_bytes(ANNEX_BYTES)) {
            Ok(contract) => Ok(contract),
            Err(error) => Err(error.clone()),
        }
    }

    /// Validates candidate annex bytes without selecting them as process authority.
    ///
    /// This conformance operation exists for artifact producers and tests. The
    /// only contract returned for runtime use remains [`Self::embedded`].
    pub fn validate_annex_candidate(bytes: &[u8]) -> std::result::Result<(), RecoverabilityError> {
        Self::from_annex_bytes(bytes).map(drop)
    }

    /// Returns the exact embedded annex bytes after successful validation.
    pub fn annex_bytes(&self) -> &[u8] {
        self.annex.as_bytes()
    }

    /// Returns the registered schema identity for `schema_contract`.
    pub fn schema_id(
        &self,
        schema_contract: &str,
    ) -> std::result::Result<&SchemaId, RecoverabilityError> {
        self.schemas
            .get(schema_contract)
            .map(|schema| &schema.schema_id)
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::UnknownSchema,
                    "schema contract is not registered",
                )
            })
    }

    /// Returns the exact registered schema contract for `schema_id`.
    pub fn schema_contract_for_id(
        &self,
        schema_id: &SchemaId,
    ) -> std::result::Result<&str, RecoverabilityError> {
        self.schema_contracts_by_id
            .get(schema_id)
            .map(String::as_str)
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::UnknownSchema,
                    "schema identity is not registered",
                )
            })
    }

    /// Returns the preimage schema and result kind registered for a domain.
    pub fn domain_contract(
        &self,
        domain: &str,
    ) -> std::result::Result<(&str, &str), RecoverabilityError> {
        let definition = self.domains.get(domain).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::UnknownDomain,
                "semantic domain is not registered",
            )
        })?;
        Ok((&definition.preimage_schema, &definition.result_kind))
    }

    /// Strictly decodes already-canonical JSON under a registered annex schema.
    ///
    /// This operation rejects noncanonical key order rather than silently
    /// normalizing persisted input.
    pub fn strict_decode(
        &self,
        schema_contract: &str,
        bytes: &[u8],
    ) -> std::result::Result<ValidatedCanonicalValueV1, RecoverabilityError> {
        if bytes.len() > self.maximum_canonical_bytes {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::OutOfBounds,
                "canonical value exceeds the annex-wide byte bound",
            ));
        }
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|_| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidCanonicalJson,
                    "input is not exact float-free canonical JSON",
                )
            })?;
        let value: Value = serde_json::from_slice(canonical.as_bytes()).map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "canonical JSON could not be decoded",
            )
        })?;
        self.validate_registered_value(schema_contract, canonical, value)
    }

    /// Strictly decodes canonical JSON under the exact registered `schema_id`.
    pub fn strict_decode_schema_id(
        &self,
        schema_id: &SchemaId,
        bytes: &[u8],
    ) -> std::result::Result<ValidatedCanonicalValueV1, RecoverabilityError> {
        self.strict_decode(self.schema_contract_for_id(schema_id)?, bytes)
    }

    /// Projects every terminal content/value reference declared by the value's annex schema.
    ///
    /// Projection follows only the closed schema algebra. It never searches
    /// object keys or string contents for reference-like data. A root
    /// `mfm.value-ref.v1` reached through an incoming content reference is
    /// transport-only, so projection returns no edges and does not traverse
    /// its fields.
    pub fn reference_edges(
        &self,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<Vec<SchemaReferenceEdgeV1>, RecoverabilityError> {
        let schema = self.schemas.get(value.schema_contract()).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::UnknownSchema,
                "validated value names an unregistered schema contract",
            )
        })?;
        if schema.schema_id != value.schema_id {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::SchemaMismatch,
                "validated value carries a different registered schema identity",
            ));
        }
        if value.schema_contract() == "mfm.value-ref.v1" {
            return Ok(Vec::new());
        }

        let mut edges = Vec::new();
        self.collect_reference_edges(&schema.shape, &value.value, "", &mut edges, 0)?;
        Ok(edges)
    }

    /// Validates and canonically encodes a typed canonical value.
    pub fn encode(
        &self,
        schema_contract: &str,
        value: &CanonicalValue,
    ) -> std::result::Result<ValidatedCanonicalValueV1, RecoverabilityError> {
        let canonical = CanonicalJsonBytes::from_value(value);
        if canonical.as_bytes().len() > self.maximum_canonical_bytes {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::OutOfBounds,
                "canonical value exceeds the annex-wide byte bound",
            ));
        }
        let plain = PlainCanonicalJsonBytes::from_canonical_json_slice(canonical.as_bytes())
            .map_err(|_| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidCanonicalJson,
                    "typed value did not produce admissible canonical JSON",
                )
            })?;
        let json = serde_json::from_slice(plain.as_bytes()).map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "canonical JSON could not be decoded",
            )
        })?;
        self.validate_registered_value(schema_contract, plain, json)
    }

    /// Derives the one universal semantic digest for a registered domain.
    pub fn semantic_digest(
        &self,
        domain: &str,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<SemanticDigest, RecoverabilityError> {
        let definition = self.domains.get(domain).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::UnknownDomain,
                "semantic domain is not registered",
            )
        })?;
        if definition.preimage_schema != value.schema_contract {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::SchemaMismatch,
                "semantic domain requires a different registered schema",
            ));
        }

        let envelope = CanonicalValue::object([
            ("domain", CanonicalValue::String(domain.to_owned())),
            ("value", json_to_canonical_value(&value.value)?),
        ])
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "validated preimage could not form the universal envelope",
            )
        })?;
        let envelope_json = serde_json::from_slice(
            CanonicalJsonBytes::from_value(&envelope).as_bytes(),
        )
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "semantic envelope could not be decoded",
            )
        })?;
        let envelope_schema = self
            .schemas
            .get(&self.domain_envelope_schema)
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "registered semantic-envelope schema is absent",
                )
            })?;
        self.validate_shape(&envelope_schema.shape, &envelope_json, "$", 0)?;
        let bytes = CanonicalJsonBytes::from_value(&envelope);
        Ok(SemanticDigest::from_digest(sha256_digest_bytes(
            bytes.as_bytes(),
        )))
    }

    branded_domain_derivation!(
        derive_admission_logical_key,
        "mfm.admission-logical-key.v1",
        "semantic_digest",
        AdmissionLogicalKey,
        "Derives the frozen logical identity for one run-admission request."
    );

    digest_only_domain_derivation!(
        derive_artifact_id,
        "mfm.artifact-id.v1",
        "artifact_id",
        ArtifactId,
        "Derives the frozen identity of one retained artifact."
    );

    branded_domain_derivation!(
        derive_correction_invocation_digest,
        "mfm.correction-invocation.v1",
        "semantic_digest",
        CorrectionInvocationDigest,
        "Derives the frozen digest of one correction invocation."
    );

    branded_domain_derivation!(
        derive_effect_key,
        "mfm.effect-key.v1",
        "effect_key",
        EffectKey,
        "Derives the frozen keyed-effect identity."
    );

    branded_domain_derivation!(
        derive_attempt_id,
        "mfm.executor-delivery-attempt.v1",
        "attempt_id",
        AttemptId,
        "Derives the frozen identity of one executor delivery attempt."
    );

    branded_domain_derivation!(
        derive_executor_frontier_digest,
        "mfm.executor-frontier.v1",
        "semantic_digest",
        ExecutorFrontierDigest,
        "Derives the frozen digest of one executor frontier."
    );

    branded_domain_derivation!(
        derive_executor_record_digest,
        "mfm.executor-record.v1",
        "semantic_digest",
        ExecutorRecordDigest,
        "Derives the frozen digest of one executor-owned record."
    );

    branded_domain_derivation!(
        derive_fact_content_identity_digest,
        "mfm.fact-content-identity.v1",
        "semantic_digest",
        FactContentIdentityDigest,
        "Derives the frozen content identity digest of one fact."
    );

    branded_domain_derivation!(
        derive_fact_logical_identity_digest,
        "mfm.fact-logical-identity.v1",
        "semantic_digest",
        FactLogicalIdentityDigest,
        "Derives the frozen logical identity digest of one fact."
    );

    branded_domain_derivation!(
        derive_fact_query_digest,
        "mfm.fact-query.v1",
        "semantic_digest",
        FactQueryDigest,
        "Derives the frozen digest of one fact-selection request."
    );

    branded_domain_derivation!(
        derive_genesis_digest,
        "mfm.genesis.v1",
        "semantic_digest",
        GenesisDigest,
        "Derives the frozen digest of one run genesis preimage."
    );

    branded_domain_derivation!(
        derive_journal_candidate_digest,
        "mfm.journal-candidate.v1",
        "semantic_digest",
        JournalCandidateDigest,
        "Derives the frozen digest of one unassigned journal candidate."
    );

    branded_domain_derivation!(
        derive_journal_commit_digest,
        "mfm.journal-commit.v1",
        "semantic_digest",
        JournalCommitDigest,
        "Derives the frozen digest of one assigned journal commit."
    );

    branded_domain_derivation!(
        derive_record_id,
        "mfm.journal-record-id.v1",
        "record_id",
        RecordId,
        "Derives the frozen immutable journal record identity."
    );

    branded_domain_derivation!(
        derive_journal_record_hash,
        "mfm.journal-record.v1",
        "semantic_digest",
        JournalRecordHash,
        "Derives the frozen semantic hash of one journal record."
    );

    digest_only_domain_derivation!(
        derive_node_id,
        "mfm.node-occurrence.v1",
        "node_id",
        NodeId,
        "Derives the frozen identity of one node occurrence."
    );

    branded_domain_derivation!(
        derive_object_evidence_digest,
        "mfm.object-evidence.v1",
        "semantic_digest",
        ObjectEvidenceDigest,
        "Derives the frozen digest of retained-object evidence."
    );

    branded_domain_derivation!(
        derive_output_logical_identity_digest,
        "mfm.output-logical-identity.v1",
        "semantic_digest",
        OutputLogicalIdentityDigest,
        "Derives the frozen logical identity digest of one output occurrence."
    );

    branded_domain_derivation!(
        derive_request_digest,
        "mfm.request.v1",
        "semantic_digest",
        RequestDigest,
        "Derives the frozen semantic digest of one external request."
    );

    digest_only_domain_derivation!(
        derive_run_id,
        "mfm.run-id.v1",
        "run_id",
        RunId,
        "Derives the frozen identity of one run."
    );

    branded_domain_derivation!(
        derive_run_semantic_state_digest,
        "mfm.run-semantic-state.v1",
        "semantic_digest",
        RunSemanticStateDigest,
        "Derives the frozen digest of one reconstructed run state."
    );

    /// Derives the frozen redaction digest of one exact cross-run source reference.
    pub fn derive_cross_run_source_redaction_digest(
        &self,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<SemanticDigest, RecoverabilityError> {
        self.semantic_digest_for(
            "mfm.cross-run-source-redaction.v1",
            "semantic_digest",
            value,
        )
    }

    branded_domain_derivation!(
        derive_source_closure_digest,
        "mfm.source-closure.v1",
        "semantic_digest",
        SourceClosureDigest,
        "Derives the frozen digest of one cross-run source closure."
    );

    digest_only_domain_derivation!(
        derive_spec_hash,
        "mfm.spec-hash.v1",
        "spec_hash",
        SpecHash,
        "Derives the frozen hash identity of one certified spec."
    );

    branded_domain_derivation!(
        derive_terminal_effect_evidence_digest,
        "mfm.terminal-effect-evidence.v1",
        "semantic_digest",
        TerminalEffectEvidenceDigest,
        "Derives the frozen digest of terminal effect evidence."
    );

    /// Derives the frozen schema identity from its exact descriptor preimage.
    pub fn derive_schema_id(
        &self,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<SchemaId, RecoverabilityError> {
        let digest = self.semantic_digest_for("mfm.schema.v1", "schema_id", value)?;
        let contract = value
            .value
            .as_object()
            .and_then(|object| object.get("contract"))
            .and_then(Value::as_str)
            .and_then(|contract| contract.strip_suffix(".v1"))
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::IdentityConstruction,
                    "schema descriptor cannot form a schema identity",
                )
            })?;
        SchemaId::new(
            contract,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        )
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::IdentityConstruction,
                "schema descriptor cannot form a schema identity",
            )
        })
    }

    /// Computes the raw SHA-256 digest of exact retained bytes.
    pub fn raw_content_digest(&self, bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
    }

    /// Constructs the lightweight annex-derived content identity for a value.
    pub fn content_ref(
        &self,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<ContentRef, RecoverabilityError> {
        let registered = self.schema_id(&value.schema_contract)?;
        if registered != &value.schema_id {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::SchemaMismatch,
                "validated value carries a schema identity not registered by the annex",
            ));
        }
        ContentRef::new(
            value.schema_id.clone(),
            self.raw_content_digest(value.as_bytes()),
        )
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::IdentityConstruction,
                "annex-derived content reference violated the identity grammar",
            )
        })
    }

    fn validate_registered_value(
        &self,
        schema_contract: &str,
        canonical: PlainCanonicalJsonBytes,
        value: Value,
    ) -> std::result::Result<ValidatedCanonicalValueV1, RecoverabilityError> {
        let schema = self.schemas.get(schema_contract).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::UnknownSchema,
                "schema contract is not registered",
            )
        })?;
        self.validate_shape(&schema.shape, &value, "$", 0)?;
        Ok(ValidatedCanonicalValueV1 {
            schema_contract: schema_contract.to_owned(),
            schema_id: schema.schema_id.clone(),
            canonical,
            value,
        })
    }

    fn semantic_digest_for(
        &self,
        domain: &'static str,
        result_kind: &'static str,
        value: &ValidatedCanonicalValueV1,
    ) -> std::result::Result<SemanticDigest, RecoverabilityError> {
        let definition = self.domains.get(domain).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "frozen semantic domain is absent",
            )
        })?;
        if definition.result_kind != result_kind {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "frozen semantic domain has an unexpected result kind",
            ));
        }
        self.semantic_digest(domain, value)
    }

    fn collect_reference_edges(
        &self,
        shape: &Value,
        value: &Value,
        path: &str,
        edges: &mut Vec<SchemaReferenceEdgeV1>,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "reference projection exceeds the schema recursion bound",
            ));
        }
        let definition = shape.as_object().ok_or_else(|| {
            value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "schema algebra node is not an object",
            )
        })?;
        if definition
            .get("wire_representation")
            .and_then(Value::as_str)
            == Some("unwrapped_native_float_free_json")
        {
            return Ok(());
        }

        match required_string(definition, "kind", RecoverabilityErrorCode::InvalidAnnex)? {
            "reference" => {
                let declared_contract = required_string(
                    definition,
                    "contract",
                    RecoverabilityErrorCode::InvalidAnnex,
                )?;
                if let Some(terminal_kind) =
                    self.resolve_reference_terminal(declared_contract, depth + 1)?
                {
                    let terminal_value = self.encode(
                        terminal_kind.schema_contract(),
                        &json_to_canonical_value(value)?,
                    )?;
                    edges.push(SchemaReferenceEdgeV1 {
                        path: CanonicalReferencePathV1 {
                            pointer: path.to_owned(),
                        },
                        declared_contract: declared_contract.to_owned(),
                        terminal_kind,
                        value: terminal_value,
                    });
                    return Ok(());
                }
                let referenced = self.schemas.get(declared_contract).ok_or_else(|| {
                    value_error(
                        RecoverabilityErrorCode::InvalidAnnex,
                        path,
                        "schema reference target is not registered",
                    )
                })?;
                self.collect_reference_edges(&referenced.shape, value, path, edges, depth + 1)
            }
            "nullable" => {
                if value.is_null() {
                    Ok(())
                } else {
                    self.collect_reference_edges(
                        required_value(definition, "value", RecoverabilityErrorCode::InvalidAnnex)?,
                        value,
                        path,
                        edges,
                        depth + 1,
                    )
                }
            }
            "optional_absent" => self.collect_reference_edges(
                required_value(definition, "value", RecoverabilityErrorCode::InvalidAnnex)?,
                value,
                path,
                edges,
                depth + 1,
            ),
            "array" => {
                let values = value.as_array().ok_or_else(|| wrong_type(path, "array"))?;
                let item_shape =
                    required_value(definition, "items", RecoverabilityErrorCode::InvalidAnnex)?;
                for (index, item) in values.iter().enumerate() {
                    let item_path = append_json_pointer_token(path, &index.to_string());
                    self.collect_reference_edges(item_shape, item, &item_path, edges, depth + 1)?;
                }
                Ok(())
            }
            "object" => {
                let object = value
                    .as_object()
                    .ok_or_else(|| wrong_type(path, "object"))?;
                self.collect_reference_fields(
                    required_array(definition, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
                    object,
                    path,
                    edges,
                    depth,
                )
            }
            "tagged_union" => {
                let object = value
                    .as_object()
                    .ok_or_else(|| wrong_type(path, "tagged union object"))?;
                let discriminator = required_string(
                    definition,
                    "discriminator",
                    RecoverabilityErrorCode::InvalidAnnex,
                )?;
                let tag = object
                    .get(discriminator)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        value_error(
                            RecoverabilityErrorCode::MissingField,
                            path,
                            "tagged union discriminator is absent",
                        )
                    })?;
                let variant = required_array(
                    definition,
                    "variants",
                    RecoverabilityErrorCode::InvalidAnnex,
                )?
                .iter()
                .find_map(|variant| {
                    let variant = variant.as_object()?;
                    (variant.get("tag").and_then(Value::as_str) == Some(tag)).then_some(variant)
                })
                .ok_or_else(|| {
                    value_error(
                        RecoverabilityErrorCode::InvalidValue,
                        path,
                        "tagged union discriminator is not registered",
                    )
                })?;
                self.collect_reference_fields(
                    required_array(variant, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
                    object,
                    path,
                    edges,
                    depth,
                )
            }
            "boolean"
            | "bounded_unsigned_integer"
            | "canonical_decimal_u64"
            | "literal"
            | "string" => Ok(()),
            _ => Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "schema algebra uses an unknown kind",
            )),
        }
    }

    fn collect_reference_fields(
        &self,
        fields: &[Value],
        object: &Map<String, Value>,
        path: &str,
        edges: &mut Vec<SchemaReferenceEdgeV1>,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        let mut present_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let field = field.as_object().ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::InvalidAnnex,
                    path,
                    "field definition is not an object",
                )
            })?;
            let name = required_string(field, "name", RecoverabilityErrorCode::InvalidAnnex)?;
            let presence =
                required_string(field, "presence", RecoverabilityErrorCode::InvalidAnnex)?;
            match object.get(name) {
                Some(value) => present_fields.push((
                    utf16_sort_key(name),
                    name,
                    required_value(field, "type", RecoverabilityErrorCode::InvalidAnnex)?,
                    value,
                )),
                None if presence == "optional_absent" => {}
                None if presence == "required" => {
                    return Err(value_error(
                        RecoverabilityErrorCode::MissingField,
                        path,
                        "required object field is absent",
                    ));
                }
                None => {
                    return Err(value_error(
                        RecoverabilityErrorCode::InvalidAnnex,
                        path,
                        "field uses an unknown presence mode",
                    ));
                }
            }
        }
        present_fields.sort_by(|left, right| left.0.cmp(&right.0));
        for (_, name, field_shape, field_value) in present_fields {
            let field_path = append_json_pointer_token(path, name);
            self.collect_reference_edges(field_shape, field_value, &field_path, edges, depth + 1)?;
        }
        Ok(())
    }

    fn resolve_reference_terminal(
        &self,
        contract: &str,
        depth: usize,
    ) -> std::result::Result<Option<ReferenceTerminalKindV1>, RecoverabilityError> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema reference alias chain exceeds its bound",
            ));
        }
        match contract {
            "mfm.content-ref.v1" => return Ok(Some(ReferenceTerminalKindV1::ContentRef)),
            "mfm.value-ref.v1" => return Ok(Some(ReferenceTerminalKindV1::ValueRef)),
            _ => {}
        }
        let referenced = self.schemas.get(contract).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema reference target is not registered",
            )
        })?;
        let definition = referenced.shape.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema algebra node is not an object",
            )
        })?;
        if definition.get("kind").and_then(Value::as_str) != Some("reference") {
            return Ok(None);
        }
        self.resolve_reference_terminal(
            required_string(
                definition,
                "contract",
                RecoverabilityErrorCode::InvalidAnnex,
            )?,
            depth + 1,
        )
    }
}

impl RecoverabilityContractV1 {
    fn from_annex_bytes(bytes: &[u8]) -> std::result::Result<Self, RecoverabilityError> {
        if bytes.len() > MAX_ANNEX_BYTES {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex exceeds the code-owned candidate byte bound",
            ));
        }
        let annex = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "embedded annex is not exact float-free canonical JSON",
            )
        })?;
        let document: Value = serde_json::from_slice(annex.as_bytes()).map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "embedded annex could not be decoded",
            )
        })?;
        let root = document.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "embedded annex root is not an object",
            )
        })?;
        validate_annex_root(root)?;

        let schema_values = required_array(root, "schemas", RecoverabilityErrorCode::InvalidAnnex)?;
        let mut schemas = BTreeMap::new();
        let mut schema_descriptors = BTreeMap::new();
        let mut schema_contracts_by_id = BTreeMap::new();
        for descriptor in schema_values {
            let descriptor = validate_schema_descriptor(descriptor)?;
            if schemas.contains_key(&descriptor.contract) {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "annex contains a duplicate schema contract",
                ));
            }
            if schema_contracts_by_id
                .insert(descriptor.schema_id.clone(), descriptor.contract.clone())
                .is_some()
            {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "annex contains a duplicate schema identity",
                ));
            }
            schema_descriptors.insert(descriptor.contract.clone(), descriptor.digest_preimage);
            schemas.insert(
                descriptor.contract,
                SchemaDefinition {
                    schema_id: descriptor.schema_id,
                    shape: descriptor.shape,
                },
            );
        }

        let domain_values = required_array(root, "domains", RecoverabilityErrorCode::InvalidAnnex)?;
        let mut domains = BTreeMap::new();
        for domain in domain_values {
            let (name, definition) = validate_domain_descriptor(domain)?;
            if domains.insert(name, definition).is_some() {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "annex contains a duplicate semantic domain",
                ));
            }
        }

        let contract = Self {
            annex,
            schemas,
            schema_contracts_by_id,
            domains,
            domain_envelope_schema: required_string(
                required_object(root, "canonicalization")?,
                "domain_envelope_schema",
                RecoverabilityErrorCode::InvalidAnnex,
            )?
            .to_owned(),
            maximum_canonical_bytes: annex_maximum_canonical_bytes(root)?,
        };
        contract.validate_annex_graph(&schema_descriptors)?;
        Ok(contract)
    }

    fn validate_annex_graph(
        &self,
        schema_descriptors: &BTreeMap<String, Value>,
    ) -> std::result::Result<(), RecoverabilityError> {
        validate_native_schema_contract(&self.schemas)?;
        for schema in self.schemas.values() {
            validate_shape_definition(&schema.shape, &self.schemas, 0)?;
        }
        validate_reference_graph(&self.schemas)?;
        if !self.schemas.contains_key(&self.domain_envelope_schema) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "canonicalization names an unknown semantic-envelope schema",
            ));
        }
        for domain in self.domains.values() {
            if !self.schemas.contains_key(&domain.preimage_schema) {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "semantic domain refers to an unknown preimage schema",
                ));
            }
        }

        let mut schema_domains = self
            .domains
            .iter()
            .filter(|(_, domain)| domain.result_kind == "schema_id");
        let (schema_domain, _) = schema_domains.next().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex has no schema-identity semantic domain",
            )
        })?;
        if schema_domains.next().is_some() {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex has more than one schema-identity semantic domain",
            ));
        }

        for (contract, descriptor) in schema_descriptors {
            let schema = self.schemas.get(contract).ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema descriptor index is inconsistent",
                )
            })?;
            let digest = semantic_digest_bytes(schema_domain, descriptor)?;
            let name = contract.strip_suffix(".v1").ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema contract does not end in '.v1'",
                )
            })?;
            let expected =
                SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest).map_err(|_| {
                    RecoverabilityError::new(
                        RecoverabilityErrorCode::InvalidAnnex,
                        "schema identity preimage violates the identity grammar",
                    )
                })?;
            if expected != schema.schema_id {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema identity does not match its annex descriptor",
                ));
            }
        }
        Ok(())
    }

    fn validate_shape(
        &self,
        shape: &Value,
        value: &Value,
        path: &str,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "schema recursion exceeds the recoverability-v1 bound",
            ));
        }
        let definition = shape.as_object().ok_or_else(|| {
            value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "schema algebra node is not an object",
            )
        })?;

        if definition
            .get("wire_representation")
            .and_then(Value::as_str)
            == Some("unwrapped_native_float_free_json")
        {
            return validate_native_canonical_value(value, path, depth);
        }

        let kind = required_string(definition, "kind", RecoverabilityErrorCode::InvalidAnnex)?;
        match kind {
            "reference" => {
                let contract = required_string(
                    definition,
                    "contract",
                    RecoverabilityErrorCode::InvalidAnnex,
                )?;
                let referenced = self.schemas.get(contract).ok_or_else(|| {
                    value_error(
                        RecoverabilityErrorCode::InvalidAnnex,
                        path,
                        "schema reference target is not registered",
                    )
                })?;
                self.validate_shape(&referenced.shape, value, path, depth + 1)
            }
            "nullable" => {
                if value.is_null() {
                    Ok(())
                } else {
                    let nested =
                        required_value(definition, "value", RecoverabilityErrorCode::InvalidAnnex)?;
                    self.validate_shape(nested, value, path, depth + 1)
                }
            }
            "optional_absent" => {
                let nested =
                    required_value(definition, "value", RecoverabilityErrorCode::InvalidAnnex)?;
                self.validate_shape(nested, value, path, depth + 1)
            }
            "literal" => {
                let expected =
                    required_value(definition, "value", RecoverabilityErrorCode::InvalidAnnex)?;
                if expected == value {
                    Ok(())
                } else {
                    Err(value_error(
                        RecoverabilityErrorCode::InvalidValue,
                        path,
                        "value does not equal the schema literal",
                    ))
                }
            }
            "boolean" => {
                if value.is_boolean() {
                    Ok(())
                } else {
                    Err(wrong_type(path, "boolean"))
                }
            }
            "string" => self.validate_string(definition, value, path),
            "bounded_unsigned_integer" => self.validate_bounded_unsigned(definition, value, path),
            "canonical_decimal_u64" => self.validate_decimal_u64(definition, value, path),
            "array" => self.validate_array(definition, value, path, depth),
            "object" => self.validate_object(definition, value, path, depth),
            "tagged_union" => self.validate_tagged_union(definition, value, path, depth),
            _ => Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "schema algebra uses an unknown kind",
            )),
        }
    }

    fn validate_string(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
    ) -> std::result::Result<(), RecoverabilityError> {
        let value = value.as_str().ok_or_else(|| wrong_type(path, "string"))?;
        let minimum = required_u64(
            definition,
            "min_length",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let maximum = required_u64(
            definition,
            "max_length",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let identity_grammar = definition
            .get("grammar")
            .and_then(Value::as_str)
            .is_some_and(is_identity_grammar);
        let length = u64::try_from(value.len()).map_err(|_| {
            value_error(
                if identity_grammar {
                    RecoverabilityErrorCode::IdentityConstruction
                } else {
                    RecoverabilityErrorCode::OutOfBounds
                },
                path,
                "string length cannot be represented",
            )
        })?;
        if length < minimum || length > maximum {
            return Err(value_error(
                if identity_grammar {
                    RecoverabilityErrorCode::IdentityConstruction
                } else {
                    RecoverabilityErrorCode::OutOfBounds
                },
                path,
                "string length is outside the schema bound",
            ));
        }

        if let Some(values) = definition.get("values") {
            let allowed = values.as_array().ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::InvalidAnnex,
                    path,
                    "string enum values are not an array",
                )
            })?;
            if !allowed
                .iter()
                .any(|candidate| candidate.as_str() == Some(value))
            {
                return Err(value_error(
                    RecoverabilityErrorCode::InvalidValue,
                    path,
                    "string is not a member of the closed enum",
                ));
            }
        }

        if let Some(grammar) = definition.get("grammar") {
            let grammar = grammar.as_str().ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::InvalidAnnex,
                    path,
                    "string grammar is not a string",
                )
            })?;
            if !string_matches_grammar(value, grammar) {
                return Err(value_error(
                    if is_identity_grammar(grammar) {
                        RecoverabilityErrorCode::IdentityConstruction
                    } else {
                        RecoverabilityErrorCode::InvalidValue
                    },
                    path,
                    "string violates its closed grammar",
                ));
            }
        }
        Ok(())
    }

    fn validate_bounded_unsigned(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
    ) -> std::result::Result<(), RecoverabilityError> {
        if definition.get("wire").and_then(Value::as_str) != Some("json_integer") {
            return Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "bounded unsigned integer has an unknown wire representation",
            ));
        }
        let value = value
            .as_u64()
            .ok_or_else(|| wrong_type(path, "unsigned JSON integer"))?;
        let minimum = required_decimal_bound(definition, "minimum")?;
        let maximum = required_decimal_bound(definition, "maximum")?;
        if value < minimum || value > maximum {
            return Err(value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "unsigned integer is outside the schema bound",
            ));
        }
        Ok(())
    }

    fn validate_decimal_u64(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
    ) -> std::result::Result<(), RecoverabilityError> {
        if definition.get("wire").and_then(Value::as_str) != Some("json_string") {
            return Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "decimal u64 has an unknown wire representation",
            ));
        }
        let text = value
            .as_str()
            .ok_or_else(|| wrong_type(path, "canonical decimal u64 string"))?;
        if !is_canonical_u64(text) {
            return Err(value_error(
                RecoverabilityErrorCode::InvalidValue,
                path,
                "decimal u64 has a noncanonical spelling",
            ));
        }
        let value = text.parse::<u64>().map_err(|_| {
            value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "decimal u64 exceeds its representation",
            )
        })?;
        let minimum = required_decimal_bound(definition, "minimum")?;
        let maximum = required_decimal_bound(definition, "maximum")?;
        if value < minimum || value > maximum {
            return Err(value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "decimal u64 is outside the schema bound",
            ));
        }
        Ok(())
    }

    fn validate_array(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        let values = value.as_array().ok_or_else(|| wrong_type(path, "array"))?;
        let minimum = required_u64(
            definition,
            "min_items",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let maximum = required_u64(
            definition,
            "max_items",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let length = u64::try_from(values.len()).map_err(|_| {
            value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "array length cannot be represented",
            )
        })?;
        if length < minimum || length > maximum {
            return Err(value_error(
                RecoverabilityErrorCode::OutOfBounds,
                path,
                "array length is outside the schema bound",
            ));
        }

        let items = required_value(definition, "items", RecoverabilityErrorCode::InvalidAnnex)?;
        for (index, item) in values.iter().enumerate() {
            self.validate_shape(items, item, &format!("{path}[{index}]"), depth + 1)?;
        }

        if required_bool(definition, "unique", RecoverabilityErrorCode::InvalidAnnex)? {
            let mut seen = BTreeSet::new();
            for item in values {
                let canonical = canonical_json(item)?;
                if !seen.insert(canonical) {
                    return Err(value_error(
                        RecoverabilityErrorCode::DuplicateItem,
                        path,
                        "array contains a duplicate canonical item",
                    ));
                }
            }
        }

        let ordering = required_string(
            definition,
            "ordering",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        self.validate_array_order(values, items, ordering, path)
    }

    fn validate_array_order(
        &self,
        values: &[Value],
        item_shape: &Value,
        ordering: &str,
        path: &str,
    ) -> std::result::Result<(), RecoverabilityError> {
        if ordering == "preserved" || is_relational_ordering(ordering) {
            return Ok(());
        }

        let mut previous: Option<Vec<String>> = None;
        for value in values {
            let key = ordering_key(self, value, item_shape, ordering)?;
            if previous.as_ref().is_some_and(|previous| previous > &key) {
                return Err(value_error(
                    RecoverabilityErrorCode::InvalidOrder,
                    path,
                    "array items are not in the schema-selected order",
                ));
            }
            previous = Some(key);
        }
        Ok(())
    }

    fn validate_object(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        if definition.get("unknown_fields").and_then(Value::as_str) != Some("reject") {
            return Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "object schema does not reject unknown fields",
            ));
        }
        let object = value
            .as_object()
            .ok_or_else(|| wrong_type(path, "object"))?;
        let fields = required_array(definition, "fields", RecoverabilityErrorCode::InvalidAnnex)?;
        self.validate_fields(fields, object, &BTreeSet::new(), path, depth)
    }

    fn validate_tagged_union(
        &self,
        definition: &Map<String, Value>,
        value: &Value,
        path: &str,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        if definition.get("unknown_fields").and_then(Value::as_str) != Some("reject") {
            return Err(value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "tagged union does not reject unknown fields",
            ));
        }
        let object = value
            .as_object()
            .ok_or_else(|| wrong_type(path, "tagged union object"))?;
        let discriminator = required_string(
            definition,
            "discriminator",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let tag = object
            .get(discriminator)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::MissingField,
                    path,
                    "tagged union discriminator is missing or not a string",
                )
            })?;
        let variants = required_array(
            definition,
            "variants",
            RecoverabilityErrorCode::InvalidAnnex,
        )?;
        let variant = variants
            .iter()
            .find(|variant| {
                variant
                    .as_object()
                    .and_then(|variant| variant.get("tag"))
                    .and_then(Value::as_str)
                    == Some(tag)
            })
            .ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::InvalidValue,
                    path,
                    "tagged union discriminator is not registered",
                )
            })?;
        let variant = variant.as_object().ok_or_else(|| {
            value_error(
                RecoverabilityErrorCode::InvalidAnnex,
                path,
                "tagged union variant is not an object",
            )
        })?;
        let fields = required_array(variant, "fields", RecoverabilityErrorCode::InvalidAnnex)?;
        self.validate_fields(
            fields,
            object,
            &BTreeSet::from([discriminator.to_owned()]),
            path,
            depth,
        )
    }

    fn validate_fields(
        &self,
        fields: &[Value],
        object: &Map<String, Value>,
        implicit_fields: &BTreeSet<String>,
        path: &str,
        depth: usize,
    ) -> std::result::Result<(), RecoverabilityError> {
        let mut allowed = implicit_fields.clone();
        for field in fields {
            let field = field.as_object().ok_or_else(|| {
                value_error(
                    RecoverabilityErrorCode::InvalidAnnex,
                    path,
                    "field definition is not an object",
                )
            })?;
            let name = required_string(field, "name", RecoverabilityErrorCode::InvalidAnnex)?;
            if !allowed.insert(name.to_owned()) {
                return Err(value_error(
                    RecoverabilityErrorCode::InvalidAnnex,
                    path,
                    "field definition is duplicated",
                ));
            }
            let presence =
                required_string(field, "presence", RecoverabilityErrorCode::InvalidAnnex)?;
            let field_shape = required_value(field, "type", RecoverabilityErrorCode::InvalidAnnex)?;
            match object.get(name) {
                Some(value) => {
                    self.validate_shape(field_shape, value, &format!("{path}.{name}"), depth + 1)?;
                }
                None if presence == "optional_absent" => {}
                None if presence == "required" => {
                    return Err(value_error(
                        RecoverabilityErrorCode::MissingField,
                        path,
                        "required object field is absent",
                    ));
                }
                None => {
                    return Err(value_error(
                        RecoverabilityErrorCode::InvalidAnnex,
                        path,
                        "field uses an unknown presence mode",
                    ));
                }
            }
        }

        if object.keys().any(|key| !allowed.contains(key)) {
            return Err(value_error(
                RecoverabilityErrorCode::UnknownField,
                path,
                "object contains a field not admitted by its schema",
            ));
        }
        Ok(())
    }
}

fn validate_native_canonical_value(
    value: &Value,
    path: &str,
    depth: usize,
) -> std::result::Result<(), RecoverabilityError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(value_error(
            RecoverabilityErrorCode::OutOfBounds,
            path,
            "native canonical value exceeds the recursion bound",
        ));
    }
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Number(value) if value.as_u64().is_some() => Ok(()),
        Value::Number(_) => Err(value_error(
            RecoverabilityErrorCode::WrongType,
            path,
            "signed and floating JSON numbers are not native canonical values",
        )),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_native_canonical_value(value, &format!("{path}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_native_canonical_value(value, &format!("{path}.*"), depth + 1)?;
            }
            Ok(())
        }
    }
}

fn validate_reference_graph(
    schemas: &BTreeMap<String, SchemaDefinition>,
) -> std::result::Result<(), RecoverabilityError> {
    fn visit(
        contract: &str,
        schemas: &BTreeMap<String, SchemaDefinition>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> std::result::Result<(), RecoverabilityError> {
        if visited.contains(contract) {
            return Ok(());
        }
        if !visiting.insert(contract.to_owned()) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema reference graph contains a cycle",
            ));
        }
        let schema = schemas.get(contract).ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema reference graph contains an unresolved target",
            )
        })?;
        let mut references = BTreeSet::new();
        collect_schema_references(&schema.shape, &mut references)?;
        for reference in references {
            if visiting.contains(&reference) {
                if is_reviewed_recursive_schema_edge(contract, &reference) {
                    continue;
                }
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema reference graph contains a cycle",
                ));
            }
            visit(&reference, schemas, visiting, visited)?;
        }
        visiting.remove(contract);
        visited.insert(contract.to_owned());
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for contract in schemas.keys() {
        visit(contract, schemas, &mut BTreeSet::new(), &mut visited)?;
    }
    Ok(())
}

fn collect_schema_references(
    shape: &Value,
    output: &mut BTreeSet<String>,
) -> std::result::Result<(), RecoverabilityError> {
    let object = shape.as_object().ok_or_else(|| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema algebra node is not an object",
        )
    })?;
    match object.get("kind").and_then(Value::as_str) {
        Some("reference") => {
            output.insert(
                required_string(object, "contract", RecoverabilityErrorCode::InvalidAnnex)?
                    .to_owned(),
            );
        }
        Some("nullable" | "optional_absent") => collect_schema_references(
            required_value(object, "value", RecoverabilityErrorCode::InvalidAnnex)?,
            output,
        )?,
        Some("array") => collect_schema_references(
            required_value(object, "items", RecoverabilityErrorCode::InvalidAnnex)?,
            output,
        )?,
        Some("object") => collect_field_references(
            required_array(object, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
            output,
        )?,
        Some("tagged_union") => {
            for variant in
                required_array(object, "variants", RecoverabilityErrorCode::InvalidAnnex)?
            {
                let variant = variant.as_object().ok_or_else(|| {
                    RecoverabilityError::new(
                        RecoverabilityErrorCode::InvalidAnnex,
                        "tagged union variant is not an object",
                    )
                })?;
                collect_field_references(
                    required_array(variant, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
                    output,
                )?;
            }
        }
        Some(
            "boolean" | "bounded_unsigned_integer" | "canonical_decimal_u64" | "literal" | "string",
        ) => {}
        _ => {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema algebra uses an unknown kind",
            ));
        }
    }
    Ok(())
}

fn collect_field_references(
    fields: &[Value],
    output: &mut BTreeSet<String>,
) -> std::result::Result<(), RecoverabilityError> {
    for field in fields {
        let field = field.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema field definition is not an object",
            )
        })?;
        collect_schema_references(
            required_value(field, "type", RecoverabilityErrorCode::InvalidAnnex)?,
            output,
        )?;
    }
    Ok(())
}

fn validate_native_schema_contract(
    schemas: &BTreeMap<String, SchemaDefinition>,
) -> std::result::Result<(), RecoverabilityError> {
    let mut native_contracts = schemas.iter().filter_map(|(contract, schema)| {
        (schema
            .shape
            .as_object()
            .and_then(|shape| shape.get("wire_representation"))
            .and_then(Value::as_str)
            == Some("unwrapped_native_float_free_json"))
        .then_some(contract.as_str())
    });
    if native_contracts.next() != Some("mfm.primitive-canonical_value.v1")
        || native_contracts.next().is_some()
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "native canonical wire semantics belong only to the reviewed canonical-value schema",
        ));
    }
    Ok(())
}

fn is_reviewed_recursive_schema_edge(source: &str, target: &str) -> bool {
    matches!(
        (source, target),
        (
            "mfm.primitive-canonical_value.v1",
            "mfm.primitive-canonical_value.v1"
        ) | (
            "mfm.primitive-canonical_value.v1",
            "mfm.primitive-canonical_object_entry.v1"
        ) | (
            "mfm.primitive-canonical_object_entry.v1",
            "mfm.primitive-canonical_value.v1"
        )
    )
}

fn json_to_canonical_value(
    value: &Value,
) -> std::result::Result<CanonicalValue, RecoverabilityError> {
    match value {
        Value::Null => Ok(CanonicalValue::Null),
        Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        Value::String(value) => Ok(CanonicalValue::String(value.clone())),
        Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidCanonicalJson,
                    "floats and integers outside i64/u64 are forbidden",
                ))
            }
        }
        Value::Array(values) => values
            .iter()
            .map(json_to_canonical_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(CanonicalValue::Array),
        Value::Object(values) => CanonicalValue::object(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_to_canonical_value(value)?)))
                .collect::<std::result::Result<Vec<_>, RecoverabilityError>>()?,
        )
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "canonical object contains duplicate fields",
            )
        }),
    }
}

fn ordering_key(
    contract: &RecoverabilityContractV1,
    value: &Value,
    item_shape: &Value,
    ordering: &str,
) -> std::result::Result<Vec<String>, RecoverabilityError> {
    if ordering == "canonical_json" {
        return Ok(vec![canonical_json(value)?]);
    }
    if ordering == "utf16_key" {
        let key = value
            .as_object()
            .and_then(|value| value.get("key"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "utf16_key ordering requires an object string field named 'key'",
                )
            })?;
        return Ok(vec![utf16_sort_key(key), canonical_json(value)?]);
    }
    if let Some(field) = ordering.strip_prefix("field:") {
        let field = value
            .as_object()
            .and_then(|value| value.get(field))
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "field ordering names a field absent from an array item",
                )
            })?;
        return Ok(vec![canonical_json(field)?, canonical_json(value)?]);
    }
    if let Some(fields) = ordering.strip_prefix("tuple:") {
        let object = value.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "tuple ordering requires object array items",
            )
        })?;
        let mut key = Vec::new();
        for field in fields.split(',') {
            key.push(canonical_json(object.get(field).ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "tuple ordering names a field absent from an array item",
                )
            })?)?);
        }
        key.push(canonical_json(value)?);
        return Ok(key);
    }
    if ordering == "binding_delta_kind_order" {
        let variants = resolved_union_variants(contract, item_shape, 0)?;
        let tag = value
            .as_object()
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "binding-delta ordering requires a tagged union item",
                )
            })?;
        let index = variants
            .iter()
            .position(|variant| {
                variant
                    .as_object()
                    .and_then(|variant| variant.get("tag"))
                    .and_then(Value::as_str)
                    == Some(tag)
            })
            .ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "binding-delta tag is absent from its referenced union",
                )
            })?;
        return Ok(vec![format!("{index:08x}"), canonical_json(value)?]);
    }
    Err(RecoverabilityError::new(
        RecoverabilityErrorCode::InvalidAnnex,
        "array schema uses an unknown ordering contract",
    ))
}

fn resolved_union_variants<'a>(
    contract: &'a RecoverabilityContractV1,
    shape: &'a Value,
    depth: usize,
) -> std::result::Result<&'a [Value], RecoverabilityError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "array item reference recursion exceeds the schema bound",
        ));
    }
    let shape = shape.as_object().ok_or_else(|| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "array item schema is not an object",
        )
    })?;
    match shape.get("kind").and_then(Value::as_str) {
        Some("tagged_union") => {
            required_array(shape, "variants", RecoverabilityErrorCode::InvalidAnnex)
        }
        Some("reference") => {
            let target = required_string(shape, "contract", RecoverabilityErrorCode::InvalidAnnex)?;
            let target = contract.schemas.get(target).ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "array item reference target is not registered",
                )
            })?;
            resolved_union_variants(contract, &target.shape, depth + 1)
        }
        _ => Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "binding-delta ordering does not refer to a tagged union",
        )),
    }
}

fn canonical_json(value: &Value) -> std::result::Result<String, RecoverabilityError> {
    let serialized = serde_json::to_string(value).map_err(|_| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidCanonicalJson,
            "JSON value could not be serialized",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&serialized)
        .map(|value| value.as_str().to_owned())
        .map_err(|_| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidCanonicalJson,
                "JSON value is not float-free canonical data",
            )
        })
}

fn append_json_pointer_token(path: &str, token: &str) -> String {
    let mut pointer = String::with_capacity(path.len() + token.len() + 1);
    pointer.push_str(path);
    pointer.push('/');
    for character in token.chars() {
        match character {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            _ => pointer.push(character),
        }
    }
    pointer
}

fn utf16_sort_key(value: &str) -> String {
    let mut key = String::with_capacity(value.len() * 4);
    for code_unit in value.encode_utf16() {
        key.push_str(&format!("{code_unit:04x}"));
    }
    key
}

fn string_matches_grammar(value: &str, grammar: &str) -> bool {
    match grammar {
        "[A-Za-z0-9_-]* with no padding" => {
            crate::CanonicalBytes::from_base64url_no_pad(value).is_ok()
        }
        "valid_unicode_scalar_string" => true,
        "-?[0-9]+ with no leading zeroes or negative zero" => is_canonical_i64(value),
        "0|-?[1-9][0-9]{0,18}" => is_canonical_i64(value),
        "0|[1-9][0-9]{0,19}" => is_canonical_u64(value),
        "dot-separated stable field segments" => {
            !value.is_empty()
                && value
                    .split('.')
                    .all(|segment| !segment.is_empty() && is_stable_id(segment))
        }
        "lowercase registered media type without parameters" => is_media_type(value),
        "versioned semantic identity with sha256-jcs-v1 digest" => {
            let mut parts = value.rsplitn(3, ':');
            let digest = parts.next();
            let algorithm = parts.next();
            let prefix = parts.next();
            digest.is_some_and(is_lower_hex_64)
                && algorithm == Some("sha256-jcs-v1")
                && prefix.is_some_and(|prefix| !prefix.is_empty())
        }
        "[a-z0-9][a-z0-9._/-]*" => is_stable_id(value),
        "mfm.<stable-domain>/<stable-name>@<positive-canonical-u64>" => is_entry_point_id(value),
        "P-[A-Z]{2,3}-[0-9]{2}" => {
            let bytes = value.as_bytes();
            matches!(bytes.len(), 7 | 8)
                && bytes[0..2] == *b"P-"
                && bytes[2..bytes.len() - 3].iter().all(u8::is_ascii_uppercase)
                && bytes[bytes.len() - 3] == b'-'
                && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_digit)
        }
        "[0-9a-f]{64}" => is_lower_hex_64(value),
        "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}" => is_uuid_v4(value),
        "mfm.[a-z0-9._-]+.v1" => {
            value.len() > 7
                && value.starts_with("mfm.")
                && value.ends_with(".v1")
                && value[4..value.len() - 3].bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        }
        "schema:<name>:1:sha256-jcs-v1:[0-9a-f]{64}" => {
            SchemaId::parse(value).is_ok_and(|schema| {
                schema.algorithm() == DigestAlgorithm::Sha256JcsV1 && schema.version() == Some("1")
            })
        }
        grammar if grammar.contains("[0-9a-f]{") => matches_fixed_digest_grammar(value, grammar),
        _ => false,
    }
}

fn is_identity_grammar(grammar: &str) -> bool {
    grammar.contains("[0-9a-f]{")
}

fn matches_fixed_digest_grammar(value: &str, grammar: &str) -> bool {
    let Some(open) = grammar.rfind("[0-9a-f]{") else {
        return false;
    };
    if !grammar.ends_with('}') {
        return false;
    }
    let width = &grammar[open + "[0-9a-f]{".len()..grammar.len() - 1];
    let Ok(width) = width.parse::<usize>() else {
        return false;
    };
    value.strip_prefix(&grammar[..open]).is_some_and(|suffix| {
        suffix.len() == width
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn is_stable_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'/' | b'-')
        })
}

fn is_entry_point_id(value: &str) -> bool {
    let Some((qualified_name, version)) = value.rsplit_once('@') else {
        return false;
    };
    if version == "0" || !is_canonical_u64(version) {
        return false;
    }
    let Some(path) = qualified_name.strip_prefix("mfm.") else {
        return false;
    };
    let Some((domain, name)) = path.split_once('/') else {
        return false;
    };
    !name.contains('/') && is_entry_point_component(domain) && is_entry_point_component(name)
}

fn is_entry_point_component(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn is_media_type(value: &str) -> bool {
    let Some((type_name, subtype)) = value.split_once('/') else {
        return false;
    };
    !type_name.is_empty()
        && !subtype.is_empty()
        && !value.contains(';')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(
                    byte,
                    b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-' | b'/'
                )
        })
}

fn is_uuid_v4(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
        })
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_canonical_u64(value: &str) -> bool {
    if value == "0" {
        return true;
    }
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

fn is_canonical_i64(value: &str) -> bool {
    if value == "0" {
        return true;
    }
    if let Some(value) = value.strip_prefix('-') {
        return !value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && format!("-{value}").parse::<i64>().is_ok();
    }
    !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<i64>().is_ok()
}

fn value_error(code: RecoverabilityErrorCode, _path: &str, message: &str) -> RecoverabilityError {
    RecoverabilityError::new(code, message)
}

fn wrong_type(path: &str, expected: &str) -> RecoverabilityError {
    value_error(
        RecoverabilityErrorCode::WrongType,
        path,
        &format!("value is not a {expected}"),
    )
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    code: RecoverabilityErrorCode,
) -> std::result::Result<&'a Value, RecoverabilityError> {
    object
        .get(field)
        .ok_or_else(|| RecoverabilityError::new(code, "required field is absent"))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    code: RecoverabilityErrorCode,
) -> std::result::Result<&'a str, RecoverabilityError> {
    required_value(object, field, code)?
        .as_str()
        .ok_or_else(|| RecoverabilityError::new(code, "field is not a string"))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    code: RecoverabilityErrorCode,
) -> std::result::Result<&'a [Value], RecoverabilityError> {
    required_value(object, field, code)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| RecoverabilityError::new(code, "field is not an array"))
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> std::result::Result<&'a Map<String, Value>, RecoverabilityError> {
    required_value(object, field, RecoverabilityErrorCode::InvalidAnnex)?
        .as_object()
        .ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "field is not an object",
            )
        })
}

fn required_u64(
    object: &Map<String, Value>,
    field: &str,
    code: RecoverabilityErrorCode,
) -> std::result::Result<u64, RecoverabilityError> {
    required_value(object, field, code)?
        .as_u64()
        .ok_or_else(|| RecoverabilityError::new(code, "field is not an unsigned integer"))
}

fn required_bool(
    object: &Map<String, Value>,
    field: &str,
    code: RecoverabilityErrorCode,
) -> std::result::Result<bool, RecoverabilityError> {
    required_value(object, field, code)?
        .as_bool()
        .ok_or_else(|| RecoverabilityError::new(code, "field is not a boolean"))
}

fn required_decimal_bound(
    object: &Map<String, Value>,
    field: &str,
) -> std::result::Result<u64, RecoverabilityError> {
    let value = required_string(object, field, RecoverabilityErrorCode::InvalidAnnex)?;
    if !is_canonical_u64(value) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "field is not a canonical u64",
        ));
    }
    value.parse::<u64>().map_err(|_| {
        RecoverabilityError::new(RecoverabilityErrorCode::InvalidAnnex, "field exceeds u64")
    })
}

struct SchemaDescriptorParts {
    contract: String,
    schema_id: SchemaId,
    shape: Value,
    digest_preimage: Value,
}

fn validate_annex_root(root: &Map<String, Value>) -> std::result::Result<(), RecoverabilityError> {
    require_exact_keys(
        root,
        &[
            "batch_legality",
            "canonicalization",
            "content_addressing",
            "contract",
            "domains",
            "error_codes",
            "identity_encodings",
            "limits",
            "logical_keys",
            "schema_algebra",
            "schemas",
            "transient_authority_types",
        ],
        "annex root",
    )?;
    let contract = required_string(root, "contract", RecoverabilityErrorCode::InvalidAnnex)?;
    if !string_matches_grammar(contract, "mfm.[a-z0-9._-]+.v1") {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "annex contract id violates the v1 contract grammar",
        ));
    }
    for field in [
        "batch_legality",
        "canonicalization",
        "content_addressing",
        "identity_encodings",
        "limits",
        "logical_keys",
    ] {
        if required_value(root, field, RecoverabilityErrorCode::InvalidAnnex)?
            .as_object()
            .is_none()
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex metadata field is not an object",
            ));
        }
    }
    for field in ["domains", "schema_algebra", "schemas"] {
        required_array(root, field, RecoverabilityErrorCode::InvalidAnnex)?;
    }
    for field in ["error_codes", "transient_authority_types"] {
        if required_value(root, field, RecoverabilityErrorCode::InvalidAnnex)?
            .as_object()
            .is_none()
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex metadata field is not an object",
            ));
        }
    }
    validate_annex_metadata(root)
}

fn validate_annex_metadata(
    root: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    let canonicalization = required_object(root, "canonicalization")?;
    require_exact_keys(
        canonicalization,
        &[
            "algorithm",
            "domain_envelope_schema",
            "duplicate_keys",
            "floats",
            "integer_numbers",
            "max_canonical_json_bytes",
            "object_key_order",
            "string_normalization",
        ],
        "canonicalization contract",
    )?;
    for (field, expected) in [
        ("algorithm", "sha256-jcs-v1"),
        ("duplicate_keys", "reject"),
        ("floats", "forbidden"),
        (
            "integer_numbers",
            "unsigned_json_integers_only_where_schema_bounded",
        ),
        ("object_key_order", "utf16_code_units"),
        ("string_normalization", "none"),
    ] {
        if required_string(
            canonicalization,
            field,
            RecoverabilityErrorCode::InvalidAnnex,
        )? != expected
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "canonicalization metadata contains an unknown value",
            ));
        }
    }
    let envelope_schema = required_string(
        canonicalization,
        "domain_envelope_schema",
        RecoverabilityErrorCode::InvalidAnnex,
    )?;
    if !string_matches_grammar(envelope_schema, "mfm.[a-z0-9._-]+.v1") {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "semantic-envelope schema violates the contract grammar",
        ));
    }

    let content = required_object(root, "content_addressing")?;
    require_exact_keys(
        content,
        &[
            "algorithm",
            "grammar",
            "input",
            "semantic_identity_substitution",
        ],
        "content-addressing contract",
    )?;
    for (field, expected) in [
        ("algorithm", "sha256-v1"),
        ("grammar", "content:sha256-v1:<64_lowercase_hex>"),
        ("input", "exact_retained_bytes"),
        ("semantic_identity_substitution", "forbidden"),
    ] {
        if required_string(content, field, RecoverabilityErrorCode::InvalidAnnex)? != expected {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "content-addressing metadata contains an unknown value",
            ));
        }
    }

    let limits = required_object(root, "limits")?;
    require_exact_keys(
        limits,
        &[
            "max_array_items",
            "max_base64url_characters",
            "max_canonical_json_bytes",
            "max_object_entries",
            "max_string_utf8_bytes",
        ],
        "annex limits",
    )?;
    for field in [
        "max_array_items",
        "max_base64url_characters",
        "max_canonical_json_bytes",
        "max_object_entries",
        "max_string_utf8_bytes",
    ] {
        let value = required_string(limits, field, RecoverabilityErrorCode::InvalidAnnex)?;
        if !is_canonical_u64(value) || value == "0" {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex limit is not a positive canonical u64 string",
            ));
        }
    }
    if required_string(
        canonicalization,
        "max_canonical_json_bytes",
        RecoverabilityErrorCode::InvalidAnnex,
    )? != required_string(
        limits,
        "max_canonical_json_bytes",
        RecoverabilityErrorCode::InvalidAnnex,
    )? {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "canonicalization and limits disagree on the byte bound",
        ));
    }

    let identities = required_object(root, "identity_encodings")?;
    require_exact_keys(
        identities,
        &[
            "artifact_id",
            "attempt_id",
            "content_digest",
            "effect_key",
            "node_id",
            "record_id",
            "run_id",
            "schema_id",
            "semantic_digest",
            "spec_hash",
            "store_scope_id",
            "tenant_scope_id",
        ],
        "identity encodings",
    )?;
    if identities.values().any(|value| {
        value
            .as_str()
            .is_none_or(|value| value.is_empty() || !value.is_ascii())
    }) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "identity encoding is not a non-empty ASCII grammar",
        ));
    }

    validate_error_code_registry(required_object(root, "error_codes")?)?;
    validate_schema_algebra_registry(required_array(
        root,
        "schema_algebra",
        RecoverabilityErrorCode::InvalidAnnex,
    )?)?;

    let transient = required_object(root, "transient_authority_types")?;
    require_exact_keys(
        transient,
        &["closed", "serialization", "values"],
        "transient authority registry",
    )?;
    if !required_bool(transient, "closed", RecoverabilityErrorCode::InvalidAnnex)?
        || required_string(
            transient,
            "serialization",
            RecoverabilityErrorCode::InvalidAnnex,
        )? != "forbidden"
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "transient authority registry is not closed and nonserializable",
        ));
    }
    validate_unique_strings(required_array(
        transient,
        "values",
        RecoverabilityErrorCode::InvalidAnnex,
    )?)?;
    validate_logical_key_registry(required_object(root, "logical_keys")?)?;
    validate_batch_registry(required_object(root, "batch_legality")?)?;
    Ok(())
}

fn validate_logical_key_registry(
    registry: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    require_exact_keys(registry, &["closed", "entries"], "logical-key registry")?;
    if !required_bool(registry, "closed", RecoverabilityErrorCode::InvalidAnnex)? {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "logical-key registry is not closed",
        ));
    }
    let mut contracts = BTreeSet::new();
    for entry in required_array(registry, "entries", RecoverabilityErrorCode::InvalidAnnex)? {
        let entry = entry.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "logical-key entry is not an object",
            )
        })?;
        require_exact_keys(
            entry,
            &["contract", "fields", "owner_predicates", "uniqueness"],
            "logical-key entry",
        )?;
        let contract = required_string(entry, "contract", RecoverabilityErrorCode::InvalidAnnex)?;
        if !string_matches_grammar(contract, "mfm.[a-z0-9._-]+.v1") || !contracts.insert(contract) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "logical-key contract is invalid or duplicated",
            ));
        }
        let fields = required_array(entry, "fields", RecoverabilityErrorCode::InvalidAnnex)?;
        validate_unique_strings(fields)?;
        if fields.is_empty()
            || required_string(entry, "uniqueness", RecoverabilityErrorCode::InvalidAnnex)?
                .is_empty()
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "logical-key contract is empty",
            ));
        }
        validate_predicate_ids(required_array(
            entry,
            "owner_predicates",
            RecoverabilityErrorCode::InvalidAnnex,
        )?)?;
    }
    Ok(())
}

fn validate_batch_registry(
    registry: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    require_exact_keys(registry, &["closed", "entries"], "batch-legality registry")?;
    if !required_bool(registry, "closed", RecoverabilityErrorCode::InvalidAnnex)? {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "batch-legality registry is not closed",
        ));
    }
    let mut batches = BTreeSet::new();
    for entry in required_array(registry, "entries", RecoverabilityErrorCode::InvalidAnnex)? {
        let entry = entry.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "batch-legality entry is not an object",
            )
        })?;
        require_exact_keys(
            entry,
            &["batch", "coordinate", "records"],
            "batch-legality entry",
        )?;
        let batch = required_string(entry, "batch", RecoverabilityErrorCode::InvalidAnnex)?;
        let coordinate =
            required_string(entry, "coordinate", RecoverabilityErrorCode::InvalidAnnex)?;
        let records = required_array(entry, "records", RecoverabilityErrorCode::InvalidAnnex)?;
        if !is_stable_id(batch)
            || !batches.insert(batch)
            || coordinate.is_empty()
            || records.is_empty()
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "batch-legality entry is empty, invalid, or duplicated",
            ));
        }
        validate_unique_strings(records)?;
    }
    Ok(())
}

fn validate_error_code_registry(
    registry: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    require_exact_keys(registry, &["codec", "relational"], "error-code registry")?;
    let codec = required_object(registry, "codec")?;
    let relational = required_object(registry, "relational")?;
    for family in [codec, relational] {
        require_exact_keys(family, &["closed", "values"], "error-code family")?;
        if !required_bool(family, "closed", RecoverabilityErrorCode::InvalidAnnex)? {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "error-code family is not closed",
            ));
        }
        validate_unique_strings(required_array(
            family,
            "values",
            RecoverabilityErrorCode::InvalidAnnex,
        )?)?;
    }
    let expected = [
        RecoverabilityErrorCode::InvalidAnnex,
        RecoverabilityErrorCode::InvalidCanonicalJson,
        RecoverabilityErrorCode::UnknownSchema,
        RecoverabilityErrorCode::UnknownDomain,
        RecoverabilityErrorCode::SchemaMismatch,
        RecoverabilityErrorCode::MissingField,
        RecoverabilityErrorCode::UnknownField,
        RecoverabilityErrorCode::WrongType,
        RecoverabilityErrorCode::InvalidValue,
        RecoverabilityErrorCode::OutOfBounds,
        RecoverabilityErrorCode::DuplicateItem,
        RecoverabilityErrorCode::InvalidOrder,
        RecoverabilityErrorCode::IdentityConstruction,
    ]
    .into_iter()
    .map(RecoverabilityErrorCode::as_str)
    .collect::<BTreeSet<_>>();
    let actual = required_array(codec, "values", RecoverabilityErrorCode::InvalidAnnex)?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "annex codec error registry does not match the public enum",
        ));
    }
    let relational = required_array(relational, "values", RecoverabilityErrorCode::InvalidAnnex)?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    if relational.is_empty()
        || !actual.is_disjoint(&relational)
        || relational.iter().any(|value| !is_stable_id(value))
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "relational error registry is empty, invalid, or overlaps codec errors",
        ));
    }
    Ok(())
}

fn validate_schema_algebra_registry(
    values: &[Value],
) -> std::result::Result<(), RecoverabilityError> {
    let actual = values
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "array",
        "boolean",
        "bounded_unsigned_integer",
        "canonical_decimal_u64",
        "literal",
        "nullable",
        "object",
        "optional_absent",
        "reference",
        "string",
        "tagged_union",
    ]);
    if actual.len() != values.len() || actual != expected {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema-algebra registry does not match the codec",
        ));
    }
    Ok(())
}

fn validate_unique_strings(values: &[Value]) -> std::result::Result<(), RecoverabilityError> {
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "closed registry value is not a string",
            )
        })?;
        if !seen.insert(value) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "closed registry contains a duplicate value",
            ));
        }
    }
    Ok(())
}

fn validate_schema_descriptor(
    value: &Value,
) -> std::result::Result<SchemaDescriptorParts, RecoverabilityError> {
    let object = value.as_object().ok_or_else(|| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema descriptor is not an object",
        )
    })?;
    require_exact_keys(
        object,
        &["contract", "owner_predicates", "schema_id", "shape"],
        "schema descriptor",
    )?;
    let contract = required_string(object, "contract", RecoverabilityErrorCode::InvalidAnnex)?;
    if !string_matches_grammar(contract, "mfm.[a-z0-9._-]+.v1") {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema contract violates the v1 contract grammar",
        ));
    }
    validate_predicate_ids(required_array(
        object,
        "owner_predicates",
        RecoverabilityErrorCode::InvalidAnnex,
    )?)?;
    let schema_id = SchemaId::parse(required_string(
        object,
        "schema_id",
        RecoverabilityErrorCode::InvalidAnnex,
    )?)
    .map_err(|_| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema id violates its identity grammar",
        )
    })?;
    if schema_id.algorithm() != DigestAlgorithm::Sha256JcsV1
        || schema_id.version() != Some("1")
        || schema_id.canonical_name() != contract.strip_suffix(".v1")
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema id does not match its contract name and version",
        ));
    }
    let shape = required_value(object, "shape", RecoverabilityErrorCode::InvalidAnnex)?.clone();
    let mut preimage = object.clone();
    preimage.remove("schema_id");
    Ok(SchemaDescriptorParts {
        contract: contract.to_owned(),
        schema_id,
        shape,
        digest_preimage: Value::Object(preimage),
    })
}

fn validate_domain_descriptor(
    value: &Value,
) -> std::result::Result<(String, DomainDefinition), RecoverabilityError> {
    let object = value.as_object().ok_or_else(|| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "semantic domain descriptor is not an object",
        )
    })?;
    require_exact_keys(
        object,
        &[
            "domain",
            "owner_predicates",
            "preimage_schema",
            "result_kind",
        ],
        "semantic domain descriptor",
    )?;
    let domain = required_string(object, "domain", RecoverabilityErrorCode::InvalidAnnex)?;
    if !string_matches_grammar(domain, "mfm.[a-z0-9._-]+.v1") {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "semantic domain violates the v1 domain grammar",
        ));
    }
    validate_predicate_ids(required_array(
        object,
        "owner_predicates",
        RecoverabilityErrorCode::InvalidAnnex,
    )?)?;
    let preimage_schema = required_string(
        object,
        "preimage_schema",
        RecoverabilityErrorCode::InvalidAnnex,
    )?;
    if !string_matches_grammar(preimage_schema, "mfm.[a-z0-9._-]+.v1") {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "domain preimage schema violates the v1 contract grammar",
        ));
    }
    let result_kind =
        required_string(object, "result_kind", RecoverabilityErrorCode::InvalidAnnex)?;
    if !is_stable_id(result_kind) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "semantic domain result kind violates the stable-id grammar",
        ));
    }
    Ok((
        domain.to_owned(),
        DomainDefinition {
            preimage_schema: preimage_schema.to_owned(),
            result_kind: result_kind.to_owned(),
        },
    ))
}

fn validate_shape_definition(
    shape: &Value,
    schemas: &BTreeMap<String, SchemaDefinition>,
    depth: usize,
) -> std::result::Result<(), RecoverabilityError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema algebra recursion exceeds its bound",
        ));
    }
    let object = shape.as_object().ok_or_else(|| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema algebra node is not an object",
        )
    })?;
    let kind = required_string(object, "kind", RecoverabilityErrorCode::InvalidAnnex)?;
    match kind {
        "reference" => {
            require_exact_keys(object, &["contract", "kind"], "reference schema node")?;
            let target =
                required_string(object, "contract", RecoverabilityErrorCode::InvalidAnnex)?;
            if !schemas.contains_key(target) {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema reference target is not registered",
                ));
            }
        }
        "nullable" | "optional_absent" => {
            require_exact_keys(object, &["kind", "value"], "wrapper schema node")?;
            validate_shape_definition(
                required_value(object, "value", RecoverabilityErrorCode::InvalidAnnex)?,
                schemas,
                depth + 1,
            )?;
        }
        "literal" => {
            require_exact_keys(object, &["kind", "value"], "literal schema node")?;
        }
        "boolean" => {
            require_exact_keys(object, &["kind"], "boolean schema node")?;
        }
        "bounded_unsigned_integer" => {
            require_exact_keys(
                object,
                &["kind", "maximum", "minimum", "wire"],
                "bounded integer schema node",
            )?;
            if required_string(object, "wire", RecoverabilityErrorCode::InvalidAnnex)?
                != "json_integer"
            {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "bounded integer uses an unknown wire representation",
                ));
            }
            validate_numeric_bounds(object)?;
        }
        "canonical_decimal_u64" => {
            require_exact_keys(
                object,
                &["grammar", "kind", "maximum", "minimum", "wire"],
                "decimal u64 schema node",
            )?;
            if required_string(object, "wire", RecoverabilityErrorCode::InvalidAnnex)?
                != "json_string"
                || required_string(object, "grammar", RecoverabilityErrorCode::InvalidAnnex)?
                    != "0|[1-9][0-9]{0,19}"
            {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "decimal u64 uses an unknown wire contract",
                ));
            }
            validate_numeric_bounds(object)?;
        }
        "string" => {
            let mut allowed = BTreeSet::from(["kind", "max_length", "min_length"]);
            if object.contains_key("grammar") {
                allowed.insert("grammar");
            }
            if object.contains_key("values") {
                allowed.insert("values");
            }
            if object.contains_key("wire_representation") {
                allowed.insert("wire_representation");
                allowed.insert("discriminator_presence");
            }
            require_exact_key_set(object, &allowed, "string schema node")?;
            validate_string_definition(object)?;
        }
        "array" => {
            require_exact_keys(
                object,
                &[
                    "items",
                    "kind",
                    "max_items",
                    "min_items",
                    "ordering",
                    "unique",
                ],
                "array schema node",
            )?;
            validate_collection_bounds(object)?;
            validate_ordering_contract(required_string(
                object,
                "ordering",
                RecoverabilityErrorCode::InvalidAnnex,
            )?)?;
            required_bool(object, "unique", RecoverabilityErrorCode::InvalidAnnex)?;
            validate_shape_definition(
                required_value(object, "items", RecoverabilityErrorCode::InvalidAnnex)?,
                schemas,
                depth + 1,
            )?;
        }
        "object" => {
            require_exact_keys(
                object,
                &["fields", "invariants", "kind", "unknown_fields"],
                "object schema node",
            )?;
            validate_closed_object_metadata(object)?;
            validate_field_definitions(
                required_array(object, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
                schemas,
                depth,
            )?;
        }
        "tagged_union" => {
            let mut allowed = BTreeSet::from([
                "discriminator",
                "invariants",
                "kind",
                "unknown_fields",
                "variants",
            ]);
            if object.contains_key("wire_representation") {
                allowed.insert("wire_representation");
                allowed.insert("discriminator_presence");
            }
            require_exact_key_set(object, &allowed, "tagged union schema node")?;
            validate_closed_object_metadata(object)?;
            validate_union_definition(object, schemas, depth)?;
        }
        _ => {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema algebra uses an unknown kind",
            ));
        }
    }
    Ok(())
}

fn validate_field_definitions(
    fields: &[Value],
    schemas: &BTreeMap<String, SchemaDefinition>,
    depth: usize,
) -> std::result::Result<(), RecoverabilityError> {
    let mut names = BTreeSet::new();
    for field in fields {
        let field = field.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema field definition is not an object",
            )
        })?;
        require_exact_keys(
            field,
            &["name", "presence", "type"],
            "schema field definition",
        )?;
        let name = required_string(field, "name", RecoverabilityErrorCode::InvalidAnnex)?;
        if !is_stable_id(name) || !names.insert(name) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "schema field name is invalid or duplicated",
            ));
        }
        let presence = required_string(field, "presence", RecoverabilityErrorCode::InvalidAnnex)?;
        let shape = required_value(field, "type", RecoverabilityErrorCode::InvalidAnnex)?;
        match presence {
            "required" => {}
            "optional_absent"
                if shape
                    .as_object()
                    .and_then(|shape| shape.get("kind"))
                    .and_then(Value::as_str)
                    == Some("optional_absent") => {}
            _ => {
                return Err(RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "schema field uses an inconsistent presence contract",
                ));
            }
        }
        validate_shape_definition(shape, schemas, depth + 1)?;
    }
    Ok(())
}

fn validate_union_definition(
    object: &Map<String, Value>,
    schemas: &BTreeMap<String, SchemaDefinition>,
    depth: usize,
) -> std::result::Result<(), RecoverabilityError> {
    if required_string(
        object,
        "discriminator",
        RecoverabilityErrorCode::InvalidAnnex,
    )? != "kind"
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "tagged union discriminator is not 'kind'",
        ));
    }
    if let Some(wire) = object.get("wire_representation") {
        if wire.as_str() != Some("unwrapped_native_float_free_json")
            || object.get("discriminator_presence").and_then(Value::as_str)
                != Some("derived_from_native_json_type_not_encoded")
        {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "tagged union uses an unknown native wire representation",
            ));
        }
    }
    let variants = required_array(object, "variants", RecoverabilityErrorCode::InvalidAnnex)?;
    if variants.is_empty() {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "tagged union has no variants",
        ));
    }
    let mut tags = BTreeSet::new();
    for variant in variants {
        let variant = variant.as_object().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "tagged union variant is not an object",
            )
        })?;
        require_exact_keys(variant, &["fields", "tag"], "tagged union variant")?;
        let tag = required_string(variant, "tag", RecoverabilityErrorCode::InvalidAnnex)?;
        if !is_stable_id(tag) || !tags.insert(tag) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "tagged union tag is invalid or duplicated",
            ));
        }
        validate_field_definitions(
            required_array(variant, "fields", RecoverabilityErrorCode::InvalidAnnex)?,
            schemas,
            depth + 1,
        )?;
    }
    Ok(())
}

fn validate_string_definition(
    object: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    let minimum = required_u64(object, "min_length", RecoverabilityErrorCode::InvalidAnnex)?;
    let maximum = required_u64(object, "max_length", RecoverabilityErrorCode::InvalidAnnex)?;
    if minimum > maximum {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "string schema has inverted length bounds",
        ));
    }
    match (object.get("grammar"), object.get("values")) {
        (Some(grammar), None) if grammar.as_str().is_some_and(is_supported_string_grammar) => {}
        (None, Some(values)) => {
            let values = values.as_array().ok_or_else(|| {
                RecoverabilityError::new(
                    RecoverabilityErrorCode::InvalidAnnex,
                    "string enum values are not an array",
                )
            })?;
            let mut seen = BTreeSet::new();
            for value in values {
                let value = value.as_str().ok_or_else(|| {
                    RecoverabilityError::new(
                        RecoverabilityErrorCode::InvalidAnnex,
                        "string enum member is not a string",
                    )
                })?;
                if !seen.insert(value) {
                    return Err(RecoverabilityError::new(
                        RecoverabilityErrorCode::InvalidAnnex,
                        "string enum member is duplicated",
                    ));
                }
            }
        }
        _ => {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "string schema must select exactly one grammar or enum",
            ));
        }
    }
    Ok(())
}

fn is_supported_string_grammar(grammar: &str) -> bool {
    matches!(
        grammar,
        "[A-Za-z0-9_-]* with no padding"
            | "valid_unicode_scalar_string"
            | "-?[0-9]+ with no leading zeroes or negative zero"
            | "0|-?[1-9][0-9]{0,18}"
            | "0|[1-9][0-9]{0,19}"
            | "dot-separated stable field segments"
            | "lowercase registered media type without parameters"
            | "versioned semantic identity with sha256-jcs-v1 digest"
            | "[a-z0-9][a-z0-9._/-]*"
            | "mfm.<stable-domain>/<stable-name>@<positive-canonical-u64>"
            | "P-[A-Z]{2,3}-[0-9]{2}"
            | "[0-9a-f]{64}"
            | "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}"
            | "mfm.[a-z0-9._-]+.v1"
            | "schema:<name>:1:sha256-jcs-v1:[0-9a-f]{64}"
    ) || (grammar.contains("[0-9a-f]{") && grammar.ends_with('}'))
}

fn validate_closed_object_metadata(
    object: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    if required_string(
        object,
        "unknown_fields",
        RecoverabilityErrorCode::InvalidAnnex,
    )? != "reject"
    {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "object-like schema does not reject unknown fields",
        ));
    }
    let invariants = required_array(object, "invariants", RecoverabilityErrorCode::InvalidAnnex)?;
    if invariants.iter().any(|value| !value.is_string()) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "schema invariant is not a string",
        ));
    }
    Ok(())
}

fn validate_numeric_bounds(
    object: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    let minimum = required_decimal_bound(object, "minimum")?;
    let maximum = required_decimal_bound(object, "maximum")?;
    if minimum > maximum {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "numeric schema has inverted bounds",
        ));
    }
    Ok(())
}

fn validate_collection_bounds(
    object: &Map<String, Value>,
) -> std::result::Result<(), RecoverabilityError> {
    let minimum = required_u64(object, "min_items", RecoverabilityErrorCode::InvalidAnnex)?;
    let maximum = required_u64(object, "max_items", RecoverabilityErrorCode::InvalidAnnex)?;
    if minimum > maximum {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "array schema has inverted bounds",
        ));
    }
    Ok(())
}

fn validate_ordering_contract(ordering: &str) -> std::result::Result<(), RecoverabilityError> {
    let valid = matches!(
        ordering,
        "binding_delta_kind_order" | "canonical_json" | "preserved" | "utf16_key"
    ) || is_relational_ordering(ordering)
        || ordering.strip_prefix("field:").is_some_and(is_stable_id)
        || ordering.strip_prefix("tuple:").is_some_and(|fields| {
            let mut count = 0;
            let valid = fields.split(',').all(|field| {
                count += 1;
                is_stable_id(field)
            });
            valid && count >= 2
        });
    if valid {
        Ok(())
    } else {
        Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "array schema uses an unknown ordering contract",
        ))
    }
}

fn is_relational_ordering(ordering: &str) -> bool {
    matches!(
        ordering,
        "certified_binding_order"
            | "certified_node_order"
            | "input_manifest_order"
            | "query_order_then_tie_break"
            | "transition_order"
    )
}

fn validate_predicate_ids(values: &[Value]) -> std::result::Result<(), RecoverabilityError> {
    if values.is_empty() {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "owner predicate list is empty",
        ));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "owner predicate id is not a string",
            )
        })?;
        if !string_matches_grammar(value, "P-[A-Z]{2,3}-[0-9]{2}") || !seen.insert(value) {
            return Err(RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "owner predicate id is invalid or duplicated",
            ));
        }
    }
    Ok(())
}

fn semantic_digest_bytes(
    domain: &str,
    value: &Value,
) -> std::result::Result<mfm_ids::DigestBytes, RecoverabilityError> {
    let envelope = CanonicalValue::object([
        ("domain", CanonicalValue::String(domain.to_owned())),
        ("value", json_to_canonical_value(value)?),
    ])
    .map_err(|_| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "annex value could not form a canonical semantic envelope",
        )
    })?;
    Ok(sha256_digest_bytes(
        CanonicalJsonBytes::from_value(&envelope).as_bytes(),
    ))
}

fn require_exact_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> std::result::Result<(), RecoverabilityError> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if object.len() != allowed.len() || object.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            format!("{context} has missing or unknown fields"),
        ));
    }
    Ok(())
}

fn require_exact_key_set(
    object: &Map<String, Value>,
    allowed: &BTreeSet<&str>,
    context: &str,
) -> std::result::Result<(), RecoverabilityError> {
    if object.len() != allowed.len() || object.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            format!("{context} has missing or unknown fields"),
        ));
    }
    Ok(())
}

fn annex_maximum_canonical_bytes(
    root: &Map<String, Value>,
) -> std::result::Result<usize, RecoverabilityError> {
    let limits = required_value(root, "limits", RecoverabilityErrorCode::InvalidAnnex)?
        .as_object()
        .ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex limits are not an object",
            )
        })?;
    let maximum = limits
        .get("max_canonical_json_bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RecoverabilityError::new(
                RecoverabilityErrorCode::InvalidAnnex,
                "annex has no canonical-value byte bound",
            )
        })?;
    let maximum = maximum.parse::<usize>().map_err(|_| {
        RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "annex canonical-value byte bound is invalid",
        )
    })?;
    if maximum == 0 {
        return Err(RecoverabilityError::new(
            RecoverabilityErrorCode::InvalidAnnex,
            "annex canonical-value byte bound is zero",
        ));
    }
    Ok(maximum)
}

#[cfg(test)]
mod entry_point_id_grammar_tests {
    use super::{is_entry_point_id, is_supported_string_grammar};

    #[test]
    fn accepts_only_versioned_mfm_entry_point_ids() {
        assert!(is_supported_string_grammar(
            "mfm.<stable-domain>/<stable-name>@<positive-canonical-u64>"
        ));
        assert!(is_entry_point_id("mfm.portfolio/snapshot@1"));
        assert!(is_entry_point_id("mfm.a/b@1"));
        assert!(is_entry_point_id(
            "mfm.portfolio.v2/snapshot_report@18446744073709551615"
        ));

        for rejected in [
            "portfolio/snapshot@1",
            "mfm.portfolio/snapshot",
            "mfm.portfolio/snapshot@0",
            "mfm.portfolio/snapshot@01",
            "mfm.portfolio/snapshot@18446744073709551616",
            "mfm.Portfolio/snapshot@1",
            "mfm.portfolio/Snapshot@1",
            "mfm.portfolio/nested/snapshot@1",
            "mfm.portfolio-/snapshot@1",
            "mfm.portfolio/snapshot_@1",
            "mfm./snapshot@1",
            "mfm.portfolio/@1",
        ] {
            assert!(!is_entry_point_id(rejected), "{rejected}");
        }
    }
}

#[cfg(test)]
mod reference_path_tests {
    use super::append_json_pointer_token;

    #[test]
    fn appends_exact_rfc_6901_tokens() {
        assert_eq!(append_json_pointer_token("", "field"), "/field");
        assert_eq!(
            append_json_pointer_token("/array/0", "nested/name~suffix"),
            "/array/0/nested~1name~0suffix"
        );
    }
}
