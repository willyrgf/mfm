use mfm_canonical::{CanonicalJsonBytes, CanonicalValue, DecimalString};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId,
};

use super::*;

#[path = "claim_extraction_tests.rs"]
mod claim_extraction_tests;
#[path = "query_evidence_tests.rs"]
mod query_evidence_tests;
#[path = "query_plan_tests.rs"]
mod query_plan_tests;
#[path = "scalar_descriptor_tests.rs"]
mod scalar_descriptor_tests;

fn schema_id(name: &str) -> SchemaId {
    SchemaId::new(
        name,
        "mfm.test.v1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([1; 32]),
    )
    .expect("schema id")
}

fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn event_id(byte: u8) -> EventId {
    EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test.capability",
        "read",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([31; 32]),
    )
    .expect("capability kind")
}

fn adapter_kind() -> AdapterKind {
    AdapterKind::new(
        "mfm.test.adapter",
        "read",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([32; 32]),
    )
    .expect("adapter kind")
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn subject_field(id: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldValueType::String,
        FactFieldExtraction::Subject(CanonicalValuePath::new("chain").expect("path")),
        FactFieldPolicy::new(
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
        )
        .required(),
    )
    .expect("subject field")
}

fn sortable_result_field(id: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldValueType::UnsignedInteger,
        FactFieldExtraction::Response(CanonicalValuePath::new("height").expect("path")),
        FactFieldPolicy::new(
            vec![
                FactQueryOperator::Equal,
                FactQueryOperator::GreaterThan,
                FactQueryOperator::LessThan,
            ],
            FactFieldExposure::Returnable,
        )
        .sortable()
        .required(),
    )
    .expect("result field")
}

fn optional_result_field(id: &str, _path: &str, extraction_path: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldValueType::UnsignedInteger,
        FactFieldExtraction::Response(
            CanonicalValuePath::new(extraction_path).expect("extraction path"),
        ),
        FactFieldPolicy::new(vec![FactQueryOperator::Equal], FactFieldExposure::QueryOnly)
            .sortable(),
    )
    .expect("optional result field")
}

fn metadata_recorded_at_field() -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new("metadata.recorded_at").expect("field id"),
        FactFieldValueType::Timestamp,
        FactFieldExtraction::Metadata(FactMetadataField::RecordedAt),
        FactFieldPolicy::new(
            vec![
                FactQueryOperator::Equal,
                FactQueryOperator::GreaterThanOrEqual,
            ],
            FactFieldExposure::Hidden,
        )
        .sortable()
        .required(),
    )
    .expect("metadata field")
}

fn decimal_result_field() -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new("result.price").expect("field id"),
        FactFieldValueType::DecimalString,
        FactFieldExtraction::Response(CanonicalValuePath::new("price").expect("path")),
        FactFieldPolicy::new(
            vec![FactQueryOperator::Equal, FactQueryOperator::GreaterThan],
            FactFieldExposure::Returnable,
        )
        .with_unit(FactUnit::new("usd").expect("unit"))
        .with_scale(FactScale::new(-2).expect("scale"))
        .sortable()
        .required(),
    )
    .expect("decimal field")
}

fn descriptor(fields: Vec<FactFieldDescriptor>) -> Result<FactDescriptor> {
    FactDescriptor::new(
        FactKind::new("chain.head")?,
        schema_id("mfm.test.fact.descriptor"),
        schema_id("mfm.test.subject"),
        schema_id("mfm.test.response"),
        fields,
        vec![FactOrderingPolicy::new(
            FactOrderingName::new("result.height.desc")?,
            vec![FactOrderingTerm::new(
                FactFieldId::new("result.height")?,
                SortDirection::Descending,
                NullOrdering::Last,
                false,
            )],
        )?],
    )
}

fn descriptor_without_orderings(fields: Vec<FactFieldDescriptor>) -> Result<FactDescriptor> {
    FactDescriptor::new(
        FactKind::new("chain.head")?,
        schema_id("mfm.test.fact.descriptor"),
        schema_id("mfm.test.subject"),
        schema_id("mfm.test.response"),
        fields,
        Vec::new(),
    )
}

fn default_query_input(
    predicates: Vec<FactQueryPredicate>,
    return_fields: &[&str],
    limit: Option<u64>,
) -> FactQueryInput {
    FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        predicates,
        return_fields
            .iter()
            .map(|field| FactFieldId::new(*field).expect("field"))
            .collect(),
        FactOrderingName::new("result.height.desc").expect("ordering"),
        limit,
    )
    .expect("query input")
}
