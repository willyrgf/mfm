//! Canonical five-family history contracts for structured Runtime execution.
//!
//! This module owns only strict persisted data and domain-separated identities.
//! Successor legality, cursor derivation, object closure, and atomic append
//! authority belong to `mfm-store`.

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    AccessAttemptId, AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm,
    FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest, InvocationIdentity,
    JournalCommitDigest, JournalRecordHash, OccurrenceId, RequestDigest, RunId,
    RunSemanticStateDigest, SemanticCallId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_program_derive::PersistedSchema;
use mfm_values::{
    CanonicalJsonPersistedSchema, FieldDescriptor, LiteralValue, MfmValue, PersistedObjectPayload,
    PersistedSchema, SchemaIdentity, SchemaKind, SchemaShape, SequenceOrdering, StringGrammar,
};
use serde::{Deserialize, Serialize};

/// Maximum rules in one prior-run source contract.
pub const MAX_PRIOR_RUN_SOURCE_RULES: usize = 1024;

/// Maximum certified programs named by one prior-run source rule.
pub const MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE: usize = 4096;

/// Maximum fact descriptors named by one prior-run source rule.
pub const MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE: usize = 4096;

/// Maximum total references in one prior-run source contract.
pub const MAX_PRIOR_RUN_SOURCE_REFERENCES: usize = 65536;

/// Maximum bytes in one canonical prior-run source manifest.
pub const MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES: usize = 16777216;

/// Maximum records in one atomic structured-history append.
pub const MAX_APPEND_RECORDS: usize = 2;

/// Maximum objects in one atomic structured-history append.
pub const MAX_APPEND_OBJECTS: usize = 65_536;

/// Object-type tag for the immutable admitted configuration root.
pub const ADMISSION_CONFIGURATION_OBJECT_TYPE: &str = "structured.admission_configuration";
/// Object-type tag for the immutable admitted context manifest.
pub const ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE: &str = "structured.admission_context_manifest";
/// Object-type tag for the immutable admitted prior-run source manifest.
pub const ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE: &str =
    "structured.admission_prior_run_source_manifest";
