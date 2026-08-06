//! Canonical five-family history contracts for structured Runtime execution.
//!
//! This module owns only strict persisted data and domain-separated identities.
//! Successor legality, cursor derivation, object closure, and atomic append
//! authority belong to `mfm-store`.

use mfm_canonical::limits::{
    MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE, MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES,
    MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE, MAX_PRIOR_RUN_SOURCE_REFERENCES,
    MAX_PRIOR_RUN_SOURCE_RULES,
};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    AccessAttemptId, AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm,
    FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest, InvocationIdentity,
    JournalCommitDigest, JournalRecordHash, OccurrenceId, RequestDigest, RunId,
    RunSemanticStateDigest, SchemaId, SemanticCallId, StableId, StoreEpoch, StoreScopeId,
    TenantScopeId,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Object-type tag for the immutable admitted configuration root.
pub const ADMISSION_CONFIGURATION_OBJECT_TYPE: &str = "structured.admission_configuration";
/// Object-type tag for the immutable admitted context manifest.
pub const ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE: &str = "structured.admission_context_manifest";
/// Object-type tag for the immutable admitted prior-run source manifest.
pub const ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE: &str =
    "structured.admission_prior_run_source_manifest";
/// Object-type tag for the immutable admitted routing policy.
pub const ADMISSION_ROUTING_POLICY_OBJECT_TYPE: &str = "structured.admission_routing_policy";
const PRIOR_RUN_SOURCE_MANIFEST_VERSION: &str = "mfm.prior-run-fact-source-manifest.v1";
const PRIOR_RUN_SOURCE_MANIFEST_SCHEMA_SEED: &[u8] =
    b"mfm.prior-run-fact-source-manifest.schema.v1";
const PRIOR_RUN_FACT_SCANNER_BINDING_OBJECT_TYPE: &str =
    "structured.prior_run_fact_scanner_binding";
const PRIOR_RUN_FACT_SCANNER_BINDING_SCHEMA_SEED: &[u8] =
    b"mfm.prior-run-fact-scanner-binding.schema.v1";

/// Result type for structured journal contract construction.
pub type Result<T> = std::result::Result<T, StructuredJournalError>;

/// Stable failure returned while validating structured journal data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StructuredJournalError {
    /// A value could not be represented as strict float-free canonical JSON.
    #[error("structured journal canonical encoding failed")]
    Canonical,
    /// A checked identity could not be constructed.
    #[error("structured journal identity construction failed")]
    Identity,
    /// Persisted data violates the closed journal shape.
    #[error("structured journal invariant failed: {0}")]
    Invariant(&'static str),
}

/// One exact content-addressed canonical object admitted with a history append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryObject {
    /// Registered public object-type tag.
    pub object_type: StableId,
    /// Exact schema and raw canonical-byte identity.
    pub content_ref: ContentRef,
    /// Exact float-free canonical JSON text.
    pub canonical_json: String,
}

impl HistoryObject {
    /// Constructs an object and proves that its bytes match its content reference.
    pub fn new(
        object_type: StableId,
        schema_id: SchemaId,
        canonical_json: impl AsRef<str>,
    ) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_json_str(canonical_json.as_ref())
            .map_err(|_| StructuredJournalError::Canonical)?;
        let content_ref = ContentRef::new(
            schema_id,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            ),
        )
        .map_err(|_| StructuredJournalError::Identity)?;
        Ok(Self {
            object_type,
            content_ref,
            canonical_json: canonical.as_str().to_owned(),
        })
    }

    /// Revalidates exact canonical bytes and content addressing.
    pub fn validate(&self) -> Result<()> {
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(self.canonical_json.as_bytes())
                .map_err(|_| StructuredJournalError::Canonical)?;
        let expected = ContentRef::new(
            self.content_ref.schema_id().clone(),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            ),
        )
        .map_err(|_| StructuredJournalError::Identity)?;
        if expected != self.content_ref {
            return Err(StructuredJournalError::Invariant(
                "object content reference differs from canonical bytes",
            ));
        }
        Ok(())
    }

    /// Strictly decodes the exact canonical object value.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        self.validate()?;
        serde_json::from_str(&self.canonical_json).map_err(|_| StructuredJournalError::Canonical)
    }
}

/// One exact certified producer class eligible for prior-run fact selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactSourceRule {
    entry_point_operation_id: StableId,
    certified_program_refs: Vec<ContentRef>,
    fact_descriptor_refs: Vec<ContentRef>,
}

impl PriorRunFactSourceRule {
    /// Constructs one canonically ordered producer-eligibility rule.
    pub fn new(
        entry_point_operation_id: StableId,
        mut certified_program_refs: Vec<ContentRef>,
        mut fact_descriptor_refs: Vec<ContentRef>,
    ) -> Result<Self> {
        certified_program_refs.sort();
        fact_descriptor_refs.sort();
        let rule = Self {
            entry_point_operation_id,
            certified_program_refs,
            fact_descriptor_refs,
        };
        rule.validate()?;
        Ok(rule)
    }

