use mfm_capabilities::{CapabilityRole, CapabilitySpec};
use mfm_facts::{
    FactCanonicalScalar, FactFieldId, FactOrderingName, FactOrderingPolicy, FactOrderingTerm,
    FactQueryScope, FactVisibilityScope, NullOrdering, ScopeDecisionEvidence, SortDirection,
    StoreScopeRef,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, SchemaId};

use super::*;

#[test]
fn capability_identity_is_read_external() {
    let descriptor = FactIndexReadCapability::descriptor().expect("descriptor");

    assert_eq!(FactIndexReadCapability::name(), "mfm.fact.index.read");
    assert_eq!(descriptor.role, CapabilityRole::ReadExternal);
    assert_eq!(descriptor.version.as_str(), "mfm.fact.index.read.v1");
}

#[test]
fn request_accepts_control_and_platform_audiences() {
    for audience in [FactAudience::Control, FactAudience::Platform] {
        let request =
            FactIndexReadRequest::new(plan(audience)).expect("supported audience request");
        assert_eq!(request.plan().query_scope().audience(), audience);
        assert!(is_supported_fact_index_audience(audience));
    }
}

#[test]
fn request_allowlist_covers_control_and_platform() {
    // Exhaustive over current FactAudience variants: both are allowed.
    // When a new audience is added, is_supported_fact_index_audience must
    // stay fail-closed (return false) until deliberately allowlisted.
    assert!(is_supported_fact_index_audience(FactAudience::Control));
    assert!(is_supported_fact_index_audience(FactAudience::Platform));
}

#[test]
fn provider_failure_redacts_backend_diagnostics() {
    let error = FactIndexReadError::redacted_provider_failure(concat!(
        "postgres://user:password@db.internal/mfm select * from fact_index ",
        "http://store.internal/?token=secret"
    ));
    let rendered = format!("{error:?} {error}");

    assert_eq!(
        error,
        FactIndexReadError::Provider {
            reason: FactIndexProviderFailure::Failed,
        }
    );
    assert!(!rendered.contains("postgres://"));
    assert!(!rendered.contains("select *"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("store.internal"));
    assert!(!rendered.contains("secret"));
}

fn plan(audience: FactAudience) -> CanonicalFactQueryPlan {
    let descriptor = mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id(0x30),
        schema_id(0x31),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                FactFieldId::new("subject.chain").expect("field"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("chain").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                FactFieldId::new("metadata.recorded_at").expect("field"),
                mfm_facts::FactFieldValueType::Timestamp,
                mfm_facts::FactFieldExtraction::Metadata(mfm_facts::FactMetadataField::RecordedAt),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::QueryOnly,
                )
                .sortable()
                .required(),
            )
            .expect("metadata field"),
            mfm_facts::FactFieldDescriptor::new(
                FactFieldId::new("result.height").expect("field"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("height").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("result field"),
        ],
        vec![FactOrderingPolicy::new(
            FactOrderingName::new("metadata.recorded_at.desc").expect("ordering"),
            vec![FactOrderingTerm::new(
                FactFieldId::new("metadata.recorded_at").expect("field"),
                SortDirection::Descending,
                NullOrdering::Last,
                true,
            )],
        )
        .expect("fact ordering")],
    )
    .expect("descriptor");
    let input = mfm_facts::FactQueryInput::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(audience, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(0x21)),
        vec![mfm_facts::FactQueryPredicate::new(
            FactFieldId::new("subject.chain").expect("field"),
            mfm_facts::FactQueryOperator::Equal,
            FactCanonicalScalar::string("bitcoin"),
        )],
        vec![FactFieldId::new("result.height").expect("field")],
        FactOrderingName::new("metadata.recorded_at.desc").expect("ordering"),
        Some(1),
    )
    .expect("query input");
    mfm_facts::compile_fact_query_plan(&descriptor, input).expect("plan")
}

fn schema_id(seed: u8) -> SchemaId {
    SchemaId::new(
        "mfm.fact.test",
        "v1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("schema id")
}

fn digest(seed: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}