/// Object-type tag for the immutable admitted routing policy.
pub const ADMISSION_ROUTING_POLICY_OBJECT_TYPE: &str = "structured.admission_routing_policy";
/// Object-type tag for a canonical value owned by one exact [`MfmValue`] schema.
pub const TYPED_VALUE_OBJECT_TYPE: &str = "structured.typed_value";
const PRIOR_RUN_SOURCE_MANIFEST_VERSION: &str = "mfm.prior-run-fact-source-manifest.v1";
const PRIOR_RUN_FACT_SCANNER_BINDING_OBJECT_TYPE: &str =
    "structured.prior_run_fact_scanner_binding";

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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

    /// Builds one history object from its typed owner.
    ///
    /// The owner supplies both the object type and the schema identity, so a
    /// caller cannot pair typed bytes with a foreign object kind or a schema
    /// identity that does not describe them.
    pub fn from_persisted<T: PersistedObjectPayload>(value: &T) -> Result<Self> {
        let canonical = value
            .encode_canonical()
            .map_err(|_| StructuredJournalError::Canonical)?;
        let content_ref = value
            .content_ref()
            .map_err(|_| StructuredJournalError::Identity)?;
        Ok(Self {
            object_type: T::object_type().map_err(|_| StructuredJournalError::Identity)?,
            content_ref,
            canonical_json: canonical.as_str().to_owned(),
        })
    }

    /// Strictly decodes this object as one typed owner.
    ///
    /// Both the retained object type and the retained schema identity must
    /// match the owner before its bytes are decoded.
    pub fn decode_persisted<T: PersistedObjectPayload>(&self) -> Result<T> {
        self.validate()?;
        let object_type = T::object_type().map_err(|_| StructuredJournalError::Identity)?;
        let schema_id = T::schema_id().map_err(|_| StructuredJournalError::Identity)?;
        if self.object_type != object_type || self.content_ref.schema_id() != &schema_id {
            return Err(StructuredJournalError::Invariant(
                "object type or schema identity differs from the requested owner",
            ));
        }
        T::decode_canonical(self.canonical_json.as_bytes())
            .map_err(|_| StructuredJournalError::Canonical)
    }

    /// Decodes a stored Runtime value through its exact concrete value owner.
    pub fn decode_mfm_value<T: MfmValue>(&self) -> Result<T> {
        self.validate()?;
        let object_type =
            StableId::new(TYPED_VALUE_OBJECT_TYPE).map_err(|_| StructuredJournalError::Identity)?;
        let descriptor = T::schema_descriptor().map_err(|_| StructuredJournalError::Canonical)?;
        let schema_id = descriptor
            .schema_id()
            .map_err(|_| StructuredJournalError::Canonical)?;
        if self.object_type != object_type || self.content_ref.schema_id() != &schema_id {
            return Err(StructuredJournalError::Invariant(
                "typed value owner does not match object identity",
            ));
        }
        descriptor
            .identity
            .validate_canonical_value(self.canonical_json.as_bytes())
            .map_err(|_| StructuredJournalError::Canonical)?;
        let value: T = serde_json::from_str(&self.canonical_json)
            .map_err(|_| StructuredJournalError::Canonical)?;
        let encoded =
            serde_json::to_string(&value).map_err(|_| StructuredJournalError::Canonical)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded)
            .map_err(|_| StructuredJournalError::Canonical)?;
        if canonical.as_str() != self.canonical_json {
            return Err(StructuredJournalError::Invariant(
                "typed value is not the owner's exact encoding",
            ));
        }
        Ok(value)
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
        validate_prior_run_source_rule_counts(
            self.certified_program_refs.len(),
            self.fact_descriptor_refs.len(),
        )?;
        if self
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
        let encoded = serde_json::to_vec(self).map_err(|_| StructuredJournalError::Canonical)?;
        validate_prior_run_source_manifest_bytes(&encoded)?;
        let reference_count = self.rules.iter().try_fold(0_usize, |total, rule| {
            total
                .checked_add(rule.certified_program_refs.len())
                .and_then(|total| total.checked_add(rule.fact_descriptor_refs.len()))
                .ok_or(StructuredJournalError::Invariant(
                    "prior-run fact source manifest reference count overflowed",
                ))
        })?;
        validate_prior_run_source_manifest_counts(self.rules.len(), reference_count)?;
        if self.version != PRIOR_RUN_SOURCE_MANIFEST_VERSION
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

fn validate_prior_run_source_rule_counts(
    certified_program_count: usize,
    fact_descriptor_count: usize,
) -> Result<()> {
    if certified_program_count > MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE
        || fact_descriptor_count == 0
        || fact_descriptor_count > MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE
    {
        return Err(StructuredJournalError::Invariant(
            "prior-run fact source rule is not bounded canonical material",
        ));
    }
    Ok(())
}

fn validate_prior_run_source_manifest_counts(
    rule_count: usize,
    reference_count: usize,
) -> Result<()> {
    if rule_count > MAX_PRIOR_RUN_SOURCE_RULES || reference_count > MAX_PRIOR_RUN_SOURCE_REFERENCES
    {
        return Err(StructuredJournalError::Invariant(
            "prior-run fact source manifest is not bounded canonical material",
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

#[cfg(test)]
mod prior_run_source_count_limit_tests {
    use super::*;

    #[test]
    fn exact_rule_counts_are_accepted_and_one_over_is_rejected() {
        assert_eq!(
            validate_prior_run_source_rule_counts(MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE, 1,),
            Ok(())
        );
        assert_eq!(
            validate_prior_run_source_rule_counts(MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE + 1, 1,),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source rule is not bounded canonical material",
            ))
        );
        assert_eq!(
            validate_prior_run_source_rule_counts(0, MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE,),
            Ok(())
        );
        assert_eq!(
            validate_prior_run_source_rule_counts(0, MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE + 1,),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source rule is not bounded canonical material",
            ))
        );
        assert_eq!(
            validate_prior_run_source_rule_counts(0, 0),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source rule is not bounded canonical material",
            ))
        );
    }

    #[test]
    fn exact_manifest_counts_are_accepted_and_one_over_is_rejected() {
        assert_eq!(
            validate_prior_run_source_manifest_counts(
                MAX_PRIOR_RUN_SOURCE_RULES,
                MAX_PRIOR_RUN_SOURCE_REFERENCES,
            ),
            Ok(())
        );
        assert_eq!(
            validate_prior_run_source_manifest_counts(
                MAX_PRIOR_RUN_SOURCE_RULES + 1,
                MAX_PRIOR_RUN_SOURCE_REFERENCES,
            ),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source manifest is not bounded canonical material",
            ))
        );
        assert_eq!(
            validate_prior_run_source_manifest_counts(
                MAX_PRIOR_RUN_SOURCE_RULES,
                MAX_PRIOR_RUN_SOURCE_REFERENCES + 1,
            ),
            Err(StructuredJournalError::Invariant(
                "prior-run fact source manifest is not bounded canonical material",
            ))
        );
    }
}