    /// Returns the exact producer entry-point operation.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the optional exact certified-program allowlist.
    ///
    /// An empty list admits every certified program under this exact entry
    /// operation while preserving the descriptor allowlist.
    pub fn certified_program_refs(&self) -> &[ContentRef] {
        &self.certified_program_refs
    }

    /// Returns the non-empty exact fact-descriptor allowlist.
    pub fn fact_descriptor_refs(&self) -> &[ContentRef] {
        &self.fact_descriptor_refs
    }

    fn validate(&self) -> Result<()> {
        if self.certified_program_refs.len() > MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE
            || self.fact_descriptor_refs.is_empty()
            || self.fact_descriptor_refs.len() > MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE
            || self
                .certified_program_refs
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .fact_descriptor_refs
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(StructuredJournalError::Invariant(
                "prior-run fact source rule is not bounded canonical material",
            ));
        }
        Ok(())
    }

    fn permits(&self, admission: &RunAdmitted, descriptor_ref: &ContentRef) -> bool {
        self.entry_point_operation_id == admission.entry_point_operation_id
            && (self.certified_program_refs.is_empty()
                || self
                    .certified_program_refs
                    .binary_search(&admission.certified_program_ref)
                    .is_ok())
            && self
                .fact_descriptor_refs
                .binary_search(descriptor_ref)
                .is_ok()
    }
}

/// Strict admitted allowlist for every prior-run fact source this run may read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactSourceManifest {
    version: String,
    rules: Vec<PriorRunFactSourceRule>,
}

impl PriorRunFactSourceManifest {
    /// Constructs one current manifest in canonical entry-operation order.
    pub fn new(mut rules: Vec<PriorRunFactSourceRule>) -> Result<Self> {
        rules.sort_by(|left, right| {
            left.entry_point_operation_id
                .cmp(&right.entry_point_operation_id)
        });
        let manifest = Self {
            version: PRIOR_RUN_SOURCE_MANIFEST_VERSION.to_owned(),
            rules,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Returns the current manifest rules in canonical order.
    pub fn rules(&self) -> &[PriorRunFactSourceRule] {
        &self.rules
    }

    /// Encodes this manifest as the sole accepted admission history object.
    pub fn to_history_object(&self) -> Result<HistoryObject> {
        self.validate()?;
        let canonical = canonical_json(self)?;
        validate_prior_run_source_manifest_bytes(canonical.as_bytes())?;
        HistoryObject::new(
            StableId::new(ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE)
                .map_err(|_| StructuredJournalError::Identity)?,
            prior_run_fact_source_manifest_schema_id()?,
            canonical.as_str(),
        )
    }

    /// Strictly decodes and validates one admitted source-manifest object.
    pub fn from_history_object(object: &HistoryObject) -> Result<Self> {
        validate_prior_run_source_manifest_bytes(object.canonical_json.as_bytes())?;
        object.validate()?;
        if object.object_type.as_str() != ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE
            || object.content_ref.schema_id() != &prior_run_fact_source_manifest_schema_id()?
        {
            return Err(StructuredJournalError::Invariant(
                "admitted prior-run source manifest has the wrong exact contract",
            ));
        }
        let manifest: Self = object.decode()?;
        manifest.validate()?;
        if canonical_json(&manifest)?.as_str() != object.canonical_json {
            return Err(StructuredJournalError::Canonical);
        }
        Ok(manifest)
    }

    /// Returns whether one verified fact producer is admitted by this manifest.
    pub fn permits(&self, admission: &RunAdmitted, descriptor_ref: &ContentRef) -> bool {
        self.rules
            .binary_search_by(|rule| {
                rule.entry_point_operation_id
                    .cmp(&admission.entry_point_operation_id)
            })
            .ok()
            .is_some_and(|index| self.rules[index].permits(admission, descriptor_ref))
    }

    fn validate(&self) -> Result<()> {
        let reference_count = self.rules.iter().try_fold(0_usize, |total, rule| {
            total
                .checked_add(rule.certified_program_refs.len())
                .and_then(|total| total.checked_add(rule.fact_descriptor_refs.len()))
                .ok_or(StructuredJournalError::Invariant(
                    "prior-run fact source manifest reference count overflowed",
                ))
        })?;
        if self.version != PRIOR_RUN_SOURCE_MANIFEST_VERSION
            || self.rules.len() > MAX_PRIOR_RUN_SOURCE_RULES
            || reference_count > MAX_PRIOR_RUN_SOURCE_REFERENCES
            || self
                .rules
                .windows(2)
                .any(|pair| pair[0].entry_point_operation_id >= pair[1].entry_point_operation_id)
        {
            return Err(StructuredJournalError::Invariant(
                "prior-run fact source manifest is not bounded canonical material",
            ));
        }
        self.rules
            .iter()
            .try_for_each(PriorRunFactSourceRule::validate)
    }
}

fn validate_prior_run_source_manifest_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES {
        return Err(StructuredJournalError::Invariant(
            "prior-run fact source manifest exceeds its canonical byte bound",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod prior_run_source_manifest_limit_tests {
    use super::*;

    #[test]
    fn exact_manifest_byte_budget_is_accepted_and_one_over_is_rejected() {
        assert_eq!(
            validate_prior_run_source_manifest_bytes(&vec![
                b'x';
                MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES
            ]),
            Ok(())
        );
        assert_eq!(
            validate_prior_run_source_manifest_bytes(&vec![
                b'x';
                MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES + 1
            ]),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source manifest exceeds its canonical byte bound",
            ))
        );
    }
}

/// Returns the exact schema identity of the sole admitted source manifest.
pub fn prior_run_fact_source_manifest_schema_id() -> Result<SchemaId> {
    SchemaId::new(
        "mfm.prior-run-fact-source-manifest",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PRIOR_RUN_SOURCE_MANIFEST_SCHEMA_SEED),
    )
    .map_err(|_| StructuredJournalError::Identity)
}

/// Store-qualified dense fact-publication frontier for one tenant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenantFactFrontier {
    /// Qualified store lineage containing the publication sequence.
    pub store_scope_id: StoreScopeId,
    /// Qualified writer epoch containing the publication sequence.
    pub store_epoch: StoreEpoch,
    /// Exact tenant whose prior-run facts are serialized.
    pub tenant_scope_id: TenantScopeId,
    /// Dense publication order; zero denotes the empty frontier.
    pub fact_order: u64,
}

impl TenantFactFrontier {
    /// Constructs the current frontier for one exact store and tenant.
    pub fn new(
        store_scope_id: StoreScopeId,
        store_epoch: StoreEpoch,
        tenant_scope_id: TenantScopeId,
        fact_order: u64,
    ) -> Self {
        Self {
            store_scope_id,
            store_epoch,
            tenant_scope_id,
            fact_order,
        }
    }