impl PersistedSchema for PriorRunFactSourceManifest {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        let reference = SchemaShape::content_ref()?;
        let rule = SchemaShape::named_struct(vec![
            FieldDescriptor::required(
                "certified_program_refs",
                bounded_reference_sequence(
                    &reference,
                    MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE as u32,
                ),
            ),
            FieldDescriptor::required(
                "entry_point_operation_id",
                SchemaShape::identity_string(StringGrammar::StableId, 256),
            ),
            FieldDescriptor::required(
                "fact_descriptor_refs",
                bounded_reference_sequence(
                    &reference,
                    MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE as u32,
                ),
            ),
        ])?;
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.prior-run-fact-source-manifest",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![
                FieldDescriptor::required(
                    "rules",
                    SchemaShape::BoundedSequence {
                        element: Box::new(rule),
                        minimum_items: 0,
                        maximum_items: MAX_PRIOR_RUN_SOURCE_RULES as u32,
                        ordering: SequenceOrdering::CanonicalAscending,
                        unique: true,
                    },
                ),
                FieldDescriptor::required(
                    "version",
                    SchemaShape::Literal(LiteralValue::String(
                        PRIOR_RUN_SOURCE_MANIFEST_VERSION.to_owned(),
                    )),
                ),
            ])?,
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        PriorRunFactSourceManifest::validate(self)
            .map_err(|error| mfm_values::ValueError::Descriptor(error.to_string()))
    }
}

impl PersistedObjectPayload for PriorRunFactSourceManifest {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new(ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE)
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

/// Returns the exact serialized shape of one [`TypedValueRef`].
pub fn typed_value_ref_shape() -> mfm_values::Result<SchemaShape> {
    let reference = SchemaShape::content_ref()?;
    SchemaShape::named_struct(vec![
        FieldDescriptor::required("contract_ref", reference.clone()),
        FieldDescriptor::required("value_ref", reference),
    ])
}

/// Returns the exact serialized shape of one [`LexicalValueRef`].
///
/// The typed value is flattened into the binding object and the structural
/// origin is absent rather than null when it carries no value, so the shape
/// describes the retained bytes rather than the Rust field layout.
pub fn lexical_value_ref_shape() -> mfm_values::Result<SchemaShape> {
    let reference = SchemaShape::content_ref()?;
    let source = vec![
        FieldDescriptor::required("arm_key", stable_id_shape()),
        FieldDescriptor::required("source_slot_ref", reference.clone()),
        FieldDescriptor::required("source_value_ref", reference.clone()),
        FieldDescriptor::required("value_contract_ref", reference.clone()),
    ];
    let match_arm = SchemaShape::named_struct(
        [
            vec![
                FieldDescriptor::required(
                    "arm_ordinal",
                    SchemaShape::UnsignedRange {
                        minimum: 0,
                        maximum: u64::from(u32::MAX),
                    },
                ),
                FieldDescriptor::required("match_path_ref", reference.clone()),
            ],
            source,
        ]
        .concat(),
    )?;
    let fan_out_lane = SchemaShape::named_struct(vec![
        FieldDescriptor::required("group_path_ref", reference.clone()),
        FieldDescriptor::required("lane_key", stable_id_shape()),
        FieldDescriptor::required(
            "lane_ordinal",
            SchemaShape::UnsignedRange {
                minimum: 0,
                maximum: u64::from(u32::MAX),
            },
        ),
        FieldDescriptor::required("outcome_contract_ref", reference.clone()),
        FieldDescriptor::required("source_slot_ref", reference.clone()),
        FieldDescriptor::required("source_value_ref", reference.clone()),
    ])?;
    let structural_origin = SchemaShape::tagged_enum(
        mfm_values::EnumTagging::Internal {
            tag: "kind".to_owned(),
        },
        vec![
            mfm_values::EnumVariantDescriptor::new("fan_out_lane", fan_out_lane),
            mfm_values::EnumVariantDescriptor::new("match_arm", match_arm),
        ],
    )?;
    SchemaShape::named_struct(vec![
        FieldDescriptor::required("contract_ref", reference.clone()),
        FieldDescriptor::optional_absent("structural_origin", structural_origin),
        FieldDescriptor::required("slot_ref", reference.clone()),
        FieldDescriptor::required("value_ref", reference),
    ])
}

fn stable_id_shape() -> SchemaShape {
    SchemaShape::identity_string(StringGrammar::StableId, 256)
}

fn bounded_reference_sequence(reference: &SchemaShape, maximum_items: u32) -> SchemaShape {
    SchemaShape::BoundedSequence {
        element: Box::new(reference.clone()),
        minimum_items: 0,
        maximum_items,
        ordering: SequenceOrdering::CanonicalAscending,
        unique: true,
    }
}

/// Store-qualified dense fact-publication frontier for one tenant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, PersistedSchema)]
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

impl PersistedSchema for PriorRunFactScannerBindingCertificate {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        let reference = SchemaShape::content_ref()?;
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.prior-run-fact-scanner-binding",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("adapter_contract_ref", reference.clone()),
                FieldDescriptor::required("adapter_implementation_ref", reference.clone()),
                FieldDescriptor::required("admitted_source_manifest_ref", reference.clone()),
                FieldDescriptor::required("capability_contract_ref", reference.clone()),
                FieldDescriptor::required("capability_implementation_ref", reference.clone()),
                FieldDescriptor::required("selector_contract_ref", reference),
                FieldDescriptor::required(
                    "store_epoch",
                    SchemaShape::identity_string(StringGrammar::CanonicalUnsignedText, 20),
                ),
                FieldDescriptor::required(
                    "store_scope_id",
                    SchemaShape::identity_string(StringGrammar::StoreScopeId, 64),
                ),
                FieldDescriptor::required(
                    "tenant_scope_id",
                    SchemaShape::identity_string(StringGrammar::TenantScopeId, 64),
                ),
            ])?,
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        mfm_values::validate_derived_persisted_owner(self, &Self::schema_identity()?)
    }
}

impl PersistedObjectPayload for PriorRunFactScannerBindingCertificate {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new(PRIOR_RUN_FACT_SCANNER_BINDING_OBJECT_TYPE)
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

/// Sole positive completeness statement carried by a selected-fact response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriorRunFactCompletenessMode {
    /// Every dense publication through the authorization barrier was verified.
    CompleteThroughAuthorizationFrontier,
}

/// Exact authorization and store barrier attested by one completed fact scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct PriorRunFactSelectionResponse {
    /// Sole current response contract version.
    #[mfm(literal = "mfm.prior-run-fact-selection-response.v1")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct LexicalValueRef {
    /// Canonical identity of the certified lexical slot recipe.
    pub slot_ref: ContentRef,
    /// Exact typed retained value.
    pub value: TypedValueRef,
    /// Selected Match-arm or fan-out-lane producer identity, when applicable.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, PersistedSchema)]
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

impl RecordRef {
    /// Constructs one exact assigned-record reference.
    pub fn new(
        run_id: RunId,
        run_sequence: u64,
        ordinal: u32,
        record_hash: JournalRecordHash,
    ) -> Self {
        Self {
            run_id,
            run_sequence,
            ordinal,
            record_hash,
        }
    }
}