    /// Returns the next dense publication frontier.
    pub fn next_publication(&self) -> Result<Self> {
        let fact_order =
            self.fact_order
                .checked_add(1)
                .ok_or(StructuredJournalError::Invariant(
                    "tenant fact publication order overflowed",
                ))?;
        Ok(Self {
            store_scope_id: self.store_scope_id.clone(),
            store_epoch: self.store_epoch,
            tenant_scope_id: self.tenant_scope_id.clone(),
            fact_order,
        })
    }
}

/// Closed tenant-fact coordinate attached to every append candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TenantFactCoordinate {
    /// The append neither publishes nor authorizes a prior-run fact read.
    #[default]
    None,
    /// A successful non-empty transition publishes the next dense tenant batch.
    FactPublication {
        /// New frontier after this publication commits.
        frontier: TenantFactFrontier,
    },
    /// An exact prior-run fact-selection Read captures the current frontier.
    FactSelectionBarrier {
        /// Current frontier atomically observed with authorization persistence.
        frontier: TenantFactFrontier,
    },
}

impl TenantFactCoordinate {
    /// Returns the coordinate's exact frontier, when present.
    pub const fn frontier(&self) -> Option<&TenantFactFrontier> {
        match self {
            Self::None => None,
            Self::FactPublication { frontier } | Self::FactSelectionBarrier { frontier } => {
                Some(frontier)
            }
        }
    }
}

/// Immutable public certificate for the store-local prior-run fact scanner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactScannerBindingCertificate {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    tenant_scope_id: TenantScopeId,
    admitted_source_manifest_ref: ContentRef,
    selector_contract_ref: ContentRef,
    capability_contract_ref: ContentRef,
    capability_implementation_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
}

impl PriorRunFactScannerBindingCertificate {
    /// Constructs one exact secret-free scanner binding certificate.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope_id: StoreScopeId,
        store_epoch: StoreEpoch,
        tenant_scope_id: TenantScopeId,
        admitted_source_manifest_ref: ContentRef,
        selector_contract_ref: ContentRef,
        capability_contract_ref: ContentRef,
        capability_implementation_ref: ContentRef,
        adapter_contract_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
    ) -> Self {
        Self {
            store_scope_id,
            store_epoch,
            tenant_scope_id,
            admitted_source_manifest_ref,
            selector_contract_ref,
            capability_contract_ref,
            capability_implementation_ref,
            adapter_contract_ref,
            adapter_implementation_ref,
        }
    }

    /// Encodes this certificate as its fixed content-addressed history object.
    pub fn to_history_object(&self) -> Result<HistoryObject> {
        HistoryObject::new(
            StableId::new(PRIOR_RUN_FACT_SCANNER_BINDING_OBJECT_TYPE)
                .map_err(|_| StructuredJournalError::Identity)?,
            prior_run_fact_scanner_binding_schema_id()?,
            canonical_json(self)?.as_str(),
        )
    }

    /// Strictly decodes one exact scanner binding certificate object.
    pub fn from_history_object(object: &HistoryObject) -> Result<Self> {
        object.validate()?;
        if object.object_type.as_str() != PRIOR_RUN_FACT_SCANNER_BINDING_OBJECT_TYPE
            || object.content_ref.schema_id() != &prior_run_fact_scanner_binding_schema_id()?
        {
            return Err(StructuredJournalError::Invariant(
                "prior-run fact scanner binding has the wrong exact contract",
            ));
        }
        let certificate: Self = object.decode()?;
        if canonical_json(&certificate)?.as_str() != object.canonical_json {
            return Err(StructuredJournalError::Canonical);
        }
        Ok(certificate)
    }

    /// Returns the exact store lineage.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the exact writer epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Returns the exact consumer tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact admitted source manifest.
    pub const fn admitted_source_manifest_ref(&self) -> &ContentRef {
        &self.admitted_source_manifest_ref
    }

    /// Returns the exact fixed selector contract.
    pub const fn selector_contract_ref(&self) -> &ContentRef {
        &self.selector_contract_ref
    }

    /// Returns the exact semantic capability contract.
    pub const fn capability_contract_ref(&self) -> &ContentRef {
        &self.capability_contract_ref
    }

    /// Returns the exact qualified capability implementation.
    pub const fn capability_implementation_ref(&self) -> &ContentRef {
        &self.capability_implementation_ref
    }

    /// Returns the exact semantic adapter contract.
    pub const fn adapter_contract_ref(&self) -> &ContentRef {
        &self.adapter_contract_ref
    }

    /// Returns the exact qualified adapter implementation.
    pub const fn adapter_implementation_ref(&self) -> &ContentRef {
        &self.adapter_implementation_ref
    }
}

/// Returns the fixed schema identity of scanner binding certificates.
pub fn prior_run_fact_scanner_binding_schema_id() -> Result<SchemaId> {
    SchemaId::new(
        "mfm.prior-run-fact-scanner-binding",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PRIOR_RUN_FACT_SCANNER_BINDING_SCHEMA_SEED),
    )
    .map_err(|_| StructuredJournalError::Identity)
}

/// Sole positive completeness statement carried by a selected-fact response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorRunFactCompletenessMode {
    /// Every dense publication through the authorization barrier was verified.
    CompleteThroughAuthorizationFrontier,
}

/// Exact authorization and store barrier attested by one completed fact scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactScanAttestation {
    /// Exact immutable store/epoch/tenant frontier reached by the scan.
    pub frontier: TenantFactFrontier,
    /// Consumer tenant captured from the verified admission.
    pub tenant_scope_id: TenantScopeId,
    /// Exact newly appended authorization that minted scan authority.
    pub authorization_ref: RecordRef,
    /// Exact secret-free scanner physical-binding certificate.
    pub physical_binding_ref: ContentRef,
    /// Sole positive completeness mode.
    pub completeness_mode: PriorRunFactCompletenessMode,
}

/// One fully verified source fact retained in a query result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedPriorRunFact {
    /// Dense tenant publication containing this fact.
    pub publication_frontier: TenantFactFrontier,
    /// Exact producer transition routed by that publication.
    pub producer_transition_ref: RecordRef,
    /// Producer's exact certified-program authority.
    pub producer_certified_program_ref: ContentRef,
    /// Producer's exact entry-point operation.
    pub producer_entry_point_operation_id: StableId,
    /// Dense fact ordinal within the producer transition.
    pub emission_ordinal: u32,
    /// Exact certified descriptor selected by the query.
    pub descriptor_ref: ContentRef,
    /// Exact typed subject reference.
    pub subject: TypedValueRef,
    /// Exact typed response reference.
    pub response: TypedValueRef,
    /// Content-addressed producer claim closure.
    pub claim_ref: ContentRef,
    /// Producer-independent exact descriptor/subject/response identity.
    pub content_identity: FactContentIdentityDigest,
    /// Producer-transition-bound deterministic logical identity.
    pub fact_identity: FactLogicalIdentityDigest,
    /// Exact retained canonical subject bytes.
    pub subject_canonical_json: String,
    /// Exact retained canonical response bytes.
    pub response_canonical_json: String,
    /// Exact retained canonical claim-closure bytes.
    pub claim_canonical_json: String,
}