/// Exact per-run physical journal head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct JournalHead {
    /// One-based atomic append sequence.
    pub run_sequence: u64,
    /// Domain-separated digest of the complete assigned append.
    pub commit_digest: JournalCommitDigest,
}

/// Fold-derived semantic head, independent of intervening audit records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, PersistedSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    /// Non-mutating bounded external observation.
    Read,
    /// Bounded operation that may enter a mutation boundary.
    Effect,
}

/// Exact immutable non-secret admission roots outside the certified program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
    ///
    /// This is simultaneously the program identity, the retained root-object
    /// key, the prior-run authorization identity, and the export identity. The
    /// admission policy and audit projections are not restated here: they are
    /// exactly `root.components` behind this one reference.
    pub certified_program_ref: ContentRef,
    /// Exact immutable configuration, context, source, routing, and lineage roots.
    pub admission_material_refs: AdmissionMaterialRefs,
    /// Declaration-ordered exact admission root values.
    pub initial_bindings: Vec<LexicalValueRef>,
    /// Canonical semantic-state digest after admission normalization.
    pub genesis_semantic_state_digest: RunSemanticStateDigest,
}

/// One nominal state outcome accepted by the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StateOutcomeRef {
    /// Exact successful state-output binding.
    Success(LexicalValueRef),
    /// Exact typed state-failure binding.
    Failure(LexicalValueRef),
}

/// Complete semantic transition of one exact executable state occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct RunClosed {
    /// Exact content-addressed nominal `OperationOutcome` object.
    pub outcome_ref: ContentRef,
}

/// Closed five-family structured run-history algebra.
// Append batches are strictly bounded; keeping the closed payloads inline
// avoids a separate heap allocation for every append, fold, and replay record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, PersistedSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
    #[mfm(persisted, minimum_items = 1, maximum_items = 2)]
    pub records: Vec<RunRecord>,
    /// Exact objects admitted or byte-identically resolved with this append.
    #[mfm(persisted, minimum_items = 0, maximum_items = 65536)]
    pub objects: Vec<HistoryObject>,
}

/// One record after sequence, ordinal, and hash assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignedRecord {
    /// Exact assigned record reference.
    pub record_ref: RecordRef,
    /// Exact closed record payload.
    pub record: RunRecord,
}

/// One complete assigned atomic append envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
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
    #[mfm(persisted, minimum_items = 1, maximum_items = 2)]
    pub records: Vec<AssignedRecord>,
    /// Objects atomically admitted or verified by this batch.
    #[mfm(persisted, minimum_items = 0, maximum_items = 65536)]
    pub objects: Vec<HistoryObject>,
    /// New exact run head.
    pub head: JournalHead,
}

/// Returns canonical float-free bytes for a structured journal value.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(|_| StructuredJournalError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| StructuredJournalError::Canonical)
}

#[derive(Serialize)]
struct RunIdHashDocument<'a> {
    domain: &'static str,
    value: RunIdPreimage<'a>,
}

#[derive(Serialize)]
struct FactContentIdentityHashDocument<'a> {
    domain: &'static str,
    value: FactContentIdentityPreimage<'a>,
}

#[derive(Serialize)]
struct FactLogicalIdentityHashDocument<'a> {
    domain: &'static str,
    value: FactLogicalIdentityPreimage<'a>,
}

#[derive(Serialize)]
struct RunIdPreimage<'a> {
    entry_point_operation_id: &'a str,
    invocation_identity: &'a str,
    store_scope_id: &'a str,
    tenant_scope_id: &'a str,
}

#[derive(Serialize)]
struct FactContentIdentityPreimage<'a> {
    fact_descriptor_ref: &'a ContentRef,
    response_ref: &'a TypedValueRef,
    subject_ref: &'a TypedValueRef,
}

#[derive(Serialize)]
struct FactLogicalIdentityPreimage<'a> {
    emission_ordinal: u32,
    fact_content_identity: &'a FactContentIdentityDigest,
    transition_ref: &'a RecordRef,
}

/// Exact semantic preimage of one immutable access-attempt identity.
///
/// The journal owns this hash language independently of the persisted authorization record, so a
/// semantic reducer and the record compiler can derive the same identity without either rebuilding
/// the other's representation.
#[derive(Serialize)]
pub struct AccessAttemptIdentityPreimage<'a> {
    /// Run authorizing the attempt.
    pub run_id: &'a RunId,
    /// State occurrence authorizing the attempt.
    pub occurrence_id: &'a OccurrenceId,
    /// Structural occurrence path.
    pub occurrence_path_ref: &'a ContentRef,
    /// Certified semantic call.
    pub semantic_call_id: &'a SemanticCallId,
    /// Exact state input.
    pub state_input_ref: &'a LexicalValueRef,
    /// Zero-based attempt ordinal for the occurrence.
    pub attempt_ordinal: u64,
    /// Read or Effect access kind.
    pub access_kind: AccessKind,
    /// Semantic head authorizing the attempt.
    pub semantic_head: &'a SemanticHead,
    /// Semantic capability contract.
    pub capability_contract_ref: &'a ContentRef,
    /// Qualified capability implementation.
    pub capability_implementation_ref: &'a ContentRef,
    /// Semantic adapter contract.
    pub adapter_contract_ref: &'a ContentRef,
    /// Qualified adapter implementation.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Immutable typed request.
    pub request: &'a TypedValueRef,
    /// Digest of the canonical request.
    pub request_digest: &'a RequestDigest,
    /// Qualified physical binding.
    pub physical_binding_ref: &'a ContentRef,
    /// Stable resource lineage for refreshable Effects.
    pub stable_resource_lineage_contract_ref: &'a Option<ContentRef>,
}

/// Exact journal-owned preimage of one assigned-record hash.
#[derive(Serialize)]
pub struct RecordHashPreimage<'a> {
    /// Target run.
    pub run_id: &'a RunId,
    /// Assigned run sequence.
    pub run_sequence: u64,
    /// Assigned ordinal within the batch.
    pub ordinal: u32,
    /// Closed record payload.
    pub record: &'a RunRecord,
}

/// Exact journal-owned preimage of one atomic-append commit digest.
#[derive(Serialize)]
pub struct CommitDigestPreimage<'a> {
    /// Qualified store scope.
    pub store_scope_id: &'a StoreScopeId,
    /// Qualified writer epoch.
    pub store_epoch: StoreEpoch,
    /// Exact predecessor.
    pub predecessor: &'a Option<JournalHead>,
    /// Stable append request identity.
    pub append_request_id: &'a AppendRequestId,
    /// Tenant fact plan coordinate.
    pub tenant_fact_coordinate: &'a TenantFactCoordinate,
    /// Unassigned candidate digest.
    pub candidate_digest: &'a ContentDigest,
    /// Assigned record references in ordinal order.
    pub record_refs: Vec<&'a RecordRef>,
    /// Newly reachable object references in canonical order.
    pub object_refs: Vec<&'a ContentRef>,
}

/// One transition contribution to the compact semantic-state identity.
#[derive(Serialize)]
pub struct SemanticTransitionPreimage<'a> {
    /// Executed occurrence.
    pub occurrence_id: &'a OccurrenceId,
    /// Exact nominal outcome artifact.
    pub outcome_ref: &'a ContentRef,
    /// Ordered committed facts.
    pub facts: &'a [CommittedFactRef],
}

/// Exact journal-owned preimage of one compact semantic state.
#[derive(Serialize)]
pub struct SemanticStatePreimage<'a> {
    /// Certified program authority.
    pub certified_program_ref: &'a ContentRef,
    /// Transitions in deterministic occurrence order.
    pub transitions: Vec<SemanticTransitionPreimage<'a>>,
    /// Live bindings in deterministic slot-reference order.
    pub live_bindings: Vec<&'a LexicalValueRef>,
}