/// Final deterministic selection for one authored query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactQueryResult {
    /// Zero-based query ordinal in the exact request.
    pub query_ordinal: u32,
    /// Exact content identity of that authored query.
    pub query_ref: ContentRef,
    /// Selected facts in the query's frozen deterministic order.
    pub selected: Vec<SelectedPriorRunFact>,
}

/// One self-contained complete prior-run fact-selection result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactSelectionResponse {
    /// Sole current response contract version.
    pub version: String,
    /// Domain-separated digest of the exact state-authored request.
    pub request_digest: FactQueryDigest,
    /// Exact admitted producer allowlist used by the scan.
    pub admitted_source_manifest_ref: ContentRef,
    /// Exact fixed selector implementation contract.
    pub selector_contract_ref: ContentRef,
    /// Authorization and completeness evidence.
    pub attestation: PriorRunFactScanAttestation,
    /// One deterministic result per authored query, in authored order.
    pub query_results: Vec<PriorRunFactQueryResult>,
}

impl PriorRunFactSelectionResponse {
    /// Sole current canonical response version.
    pub const VERSION: &'static str = "mfm.prior-run-fact-selection-response.v1";
}

/// Exact producer-independent typed retained value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedValueRef {
    /// Exact retained-value contract required by that slot.
    pub contract_ref: ContentRef,
    /// Exact content-addressed retained value.
    pub value_ref: ContentRef,
}

/// Exact typed value bytes bound to one certified lexical slot.
///
/// Aggregate payload bytes do not carry structural authority. When a value is
/// produced by Match-arm merge or fan-out-lane completion, [`structural_origin`]
/// binds the selected arm/lane independently of payload equality so two
/// byte-identical products remain distinguishable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LexicalValueRef {
    /// Canonical identity of the certified lexical slot recipe.
    pub slot_ref: ContentRef,
    /// Exact typed retained value.
    #[serde(flatten)]
    pub value: TypedValueRef,
    /// Selected Match-arm or fan-out-lane producer identity, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_origin: Option<StructuralValueOrigin>,
}

impl LexicalValueRef {
    /// Constructs a binding without structural Match/FanOut origin.
    pub fn new(slot_ref: ContentRef, value: TypedValueRef) -> Self {
        Self {
            slot_ref,
            value,
            structural_origin: None,
        }
    }

    /// Constructs a binding with an explicit structural origin.
    pub fn with_origin(
        slot_ref: ContentRef,
        value: TypedValueRef,
        structural_origin: StructuralValueOrigin,
    ) -> Self {
        Self {
            slot_ref,
            value,
            structural_origin: Some(structural_origin),
        }
    }
}

/// Nominal origin of a Match-arm or fan-out-lane product.
///
/// Fields are independent of payload bytes so identical values from distinct
/// arms or lanes remain distinguishable by group path, key, ordinal, contract,
/// and source reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuralValueOrigin {
    /// Selected arm of an exhaustive Match merge.
    MatchArm {
        /// Structural path of the owning Match.
        match_path_ref: ContentRef,
        /// Declaration ordinal of the selected arm.
        arm_ordinal: u32,
        /// Diagnostic arm key (not used for identity ordering).
        arm_key: StableId,
        /// Value contract of the selected arm product.
        value_contract_ref: ContentRef,
        /// Exact source LexicalValueRef slot that produced the arm value.
        source_slot_ref: ContentRef,
        /// Exact source value content identity.
        source_value_ref: ContentRef,
    },
    /// Completed fan-out lane outcome wrapper.
    FanOutLane {
        /// Structural path of the owning fan-out group.
        group_path_ref: ContentRef,
        /// Declaration ordinal of the lane.
        lane_ordinal: u32,
        /// Diagnostic lane key (not used for identity ordering).
        lane_key: StableId,
        /// Success or failure contract of the lane outcome.
        outcome_contract_ref: ContentRef,
        /// Exact source LexicalValueRef slot that produced the lane body value.
        source_slot_ref: ContentRef,
        /// Exact source value content identity.
        source_value_ref: ContentRef,
    },
}

/// One state-produced durable fact with its complete certified claim closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedFactRef {
    /// Dense emission ordinal inside this transition.
    pub emission_ordinal: u32,
    /// Exact certified homogeneous fact slot.
    pub fact_slot_ordinal: u32,
    /// Exact certified fact descriptor.
    pub descriptor_ref: ContentRef,
    /// Exact typed public fact subject.
    pub subject: TypedValueRef,
    /// Exact typed public fact response.
    pub response: TypedValueRef,
    /// Content-addressed claim binding descriptor, subject, and response.
    pub claim_ref: ContentRef,
}

/// Exact reference to one assigned record in a run prefix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordRef {
    /// Run containing the record.
    pub run_id: RunId,
    /// One-based atomic append sequence.
    pub run_sequence: u64,
    /// Dense record ordinal inside the append.
    pub ordinal: u32,
    /// Domain-separated hash of the exact assigned record.
    pub record_hash: JournalRecordHash,
}

/// Exact per-run physical journal head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalHead {
    /// One-based atomic append sequence.
    pub run_sequence: u64,
    /// Domain-separated digest of the complete assigned append.
    pub commit_digest: JournalCommitDigest,
}

/// Fold-derived semantic head, independent of intervening audit records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticHead {
    /// Admission-root semantic genesis.
    Genesis {
        /// Exact admission record.
        admission_ref: RecordRef,
        /// Canonical semantic state after admission normalization.
        semantic_state_digest: RunSemanticStateDigest,
    },
    /// Most recent committed state transition.
    Transition {
        /// Exact semantic transition record.
        transition_ref: RecordRef,
        /// Canonical semantic state after the transition.
        semantic_state_digest: RunSemanticStateDigest,
    },
}

impl SemanticHead {
    /// Returns the current canonical semantic-state digest.
    pub const fn semantic_state_digest(&self) -> &RunSemanticStateDigest {
        match self {
            Self::Genesis {
                semantic_state_digest,
                ..
            }
            | Self::Transition {
                semantic_state_digest,
                ..
            } => semantic_state_digest,
        }
    }
}

/// Closed external-access kind recorded by Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    /// Non-mutating bounded external observation.
    Read,
    /// Bounded operation that may enter a mutation boundary.
    Effect,
}

/// Exact audit projections bound to one certified program authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedProgramAuditRefs {
    /// Canonical authored program.
    pub authored_program_ref: ContentRef,
    /// Canonical fully expanded program.
    pub expanded_program_ref: ContentRef,
    /// Exact trusted expansion profile.
    pub expansion_profile_ref: ContentRef,
    /// Exact expansion proof.
    pub expansion_proof_ref: ContentRef,
    /// Exact policy coverage proof.
    pub policy_coverage_proof_ref: ContentRef,
    /// Exact semantic component manifest.
    pub component_manifest_ref: ContentRef,
    /// Exact secret-free implementation manifest.
    pub implementation_manifest_ref: ContentRef,
}

/// Exact immutable non-secret admission roots outside the certified program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionMaterialRefs {
    /// Canonical deployment-selected configuration object.
    pub configuration_ref: ContentRef,
    /// Canonical immutable context-root manifest.
    pub context_manifest_ref: ContentRef,
    /// Canonical admitted prior-run source manifest.
    pub prior_run_source_manifest_ref: ContentRef,
    /// Immutable secret-free routing policy selected for this run.
    pub routing_policy_ref: ContentRef,
    /// Canonically ordered stable resource-lineage contracts available to the run.
    pub stable_resource_lineage_contract_refs: Vec<ContentRef>,
}

/// Sole immutable root record of one admitted structured run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunAdmitted {
    /// Qualified store lineage that owns this run.
    pub store_scope_id: StoreScopeId,
    /// Authoritative writer epoch that admitted this run.
    pub store_epoch: StoreEpoch,
    /// Deterministic run identity.
    pub run_id: RunId,
    /// App-authorized tenant scope.
    pub tenant_scope_id: TenantScopeId,
    /// Caller invocation identity.
    pub invocation_identity: InvocationIdentity,
    /// Exact qualified entry-point operation.
    pub entry_point_operation_id: StableId,
    /// Sole certified-program authority reference.
    pub certified_program_ref: ContentRef,
    /// Content-addressed certified-program root stored separately from its component closure.
    pub certified_program_root_ref: ContentRef,
    /// Exact registry-selected admission policy.
    pub qualified_entry_point_admission_policy_ref: ContentRef,
    /// Reviewed audit projections that must equal the certified root.
    pub audit_refs: CertifiedProgramAuditRefs,
    /// Exact immutable configuration, context, source, routing, and lineage roots.
    pub admission_material_refs: AdmissionMaterialRefs,
    /// Declaration-ordered exact admission root values.
    pub initial_bindings: Vec<LexicalValueRef>,
    /// Canonical semantic-state digest after admission normalization.
    pub genesis_semantic_state_digest: RunSemanticStateDigest,
}

/// One nominal state outcome accepted by the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StateOutcomeRef {
    /// Exact successful state-output binding.
    Success(LexicalValueRef),
    /// Exact typed state-failure binding.
    Failure(LexicalValueRef),
}