/// Derives the frozen identity of one run from its complete admission
/// coordinates.
///
/// This is the sole run-identity rule. Admission and hostile genesis
/// qualification call it with the same four inputs, so a history whose envelope
/// and `RunAdmitted.run_id` agree with each other but disagree with these
/// coordinates is rejected.
pub fn derive_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    entry_point_operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> Result<RunId> {
    let canonical = canonical_json(&RunIdHashDocument {
        domain: "mfm.run-id.v1",
        value: RunIdPreimage {
            entry_point_operation_id: entry_point_operation_id.as_str(),
            invocation_identity: invocation_identity.as_str(),
            store_scope_id: store_scope_id.as_str(),
            tenant_scope_id: tenant_scope_id.as_str(),
        },
    })?;
    let digest = sha256_digest_bytes(canonical.as_bytes());
    Ok(RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest))
}

/// Derives the producer-independent content identity of one retained fact.
pub fn derive_fact_content_identity(fact: &CommittedFactRef) -> Result<FactContentIdentityDigest> {
    let canonical = canonical_json(&FactContentIdentityHashDocument {
        domain: "mfm.fact-content-identity.v1",
        value: FactContentIdentityPreimage {
            fact_descriptor_ref: &fact.descriptor_ref,
            response_ref: &fact.response,
            subject_ref: &fact.subject,
        },
    })?;
    let digest = sha256_digest_bytes(canonical.as_bytes());
    Ok(FactContentIdentityDigest::from_digest(digest))
}

/// Derives the producer-transition-bound logical identity of one emitted fact.
///
/// The emission ordinal and content identity are read from the fact itself, so a
/// caller cannot pair a foreign ordinal or content digest with this transition.
pub fn derive_fact_logical_identity(
    producer_transition_ref: &RecordRef,
    fact: &CommittedFactRef,
) -> Result<FactLogicalIdentityDigest> {
    let fact_content_identity = derive_fact_content_identity(fact)?;
    let canonical = canonical_json(&FactLogicalIdentityHashDocument {
        domain: "mfm.fact-logical-identity.v1",
        value: FactLogicalIdentityPreimage {
            emission_ordinal: fact.emission_ordinal,
            fact_content_identity: &fact_content_identity,
            transition_ref: producer_transition_ref,
        },
    })?;
    let digest = sha256_digest_bytes(canonical.as_bytes());
    Ok(FactLogicalIdentityDigest::from_digest(digest))
}

/// Derives the exact digest of one candidate append before record assignment.
///
/// The exact candidate owner fixes the complete preimage shape.
pub fn derive_candidate_digest(candidate: &CommitCandidate) -> Result<ContentDigest> {
    let canonical = candidate
        .encode_canonical()
        .map_err(|_| StructuredJournalError::Canonical)?;
    let mut preimage = b"mfm.structured-candidate.v1\0".to_vec();
    preimage.extend_from_slice(canonical.as_bytes());
    Ok(ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(&preimage),
    ))
}

/// Derives one immutable access-attempt identity from its exact semantic preimage.
pub fn derive_access_attempt_id(
    preimage: &AccessAttemptIdentityPreimage<'_>,
) -> Result<AccessAttemptId> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-access-attempt.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(AccessAttemptId::from_digest(sha256_digest_bytes(&bytes)))
}

/// Derives the exact hash of an assigned record.
pub fn derive_record_hash(preimage: &RecordHashPreimage<'_>) -> Result<JournalRecordHash> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-record.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(JournalRecordHash::from_digest(sha256_digest_bytes(&bytes)))
}

/// Derives the exact digest of an assigned atomic append.
pub fn derive_commit_digest(preimage: &CommitDigestPreimage<'_>) -> Result<JournalCommitDigest> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-commit.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(JournalCommitDigest::from_digest(sha256_digest_bytes(
        &bytes,
    )))
}

/// Derives one compact semantic-state digest from its exact named preimage.
pub fn derive_semantic_state_digest(
    preimage: &SemanticStatePreimage<'_>,
) -> Result<RunSemanticStateDigest> {
    let canonical = canonical_json(preimage)?;
    let mut bytes = b"mfm.structured-semantic-state.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(RunSemanticStateDigest::from_digest(sha256_digest_bytes(
        &bytes,
    )))
}

#[cfg(test)]
#[path = "structured_tests.rs"]
mod tests;