/// Complete semantic transition of one exact executable state occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateTransitionCommitted {
    /// Exact executable occurrence.
    pub occurrence_id: OccurrenceId,
    /// Canonical reference of the normalized occurrence path.
    pub occurrence_path_ref: ContentRef,
    /// Stable authored/injected semantic call identity.
    pub semantic_call_id: SemanticCallId,
    /// Exact fold-derived state input consumed by the callback.
    pub input: LexicalValueRef,
    /// Exact normal observation consumed by Read/Effect settlement.
    pub consumed_observation_ref: Option<RecordRef>,
    /// Store-constructed nominal state outcome.
    pub outcome_ref: ContentRef,
    /// Exact selected value inside the nominal outcome object.
    pub outcome: StateOutcomeRef,
    /// Declaration-ordered facts produced by this callback result.
    pub facts: Vec<CommittedFactRef>,
    /// Semantic state before this transition.
    pub before_semantic_state_digest: RunSemanticStateDigest,
    /// Semantic state after callback-free normalization.
    pub after_semantic_state_digest: RunSemanticStateDigest,
}

/// Durable authorization of exactly one current structured access occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAccessAuthorized {
    /// Kernel-derived immutable attempt identity.
    pub access_attempt_id: AccessAttemptId,
    /// Fold-derived ordinal; the first attempt is zero.
    pub attempt_ordinal: u64,
    /// Exact current executable occurrence.
    pub occurrence_id: OccurrenceId,
    /// Canonical reference of the normalized occurrence path.
    pub occurrence_path_ref: ContentRef,
    /// Stable semantic call identity.
    pub semantic_call_id: SemanticCallId,
    /// Exact current producer-bound input used to author this access request.
    pub state_input_ref: LexicalValueRef,
    /// Read or Effect protocol kind.
    pub access_kind: AccessKind,
    /// Semantic anchor unchanged by intervening audit records.
    pub semantic_head: SemanticHead,
    /// Immutable store lineage containing the authorization.
    pub store_scope_id: StoreScopeId,
    /// Authoritative writer epoch containing the authorization.
    pub store_epoch: StoreEpoch,
    /// Authenticated tenant admitted for the run.
    pub tenant_scope_id: TenantScopeId,
    /// Immutable routing policy admitted for the run.
    pub admitted_routing_policy_ref: ContentRef,
    /// Minimum non-rollback lineage head required by a refreshed access.
    pub minimum_lineage_head_ref: Option<ContentRef>,
    /// Exact semantic capability contract.
    pub capability_contract_ref: ContentRef,
    /// Exact secret-free capability implementation contract.
    pub capability_implementation_ref: ContentRef,
    /// Exact semantic adapter contract.
    pub adapter_contract_ref: ContentRef,
    /// Exact secret-free adapter implementation contract.
    pub adapter_implementation_ref: ContentRef,
    /// Immutable typed request authored before authorization.
    pub request: TypedValueRef,
    /// Digest of the canonical request bytes.
    pub request_digest: RequestDigest,
    /// Secret-free exact physical binding certificate.
    pub physical_binding_ref: ContentRef,
    /// Stable admitted resource lineage for refreshable Effects only.
    pub stable_resource_lineage_contract_ref: Option<ContentRef>,
}

/// Closed observed completion of one authorized access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationOutcome {
    /// Schema-valid typed returned value.
    Returned {
        /// Exact state-consumable returned value.
        value: TypedValueRef,
    },
    /// Reviewed definite state-facing failure.
    SafeFailure {
        /// Exact state-consumable safe-failure value.
        value: TypedValueRef,
    },
    /// Qualified proof that the old Effect binding did not enter.
    SupersededBeforeEntry {
        /// Exact public non-rollback lineage head.
        public_lineage_head_ref: ContentRef,
        /// Exact typed refresh evidence retained for audit.
        evidence_ref: ContentRef,
    },
    /// Effect entry may have happened and completion is unavailable.
    EntryUnknown {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
    /// Committed integrity evidence that blocks semantic progress.
    IntegrityFault {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
}

/// Durable observation linked to one exact authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAccessObserved {
    /// Exact authorization being resolved.
    pub authorization_ref: RecordRef,
    /// Immutable attempt identity copied from the authorization.
    pub access_attempt_id: AccessAttemptId,
    /// Exact closed completion.
    pub outcome: ObservationOutcome,
}

/// Sole terminal record of one structured run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunClosed {
    /// Exact content-addressed nominal `OperationOutcome` object.
    pub outcome_ref: ContentRef,
}

/// Closed five-family structured run-history algebra.
// Append batches are strictly bounded; keeping the closed payloads inline
// avoids a separate heap allocation for every append, fold, and replay record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RunRecord {
    /// Immutable run admission.
    RunAdmitted(RunAdmitted),
    /// One semantic state transition.
    StateTransitionCommitted(StateTransitionCommitted),
    /// One external-access authorization.
    ExternalAccessAuthorized(ExternalAccessAuthorized),
    /// One linked external-access observation.
    ExternalAccessObserved(ExternalAccessObserved),
    /// Sole atomic root closure.
    RunClosed(RunClosed),
}

/// Closed logical identity used for exact-content idempotency.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordLogicalKey {
    /// Sole admission for one run.
    Admission {
        /// Exact admitted run.
        run_id: RunId,
    },
    /// Sole settlement of one executable occurrence.
    Transition {
        /// Exact settled occurrence.
        occurrence_id: OccurrenceId,
    },
    /// One immutable access attempt.
    Authorization {
        /// Exact immutable access attempt.
        access_attempt_id: AccessAttemptId,
    },
    /// Sole observation of one access attempt.
    Observation {
        /// Exact immutable access attempt.
        access_attempt_id: AccessAttemptId,
    },
    /// Sole closure for one run.
    Closure {
        /// Exact closed run.
        run_id: RunId,
    },
}

impl RunRecord {
    /// Returns the exact logical idempotency key of this record.
    pub fn logical_key(&self, assigned_run_id: &RunId) -> RecordLogicalKey {
        match self {
            Self::RunAdmitted(_) => RecordLogicalKey::Admission {
                run_id: assigned_run_id.clone(),
            },
            Self::StateTransitionCommitted(record) => RecordLogicalKey::Transition {
                occurrence_id: record.occurrence_id.clone(),
            },
            Self::ExternalAccessAuthorized(record) => RecordLogicalKey::Authorization {
                access_attempt_id: record.access_attempt_id.clone(),
            },
            Self::ExternalAccessObserved(record) => RecordLogicalKey::Observation {
                access_attempt_id: record.access_attempt_id.clone(),
            },
            Self::RunClosed(_) => RecordLogicalKey::Closure {
                run_id: assigned_run_id.clone(),
            },
        }
    }
}

/// One exact unassigned atomic append proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitCandidate {
    /// Target run.
    pub run_id: RunId,
    /// Exact current head, or genesis for admission.
    pub expected_head: Option<JournalHead>,
    /// Stable physical acknowledgement-resolution identity.
    pub append_request_id: AppendRequestId,
    /// Exact tenant-fact publication or authorization-barrier coordinate.
    pub tenant_fact_coordinate: TenantFactCoordinate,
    /// One record, or one semantic record plus adjacent closure.
    pub records: Vec<RunRecord>,
    /// Exact objects admitted or byte-identically resolved with this append.
    pub objects: Vec<HistoryObject>,
}

/// One record after sequence, ordinal, and hash assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedRecord {
    /// Exact assigned record reference.
    pub record_ref: RecordRef,
    /// Exact closed record payload.
    pub record: RunRecord,
}

/// One complete assigned atomic append envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedBatch {
    /// Qualified store lineage.
    pub store_scope_id: StoreScopeId,
    /// Qualified store writer epoch.
    pub store_epoch: StoreEpoch,
    /// Exact predecessor expected by the candidate.
    pub predecessor: Option<JournalHead>,
    /// Stable acknowledgement-resolution identity.
    pub append_request_id: AppendRequestId,
    /// Exact tenant-fact publication or authorization-barrier coordinate.
    pub tenant_fact_coordinate: TenantFactCoordinate,
    /// Complete candidate digest before record assignment.
    pub candidate_digest: ContentDigest,
    /// Assigned records in dense ordinal order.
    pub records: Vec<AssignedRecord>,
    /// Objects atomically admitted or verified by this batch.
    pub objects: Vec<HistoryObject>,
    /// New exact run head.
    pub head: JournalHead,
}

/// Returns canonical float-free bytes for a structured journal value.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(|_| StructuredJournalError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| StructuredJournalError::Canonical)
}

/// Returns a domain-separated raw content digest for canonical structured data.
pub fn domain_content_digest<T: Serialize>(domain: &str, value: &T) -> Result<ContentDigest> {
    let canonical = canonical_json(value)?;
    let mut preimage = domain.as_bytes().to_vec();
    preimage.push(0);
    preimage.extend_from_slice(canonical.as_bytes());
    Ok(ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(&preimage),
    ))
}

/// Derives one immutable access-attempt identity from its complete preimage.
pub fn derive_access_attempt_id<T: Serialize>(preimage: &T) -> Result<AccessAttemptId> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-access-attempt.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(AccessAttemptId::from_digest(sha256_digest_bytes(&bytes)))
}

/// Derives the exact hash of an assigned record.
pub fn derive_record_hash<T: Serialize>(preimage: &T) -> Result<JournalRecordHash> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-record.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(JournalRecordHash::from_digest(sha256_digest_bytes(&bytes)))
}

/// Derives the exact digest of an assigned atomic append.
pub fn derive_commit_digest<T: Serialize>(preimage: &T) -> Result<JournalCommitDigest> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-commit.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(JournalCommitDigest::from_digest(sha256_digest_bytes(
        &bytes,
    )))
}

#[cfg(test)]
#[path = "structured_tests.rs"]
mod tests;
