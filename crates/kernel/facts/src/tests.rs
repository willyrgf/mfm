use mfm_canonical::{CanonicalJsonBytes, CanonicalValue, DecimalString};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId,
};

use super::*;

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

fn subject_field(id: &str, path: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldPath::new(path).expect("field path"),
        FactFieldValueType::String,
        FactFieldExtraction::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
        vec![FactQueryOperator::Equal],
        FactFieldExposure::Returnable,
        None,
        None,
        false,
        true,
    )
    .expect("subject field")
}

fn sortable_result_field(id: &str, path: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldPath::new(path).expect("field path"),
        FactFieldValueType::UnsignedInteger,
        FactFieldExtraction::ResponsePath(CanonicalValuePath::new("height").expect("path")),
        vec![
            FactQueryOperator::Equal,
            FactQueryOperator::GreaterThan,
            FactQueryOperator::LessThan,
        ],
        FactFieldExposure::Returnable,
        None,
        None,
        true,
        true,
    )
    .expect("result field")
}

fn optional_result_field(id: &str, path: &str, extraction_path: &str) -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new(id).expect("field id"),
        FactFieldPath::new(path).expect("field path"),
        FactFieldValueType::UnsignedInteger,
        FactFieldExtraction::ResponsePath(
            CanonicalValuePath::new(extraction_path).expect("extraction path"),
        ),
        vec![FactQueryOperator::Equal],
        FactFieldExposure::QueryOnly,
        None,
        None,
        true,
        false,
    )
    .expect("optional result field")
}

fn metadata_recorded_at_field() -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new("metadata.recorded_at").expect("field id"),
        FactFieldPath::new("metadata.recorded_at").expect("field path"),
        FactFieldValueType::Timestamp,
        FactFieldExtraction::Metadata(FactMetadataField::RecordedAt),
        vec![
            FactQueryOperator::Equal,
            FactQueryOperator::GreaterThanOrEqual,
        ],
        FactFieldExposure::Hidden,
        None,
        None,
        true,
        true,
    )
    .expect("metadata field")
}

fn decimal_result_field() -> FactFieldDescriptor {
    FactFieldDescriptor::new(
        FactFieldId::new("result.price").expect("field id"),
        FactFieldPath::new("result.price").expect("field path"),
        FactFieldValueType::DecimalString,
        FactFieldExtraction::ResponsePath(CanonicalValuePath::new("price").expect("path")),
        vec![FactQueryOperator::Equal, FactQueryOperator::GreaterThan],
        FactFieldExposure::Returnable,
        Some(FactUnit::new("usd").expect("unit")),
        Some(FactScale::new(-2).expect("scale")),
        true,
        true,
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

#[test]
fn scalar_json_conversion_preserves_context_specific_behavior() {
    let field_id = FactFieldId::new("result.height").expect("field");
    let bad_decimal = serde_json::json!("12..34");
    let max_u64 = serde_json::json!(u64::MAX);

    assert_eq!(
        parse_canonical_scalar_value(
            FactFieldValueType::UnsignedInteger,
            &serde_json::Value::Null,
            &field_id,
        )
        .expect_err("null unsigned")
        .to_string(),
        "fact field result.height error: expected unsigned integer value"
    );
    assert_eq!(
        json_to_typed_fact_scalar(
            &serde_json::json!("12.34"),
            FactFieldValueType::DecimalString,
            "price",
        )
        .expect("decimal"),
        CanonicalValue::Decimal(DecimalString::new_variable("12.34").expect("decimal"))
    );
    assert!(parse_canonical_scalar_value(
        FactFieldValueType::DecimalString,
        &bad_decimal,
        &FactFieldId::new("result.price").expect("field"),
    )
    .expect_err("canonical decimal error")
    .to_string()
    .starts_with("fact canonicalization error:"));
    assert!(
        json_to_typed_fact_scalar(&bad_decimal, FactFieldValueType::DecimalString, "price")
            .expect_err("response decimal error")
            .to_string()
            .starts_with("fact descriptor error:")
    );
    let raw = digest(7).as_str().to_owned();
    assert_eq!(
        json_to_typed_fact_scalar(
            &serde_json::Value::String(raw.clone()),
            FactFieldValueType::Digest,
            "tx.hash",
        )
        .expect("digest"),
        CanonicalValue::String(raw)
    );
    assert_eq!(
        json_to_typed_fact_scalar(&max_u64, FactFieldValueType::UnsignedInteger, "height")
            .expect("u64 max"),
        CanonicalValue::Unsigned(u64::MAX)
    );
    assert_eq!(
            json_to_typed_fact_scalar(&max_u64, FactFieldValueType::SignedInteger, "height")
                .expect_err("u64 max signed")
                .to_string(),
            "fact descriptor error: fact response path height does not match declared value type SignedInteger"
        );

    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let subject = typed_fact_subject_value(&descriptor, &serde_json::json!({ "chain": "bitcoin" }))
        .expect("typed subject");
    assert_eq!(
        subject,
        CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".to_owned()))])
            .expect("canonical subject")
    );
    let subject_evidence =
        typed_fact_subject_evidence(&descriptor, &serde_json::json!({ "chain": "bitcoin" }))
            .expect("subject evidence");
    assert_eq!(
        subject_evidence.subject_material_hash(),
        &subject_material_hash(
            &extract_subject_material(&descriptor, &subject).expect("subject material")
        )
        .expect("subject material hash")
    );
    assert_eq!(
        typed_fact_subject_value(&descriptor, &serde_json::json!({ "chain": null }))
            .expect_err("null typed subject")
            .to_string(),
        "fact descriptor error: fact subject path chain is not a string"
    );
}

#[test]
fn fact_query_scalar_comparison_uses_numeric_decimal_ordering() {
    let two = FactCanonicalScalar::decimal_variable("2").expect("decimal");
    let ten = FactCanonicalScalar::decimal_variable("10").expect("decimal");
    let negative_two = FactCanonicalScalar::decimal_variable("-2").expect("decimal");
    let negative_ten = FactCanonicalScalar::decimal_variable("-10").expect("decimal");

    assert_eq!(two.query_cmp(&ten), Some(std::cmp::Ordering::Less));
    assert_eq!(
        negative_ten.query_cmp(&negative_two),
        Some(std::cmp::Ordering::Less)
    );
    assert!(ten.matches_query_operator(FactQueryOperator::GreaterThan, &two));
    assert!(negative_ten.matches_query_operator(FactQueryOperator::LessThan, &negative_two));
}

#[test]
fn fact_query_ordering_term_comparison_keeps_null_policy_independent_of_direction() {
    let field_id = FactFieldId::new("result.optional").expect("field");
    let value = FactCanonicalScalar::UnsignedInteger(7);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let nulls_first =
            FactOrderingTerm::new(field_id.clone(), direction, NullOrdering::First, false);
        let nulls_last =
            FactOrderingTerm::new(field_id.clone(), direction, NullOrdering::Last, false);

        assert_eq!(
            nulls_first.compare_values(None, Some(&value)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            nulls_first.compare_values(Some(&value), None),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            nulls_last.compare_values(None, Some(&value)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            nulls_last.compare_values(Some(&value), None),
            Some(std::cmp::Ordering::Less)
        );
    }
}

#[test]
fn descriptor_accepts_valid_subject_result_and_ordering() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("valid descriptor");

    assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
    assert_eq!(descriptor.fields().len(), 2);
}

#[test]
fn descriptor_rejects_duplicate_field_ids() {
    let error = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        subject_field("subject.chain", "subject.network"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect_err("duplicate field id");

    assert!(error.to_string().contains("duplicate field id"));
}

#[test]
fn descriptor_rejects_zero_subject_fields() {
    let error = descriptor(vec![sortable_result_field(
        "result.height",
        "result.height",
    )])
    .expect_err("zero subject fields");

    assert!(error.to_string().contains("at least one subject field"));
}

#[test]
fn descriptor_rejects_optional_subject_fields() {
    let optional_subject = FactFieldDescriptor::new(
        FactFieldId::new("subject.chain").expect("field id"),
        FactFieldPath::new("subject.chain").expect("field path"),
        FactFieldValueType::String,
        FactFieldExtraction::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
        vec![FactQueryOperator::Equal],
        FactFieldExposure::Returnable,
        None,
        None,
        false,
        false,
    )
    .expect("field constructor allows subject required check at descriptor level");

    let error = descriptor(vec![
        optional_subject,
        sortable_result_field("result.height", "result.height"),
    ])
    .expect_err("optional subject");

    assert!(error
        .to_string()
        .contains("subject fields must be required"));
}

#[test]
fn field_constructor_rejects_extraction_path_prefix_conflict() {
    let error = FactFieldDescriptor::new(
        FactFieldId::new("subject.chain").expect("field id"),
        FactFieldPath::new("result.chain").expect("field path"),
        FactFieldValueType::String,
        FactFieldExtraction::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
        vec![FactQueryOperator::Equal],
        FactFieldExposure::Returnable,
        None,
        None,
        false,
        true,
    )
    .expect_err("prefix mismatch");

    assert!(error.to_string().contains("path prefix"));
}

#[test]
fn field_constructor_rejects_incompatible_operator() {
    let error = FactFieldDescriptor::new(
        FactFieldId::new("subject.chain").expect("field id"),
        FactFieldPath::new("subject.chain").expect("field path"),
        FactFieldValueType::String,
        FactFieldExtraction::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
        vec![FactQueryOperator::GreaterThan],
        FactFieldExposure::Returnable,
        None,
        None,
        false,
        true,
    )
    .expect_err("incompatible operator");

    assert!(error.to_string().contains("incompatible"));
}

#[test]
fn descriptor_rejects_ordering_for_missing_field() {
    let fields = vec![subject_field("subject.chain", "subject.chain")];
    let error = descriptor(fields).expect_err("missing ordering field");

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn descriptor_rejects_ordering_for_non_sortable_field() {
    let error = FactDescriptor::new(
        FactKind::new("chain.head").expect("kind"),
        schema_id("mfm.test.fact.descriptor"),
        schema_id("mfm.test.subject"),
        schema_id("mfm.test.response"),
        vec![subject_field("subject.chain", "subject.chain")],
        vec![FactOrderingPolicy::new(
            FactOrderingName::new("subject.chain.asc").expect("ordering"),
            vec![FactOrderingTerm::new(
                FactFieldId::new("subject.chain").expect("field"),
                SortDirection::Ascending,
                NullOrdering::Last,
                false,
            )],
        )
        .expect("ordering")],
    )
    .expect_err("non-sortable ordering field");

    assert!(error.to_string().contains("non-sortable"));
}

#[test]
fn checked_strings_reject_invalid_fact_kind() {
    let error = FactKind::new("Wallet Balance").expect_err("invalid kind");

    assert!(error.to_string().contains("lowercase ascii"));
}

#[test]
fn visibility_indexed_default_records_audience_and_scope() {
    let visibility = FactVisibility::indexed_default(FactAudience::Control);

    assert_eq!(
        visibility,
        FactVisibility::Indexed {
            audience: FactAudience::Control,
            scope: FactVisibilityScope::Default,
        }
    );
}

#[test]
fn descriptor_hash_is_stable_for_reordered_fields() {
    let first = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let second = descriptor(vec![
        sortable_result_field("result.height", "result.height"),
        subject_field("subject.chain", "subject.chain"),
    ])
    .expect("descriptor");

    assert_eq!(
        canonical_fact_descriptor_bytes(&first).expect("canonical first"),
        canonical_fact_descriptor_bytes(&second).expect("canonical second")
    );
    assert_eq!(
        fact_descriptor_hash(&first).expect("hash first"),
        fact_descriptor_hash(&second).expect("hash second")
    );
}

#[test]
fn canonical_descriptor_bytes_parse_back_to_descriptor_only_when_canonical() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let canonical = canonical_fact_descriptor_bytes(&descriptor).expect("canonical");
    let parsed = parse_canonical_fact_descriptor_bytes(canonical.as_bytes()).expect("parsed");

    assert_eq!(
        canonical_fact_descriptor_bytes(&parsed).expect("parsed canonical"),
        canonical
    );
    assert_eq!(
        fact_descriptor_hash(&parsed).expect("parsed hash"),
        fact_descriptor_hash(&descriptor).expect("descriptor hash")
    );

    let serde_json = serde_json::to_vec(&descriptor).expect("serde descriptor");
    assert!(parse_canonical_fact_descriptor_bytes(&serde_json).is_err());
}

#[test]
fn subject_evidence_carries_canonical_subject_material() {
    let material = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
        FactFieldId::new("subject.chain").expect("field"),
        FactFieldValueType::String,
        FactCanonicalScalar::string("bitcoin"),
    )
    .expect("value")])
    .expect("material");
    let namespace_hash = digest(41);
    let evidence = FactSubjectEvidence::from_material(namespace_hash.clone(), &material)
        .expect("subject evidence");
    let canonical = canonical_fact_subject_material_bytes(&material).expect("canonical");

    assert_eq!(evidence.subject_material().as_bytes(), canonical.as_bytes());
    assert_eq!(
        evidence.subject_material_hash(),
        &subject_material_hash(&material).expect("material hash")
    );
    assert_eq!(
        evidence.fact_key(),
        &derive_fact_key(namespace_hash, evidence.subject_material_hash().clone())
            .expect("fact key")
    );
    assert_eq!(
        parse_canonical_fact_subject_material_bytes(evidence.subject_material().as_bytes())
            .expect("parsed"),
        material
    );
}

#[test]
fn subject_namespace_excludes_result_fields() {
    let first = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let second = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.block_number", "result.block_number"),
    ])
    .expect("descriptor");

    let first_namespace = fact_subject_namespace(&first).expect("namespace");
    let second_namespace = fact_subject_namespace(&second).expect("namespace");

    assert_eq!(
        canonical_fact_subject_namespace_bytes(&first_namespace).expect("first bytes"),
        canonical_fact_subject_namespace_bytes(&second_namespace).expect("second bytes")
    );
}

#[test]
fn fact_key_changes_when_subject_value_changes() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let namespace = fact_subject_namespace(&descriptor).expect("namespace");
    let namespace_hash = fact_subject_namespace_hash(&namespace).expect("namespace hash");

    let bitcoin = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
        FactFieldId::new("subject.chain").expect("field"),
        FactFieldValueType::String,
        FactCanonicalScalar::string("bitcoin"),
    )
    .expect("value")])
    .expect("material");
    let ethereum = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
        FactFieldId::new("subject.chain").expect("field"),
        FactFieldValueType::String,
        FactCanonicalScalar::string("ethereum"),
    )
    .expect("value")])
    .expect("material");

    let bitcoin_key = derive_fact_key(
        namespace_hash.clone(),
        subject_material_hash(&bitcoin).expect("bitcoin material hash"),
    )
    .expect("bitcoin key");
    let ethereum_key = derive_fact_key(
        namespace_hash,
        subject_material_hash(&ethereum).expect("ethereum material hash"),
    )
    .expect("ethereum key");

    assert_ne!(bitcoin_key, ethereum_key);
}

#[test]
fn fact_claim_id_uses_run_stream_coordinates() {
    let run_id = run_id(9);
    let claim_id = derive_fact_claim_id(run_id.clone(), 7, 2).expect("claim id");

    assert_eq!(claim_id.source_run_id(), &run_id);
    assert_eq!(claim_id.source_seq(), 7);
    assert_eq!(claim_id.source_ordinal(), 2);
    assert!(canonical_fact_claim_id_bytes(&claim_id)
        .expect("canonical claim id")
        .as_str()
        .contains("\"source_seq\":7"));
    assert!(derive_fact_claim_id(run_id, 0, 0).is_err());
}

#[test]
fn extraction_derives_subject_material_and_terms() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
        metadata_recorded_at_field(),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let response =
        CanonicalValue::object([("height", CanonicalValue::Unsigned(850_000))]).expect("response");
    let metadata =
        FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42).expect("metadata");

    let material = extract_subject_material(&descriptor, &subject).expect("subject material");
    assert_eq!(material.values().len(), 1);
    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

    assert_eq!(terms.len(), 3);
    assert!(terms
        .iter()
        .any(|term| term.field_id().as_str() == "metadata.recorded_at"));
}

#[test]
fn extraction_optional_missing_field_yields_no_term() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        optional_result_field(
            "result.optional_height",
            "result.optional_height",
            "missing",
        ),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let response =
        CanonicalValue::object([("height", CanonicalValue::Unsigned(850_000))]).expect("response");
    let metadata =
        FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42).expect("metadata");

    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].field_id().as_str(), "subject.chain");
}

#[test]
fn extraction_required_missing_subject_fails() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("network", CanonicalValue::String("mainnet".into()))])
        .expect("subject");

    let error = extract_subject_material(&descriptor, &subject).expect_err("missing subject field");

    assert!(error.to_string().contains("required subject field missing"));
}

#[test]
fn extraction_rejects_scalar_type_mismatch() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let response = CanonicalValue::object([("height", CanonicalValue::String("850000".into()))])
        .expect("response");
    let metadata =
        FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42).expect("metadata");

    let error =
        extract_terms(&descriptor, &subject, &response, &metadata).expect_err("type mismatch");

    assert!(error.to_string().contains("scalar type does not match"));
}

#[test]
fn extraction_rejects_arrays() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([(
        "chain",
        CanonicalValue::Array(vec![CanonicalValue::String("bitcoin".into())]),
    )])
    .expect("subject");

    let error = extract_subject_material(&descriptor, &subject).expect_err("array subject value");

    assert!(error.to_string().contains("arrays"));
}

#[test]
fn extraction_validates_decimal_scale() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        decimal_result_field(),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let metadata =
        FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42).expect("metadata");
    let valid_response = CanonicalValue::object([(
        "price",
        CanonicalValue::Decimal(DecimalString::new_variable("12.34").expect("decimal")),
    )])
    .expect("response");
    let invalid_response = CanonicalValue::object([(
        "price",
        CanonicalValue::Decimal(DecimalString::new_variable("12.3").expect("decimal")),
    )])
    .expect("response");

    assert!(extract_terms(&descriptor, &subject, &valid_response, &metadata).is_ok());
    let error =
        extract_terms(&descriptor, &subject, &invalid_response, &metadata).expect_err("scale");
    assert!(error.to_string().contains("fractional digits"));
}

#[test]
fn canonical_fact_response_parse_preserves_optional_null_fields() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain", "subject.chain"),
        optional_result_field(
            "result.observed_at_unix_ms",
            "result.observed_at_unix_ms",
            "observed_at_unix_ms",
        ),
    ])
    .expect("descriptor");
    let response =
        parse_canonical_fact_response_bytes(&descriptor, br#"{"observed_at_unix_ms":null}"#)
            .expect("response");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let metadata =
        FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42).expect("metadata");

    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");
    assert!(terms
        .iter()
        .all(|term| term.field_id().as_str() != "result.observed_at_unix_ms"));
}

#[test]
fn query_plan_computes_canonical_query_hash_and_rejects_zero_limit() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let ordering = descriptor
        .orderings()
        .first()
        .expect("descriptor ordering")
        .clone();
    let canonical_query = CanonicalJsonBytes::from_value(
        &CanonicalValue::object([("kind", CanonicalValue::String("chain.head".into()))])
            .expect("query value"),
    );

    let plan = CanonicalFactQueryPlan::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
        FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
        digest(1),
        ScopeDecisionEvidence::new(digest(2)),
        canonical_query.clone(),
        ordering.clone(),
        Some(10),
    )
    .expect("plan");

    assert_eq!(
        plan.canonical_query_hash(),
        &canonical_query.content_digest()
    );
    assert!(CanonicalFactQueryPlan::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
        FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
        digest(1),
        ScopeDecisionEvidence::new(digest(2)),
        canonical_query,
        ordering,
        Some(0),
    )
    .is_err());
}

#[test]
fn query_compiler_builds_descriptor_scoped_canonical_plan() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
        optional_result_field(
            "result.confirmations",
            "result.confirmations",
            "confirmations",
        ),
    ])
    .expect("descriptor");
    let descriptor_hash = fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let input = FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        vec![
            FactQueryPredicate::new(
                FactFieldId::new("subject.chain").expect("field"),
                FactQueryOperator::Equal,
                FactCanonicalScalar::string("bitcoin"),
            ),
            FactQueryPredicate::new(
                FactFieldId::new("result.height").expect("field"),
                FactQueryOperator::GreaterThan,
                FactCanonicalScalar::UnsignedInteger(800_000),
            ),
        ],
        vec![
            FactQueryReturnField::new(FactFieldId::new("subject.chain").expect("field")),
            FactQueryReturnField::new(FactFieldId::new("result.height").expect("field")),
        ],
        FactOrderingName::new("result.height.desc").expect("ordering"),
        Some(25),
    )
    .expect("input");

    let plan = compile_fact_query_plan(&descriptor, input).expect("plan");

    assert_eq!(plan.resolved_descriptor(), &descriptor_hash);
    assert_eq!(
        plan.query_compiler_version().as_str(),
        FACT_QUERY_COMPILER_VERSION
    );
    assert_eq!(
        plan.canonicalizer_version().as_str(),
        FACT_QUERY_CANONICALIZER_VERSION
    );
    assert_eq!(plan.limit(), Some(25));
    assert_eq!(plan.ordering().name().as_str(), "result.height.desc");
    assert_eq!(
        plan.canonical_query_hash(),
        &plan.canonical_query().content_digest()
    );
    let query: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    assert_eq!(query["version"], "mfm.fact-query.v1");
    assert_eq!(query["fact_kind"], "chain.head");
    assert_eq!(query["resolved_descriptor"], descriptor_hash.as_str());
    assert_eq!(query["limit"], 25);
    assert_eq!(query["ordering"], "result.height.desc");
    let predicates = query["predicates"].as_array().expect("predicates");
    assert_eq!(predicates[0]["field_id"], "result.height");
    assert_eq!(predicates[0]["operator"], "greater_than");
    assert_eq!(predicates[0]["value_type"], "unsigned_integer");
    assert_eq!(predicates[1]["field_id"], "subject.chain");
    let return_fields = query["return_fields"].as_array().expect("return fields");
    assert_eq!(return_fields[0]["field_id"], "subject.chain");
    assert_eq!(return_fields[1]["field_id"], "result.height");
    let parsed = parse_canonical_fact_query_shape(&plan).expect("parsed query shape");
    assert_eq!(parsed.predicates().len(), 2);
    assert_eq!(parsed.return_fields().len(), 2);
}

#[test]
fn query_compiler_enforces_exposure_policy() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
        optional_result_field(
            "result.confirmations",
            "result.confirmations",
            "confirmations",
        ),
        metadata_recorded_at_field(),
    ])
    .expect("descriptor");

    let hidden_predicate = FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        vec![FactQueryPredicate::new(
            FactFieldId::new("metadata.recorded_at").expect("field"),
            FactQueryOperator::GreaterThanOrEqual,
            FactCanonicalScalar::timestamp("2026-01-02T00:00:00Z").expect("timestamp"),
        )],
        vec![FactQueryReturnField::new(
            FactFieldId::new("subject.chain").expect("field"),
        )],
        FactOrderingName::new("result.height.desc").expect("ordering"),
        Some(10),
    )
    .expect("input");
    assert!(compile_fact_query_plan(&descriptor, hidden_predicate)
        .expect_err("hidden predicate")
        .to_string()
        .contains("hidden field"));

    let query_only_return = FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        Vec::new(),
        vec![FactQueryReturnField::new(
            FactFieldId::new("result.confirmations").expect("field"),
        )],
        FactOrderingName::new("result.height.desc").expect("ordering"),
        Some(10),
    )
    .expect("input");
    assert!(compile_fact_query_plan(&descriptor, query_only_return)
        .expect_err("query-only return")
        .to_string()
        .contains("not returnable"));
}

#[test]
fn query_compiler_rejects_invalid_predicates() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");

    let undeclared_operator = FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        vec![FactQueryPredicate::new(
            FactFieldId::new("result.height").expect("field"),
            FactQueryOperator::GreaterThanOrEqual,
            FactCanonicalScalar::UnsignedInteger(800_000),
        )],
        vec![FactQueryReturnField::new(
            FactFieldId::new("result.height").expect("field"),
        )],
        FactOrderingName::new("result.height.desc").expect("ordering"),
        Some(10),
    )
    .expect("input");
    assert!(compile_fact_query_plan(&descriptor, undeclared_operator)
        .expect_err("undeclared operator")
        .to_string()
        .contains("not declared"));

    let wrong_type = FactQueryInput::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(digest(2)),
        vec![FactQueryPredicate::new(
            FactFieldId::new("result.height").expect("field"),
            FactQueryOperator::GreaterThan,
            FactCanonicalScalar::string("800000"),
        )],
        vec![FactQueryReturnField::new(
            FactFieldId::new("result.height").expect("field"),
        )],
        FactOrderingName::new("result.height.desc").expect("ordering"),
        Some(10),
    )
    .expect("input");
    assert!(compile_fact_query_plan(&descriptor, wrong_type)
        .expect_err("wrong predicate type")
        .to_string()
        .contains("field expects"));
}

#[test]
fn selection_evidence_requires_sorted_unique_indices() {
    assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 4], None).is_ok());
    assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 2], None).is_err());
    assert!(FactSelectionEvidence::new(digest(1), vec![2, 1], None).is_err());
}

#[test]
fn receipt_authentication_requires_local_ed25519_key_id_and_signature() {
    assert!(StoreReceiptAuthentication::new(
        StoreIdentity::new("store.default").expect("store"),
        StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        Some(StoreKeyId::new("key.default").expect("key")),
        vec![7; 64],
    )
    .is_ok());

    assert!(StoreReceiptAuthentication::new(
        StoreIdentity::new("store.default").expect("store"),
        StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        None,
        vec![7; 64],
    )
    .is_err());

    assert!(StoreReceiptAuthentication::new(
        StoreIdentity::new("store.default").expect("store"),
        StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        Some(StoreKeyId::new("key.default").expect("key")),
        vec![7; 63],
    )
    .is_err());
}

fn internal_ref_parts(visibility: FactVisibility) -> InternalFactRefParts {
    InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(10), 1, 0).expect("claim id"),
        source_event_id: event_id(11),
        recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        producer_node_id: NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([19; 32]),
        ),
        observed_at: None,
        visibility,
        fact_kind: FactKind::new("chain.head").expect("kind"),
        fact_descriptor_hash: digest(12),
        fact_subject_namespace_hash: digest(13),
        fact_key: FactKey::from_digest(digest(14)),
        subject_material_hash: digest(15),
        request_schema_id: None,
        request_hash: None,
        response_schema_id: schema_id("mfm.test.response"),
        response_hash: digest(16),
        artifact_id: artifact_id(17),
        artifact_evidence_hash: digest(18),
        capability_kind: capability_kind(),
        capability_version: CapabilityVersion::new("mfm.capability.test.v1")
            .expect("capability version"),
        adapter_kind: adapter_kind(),
        adapter_version: AdapterVersion::new("mfm.adapter.test.v1").expect("adapter version"),
    }
}

fn query_evidence_fixture() -> (CanonicalFactQueryPlan, FactQueryReceipt, FactQueryEvidence) {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let ordering = descriptor.orderings().first().expect("ordering").clone();
    let canonical_query = CanonicalJsonBytes::from_value(
        &CanonicalValue::object([("kind", CanonicalValue::String("chain.head".into()))])
            .expect("query value"),
    );
    let plan = CanonicalFactQueryPlan::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
        FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
        digest(1),
        ScopeDecisionEvidence::new(digest(2)),
        canonical_query,
        ordering,
        Some(10),
    )
    .expect("plan");
    let frontier = StoreReadFrontier::new(
        StoreScopeRef::new("default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(3),
        FactProjectionGeneration::new(4),
        99,
        StoreCommitWatermark::new(100),
    );
    let returned_ref = InternalFactRef::new(internal_ref_parts(FactVisibility::indexed_default(
        FactAudience::Platform,
    )))
    .expect("returned ref");
    let rows = [FactQueryResultRow::new(
        returned_ref,
        vec![ReturnedFieldValueSummary::new(
            FactFieldId::new("result.height").expect("field"),
            FactFieldValueType::UnsignedInteger,
            FactCanonicalScalar::UnsignedInteger(800_000),
        )
        .expect("summary")],
    )];
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
    let receipt_material = FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        frontier,
        StoreReadFrontierType::Snapshot,
        &rows,
        true,
        None,
    )
    .expect("receipt material");
    let auth = StoreReceiptAuthentication::new(
        StoreIdentity::new("store.default").expect("store"),
        StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        Some(StoreKeyId::new("key.default").expect("key")),
        vec![7; 64],
    )
    .expect("auth");
    let receipt = receipt_material.into_receipt(auth);
    let selected_summaries_digest = selected_returned_field_summaries_digest(
        receipt.returned_field_summaries().expect("summaries"),
        &[0],
    )
    .expect("selected summaries digest");
    let selection =
        FactSelectionEvidence::new(digest(21), vec![0], Some(selected_summaries_digest))
            .expect("selection");
    let evidence = FactQueryEvidence::new(plan.clone(), receipt.clone(), selection);
    (plan, receipt, evidence)
}

fn query_result_row_from_receipt(receipt: &FactQueryReceipt) -> FactQueryResultRow {
    let fields = receipt
        .returned_field_summaries()
        .expect("summaries")
        .summaries()[0]
        .fields()
        .to_vec();
    FactQueryResultRow::new(receipt.returned_refs()[0].clone(), fields)
}

#[test]
fn fact_query_result_accepts_receipt_aligned_rows() {
    let (_, receipt, _) = query_evidence_fixture();
    let row = query_result_row_from_receipt(&receipt);
    let result = FactQueryResult::new(vec![row.clone()], receipt.clone()).expect("query result");

    assert_eq!(result.rows(), &[row]);
    assert_eq!(result.receipt(), &receipt);
}

#[test]
fn fact_query_receipt_material_from_rows_derives_receipt_shape() {
    let (plan, receipt, _) = query_evidence_fixture();
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
    let row = query_result_row_from_receipt(&receipt);
    let rows = [row.clone()];

    let limited = FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        true,
        Some(1),
    )
    .expect("limited receipt material");
    assert_eq!(limited.returned_refs.as_slice(), receipt.returned_refs());
    assert_eq!(
        limited.returned_field_summaries.as_ref(),
        receipt.returned_field_summaries()
    );
    assert_eq!(
        limited.result_cardinality,
        QueryResultCardinality::AtLeast(1)
    );

    let without_summaries = FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        false,
        None,
    )
    .expect("receipt material without summaries");
    assert_eq!(without_summaries.returned_field_summaries.as_ref(), None);

    let exact = FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        true,
        Some(2),
    )
    .expect("exact receipt material");
    assert_eq!(
        exact.result_cardinality,
        QueryResultCardinality::Exact(rows.len() as u64)
    );
}

#[test]
fn fact_query_result_rejects_row_ref_mismatch() {
    let (_, receipt, _) = query_evidence_fixture();
    let mut parts = internal_ref_parts(FactVisibility::indexed_default(FactAudience::Platform));
    parts.fact_claim_id = FactClaimId::new(run_id(10), 1, 1).expect("claim id");
    let wrong_ref = InternalFactRef::new(parts).expect("wrong ref");
    let fields = receipt
        .returned_field_summaries()
        .expect("summaries")
        .summaries()[0]
        .fields()
        .to_vec();
    let error = FactQueryResult::new(vec![FactQueryResultRow::new(wrong_ref, fields)], receipt)
        .expect_err("row ref mismatch");

    assert!(error.to_string().contains("row ref"));
}

#[test]
fn fact_query_result_rejects_unpinned_field_summaries() {
    let (_, mut receipt, _) = query_evidence_fixture();
    let row = query_result_row_from_receipt(&receipt);
    receipt.returned_field_summaries = None;

    let error = FactQueryResult::new(vec![row], receipt).expect_err("unpinned fields");
    assert!(error.to_string().contains("not pinned"));
}

#[test]
fn internal_fact_ref_requires_indexed_visibility_and_request_pairing() {
    assert!(
        InternalFactRef::new(internal_ref_parts(FactVisibility::indexed_default(
            FactAudience::Platform,
        )))
        .is_ok()
    );
    assert!(InternalFactRef::new(internal_ref_parts(FactVisibility::RunPrivate)).is_err());

    let mut parts = internal_ref_parts(FactVisibility::indexed_default(FactAudience::Platform));
    parts.request_schema_id = Some(schema_id("mfm.test.request"));
    assert!(InternalFactRef::new(parts).is_err());
}

#[test]
fn canonical_fact_descriptor_bytes_round_trip_metadata_extractions() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        metadata_recorded_at_field(),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let bytes = canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");

    assert!(bytes
        .as_str()
        .contains(r#""extraction":{"path":"recorded_at","source":"metadata"}"#));
    let parsed =
        parse_canonical_fact_descriptor_bytes(bytes.as_bytes()).expect("parsed descriptor");
    assert_eq!(
        canonical_fact_descriptor_bytes(&parsed).expect("parsed descriptor bytes"),
        bytes
    );
}

#[test]
fn canonical_goldens_match_expected_values() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain", "subject.chain"),
        sortable_result_field("result.height", "result.height"),
    ])
    .expect("descriptor");
    let namespace = fact_subject_namespace(&descriptor).expect("namespace");
    let material = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
        FactFieldId::new("subject.chain").expect("field"),
        FactFieldValueType::String,
        FactCanonicalScalar::string("bitcoin"),
    )
    .expect("value")])
    .expect("material");
    let namespace_hash = fact_subject_namespace_hash(&namespace).expect("namespace hash");
    let material_hash = subject_material_hash(&material).expect("material hash");
    let fact_key =
        derive_fact_key(namespace_hash.clone(), material_hash.clone()).expect("fact key");
    let claim_id = FactClaimId::new(run_id(9), 7, 2).expect("claim id");
    let (plan, receipt, evidence) = query_evidence_fixture();
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");

    assert_eq!(
        canonical_fact_descriptor_bytes(&descriptor)
            .expect("descriptor bytes")
            .as_str(),
        r#"{"descriptor_schema_id":"schema:mfm.test.fact.descriptor:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","fact_kind":"chain.head","fields":[{"exposure":"returnable","extraction":{"path":"height","source":"result"},"field_id":"result.height","operators":["equal","less_than","greater_than"],"path":"result.height","required":true,"scale":null,"sortable":true,"unit":null,"value_type":"unsigned_integer"},{"exposure":"returnable","extraction":{"path":"chain","source":"subject"},"field_id":"subject.chain","operators":["equal"],"path":"subject.chain","required":true,"scale":null,"sortable":false,"unit":null,"value_type":"string"}],"orderings":[{"name":"result.height.desc","terms":[{"direction":"descending","field_id":"result.height","nulls":"last","tie_breaker":false}]}],"response_schema_id":"schema:mfm.test.response:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","subject_schema_id":"schema:mfm.test.subject:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","version":"mfm.facts.v1"}"#
    );
    assert_eq!(
        fact_descriptor_hash(&descriptor)
            .expect("descriptor hash")
            .as_str(),
        "content:sha256-jcs-v1:a01cb8123321722f75acc02c7e3ff1e605464f0136104572e25cd01c8ee55c16"
    );
    assert_eq!(
        canonical_fact_subject_namespace_bytes(&namespace)
            .expect("namespace bytes")
            .as_str(),
        r#"{"fact_kind":"chain.head","fields":[{"field_id":"subject.chain","scale":null,"unit":null,"value_type":"string"}],"version":"mfm.fact-subject-namespace.v1"}"#
    );
    assert_eq!(
        namespace_hash.as_str(),
        "content:sha256-jcs-v1:4a7e09033d1106f765d2995eaad05e49e358430516e541b1ad6f803a8fd464e1"
    );
    assert_eq!(
        canonical_fact_subject_material_bytes(&material)
            .expect("material bytes")
            .as_str(),
        r#"{"values":[{"field_id":"subject.chain","value":"bitcoin","value_type":"string"}],"version":"mfm.fact-subject-material.v1"}"#
    );
    assert_eq!(
        material_hash.as_str(),
        "content:sha256-jcs-v1:b0fafb9b373281b4c3fad4c39cc691551d305ec5e744b928e48466782e1cc736"
    );
    assert_eq!(
        fact_key.as_str(),
        "content:sha256-jcs-v1:09c665bf288b20e41297b36b39adda099b2ad87948414c60fba3024f95f62455"
    );
    assert_eq!(
        canonical_fact_claim_id_bytes(&claim_id)
            .expect("claim id bytes")
            .as_str(),
        r#"{"source_ordinal":2,"source_run_id":"run:sha256-jcs-v1:0909090909090909090909090909090909090909090909090909090909090909","source_seq":7,"version":"mfm.fact-claim-id.v1"}"#
    );
    assert_eq!(
        canonical_fact_query_plan_bytes(&plan)
            .expect("plan bytes")
            .as_str(),
        r#"{"canonical_query":"{\"kind\":\"chain.head\"}","canonical_query_hash":"content:sha256-jcs-v1:63dccff9320cdcc68affe1e82a03834050ecbef8210854d4ed865721b8f03020","canonicalizer_version":"mfm.canonical.v1","limit":10,"ordering":{"name":"result.height.desc","terms":[{"direction":"descending","field_id":"result.height","nulls":"last","tie_breaker":false}]},"query_compiler_version":"mfm.facts.query.v1","query_scope":{"audience":"platform","scope":"default"},"resolved_descriptor":"content:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","scope_decision_evidence":{"decision_hash":"content:sha256-jcs-v1:0202020202020202020202020202020202020202020202020202020202020202"},"store_scope":"default","version":"mfm.fact-query-plan.v1"}"#
    );
    assert_eq!(
        plan_hash.as_str(),
        "content:sha256-jcs-v1:9282f7eb855fd79a7f907f2745fbf49f3f39b54dfb80a1e51a92444f4f0981a9"
    );
    assert_eq!(
        canonical_fact_query_receipt_body_bytes(&plan_hash, &receipt)
            .expect("receipt body bytes")
            .as_str(),
        r#"{"frontier_type":"snapshot","plan_hash":"content:sha256-jcs-v1:9282f7eb855fd79a7f907f2745fbf49f3f39b54dfb80a1e51a92444f4f0981a9","read_frontier":{"commit_watermark":100,"descriptor_catalog_watermark":3,"max_included_store_commit_order":99,"projection_generation":4,"query_scope":{"audience":"platform","scope":"default"},"store_scope":"default"},"result_cardinality":{"kind":"exact","value":1},"result_set_digest":"content:sha256-jcs-v1:b0499b4df552c3d0e86c2b5044e8b2dd6d93d1d7b376647f522cdd91b0a8f579","returned_field_summaries":[{"fact_claim_id":{"source_ordinal":0,"source_run_id":"run:sha256-jcs-v1:0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a","source_seq":1,"version":"mfm.fact-claim-id.v1"},"fields":[{"field_id":"result.height","value":800000,"value_type":"unsigned_integer"}]}],"returned_refs":[{"adapter_kind":"adapter:mfm.test.adapter:read:sha256-jcs-v1:2020202020202020202020202020202020202020202020202020202020202020","adapter_version":"mfm.adapter.test.v1","artifact_evidence_hash":"content:sha256-jcs-v1:1212121212121212121212121212121212121212121212121212121212121212","artifact_id":"artifact:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111","capability_kind":"capability:mfm.test.capability:read:sha256-jcs-v1:1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","capability_version":"mfm.capability.test.v1","fact_claim_id":{"source_ordinal":0,"source_run_id":"run:sha256-jcs-v1:0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a","source_seq":1,"version":"mfm.fact-claim-id.v1"},"fact_descriptor_hash":"content:sha256-jcs-v1:0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c","fact_key":"content:sha256-jcs-v1:0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e","fact_kind":"chain.head","fact_subject_namespace_hash":"content:sha256-jcs-v1:0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d","observed_at":null,"producer_node_id":"node:sha256-jcs-v1:1313131313131313131313131313131313131313131313131313131313131313","recorded_at":"2026-07-01T00:00:00Z","request_hash":null,"request_schema_id":null,"response_hash":"content:sha256-jcs-v1:1010101010101010101010101010101010101010101010101010101010101010","response_schema_id":"schema:mfm.test.response:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","source_event_id":"event:sha256-jcs-v1:0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b","subject_material_hash":"content:sha256-jcs-v1:0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f","visibility":{"audience":"platform","kind":"indexed","scope":"default"}}],"version":"mfm.fact-query-receipt-body.v1"}"#
    );
    assert_eq!(
        fact_query_receipt_body_hash(&plan_hash, &receipt)
            .expect("receipt body hash")
            .as_str(),
        "content:sha256-jcs-v1:8dc8b91d6ded51205a064ba5002c8948de766c97c1cadf6f58f33801416528fe"
    );
    assert_eq!(
        fact_query_evidence_hash(&evidence)
            .expect("evidence hash")
            .as_str(),
        "content:sha256-jcs-v1:820b72c5f0fee9e2050f23a9a07d5b93c28e9e84298c5fca0bda03c2eabc0f39"
    );
}

#[test]
fn receipt_body_hash_excludes_receipt_hash_and_authentication() {
    let (plan, receipt, _) = query_evidence_fixture();
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
    let baseline = fact_query_receipt_body_hash(&plan_hash, &receipt).expect("baseline body hash");

    let mut changed = receipt.clone();
    changed.store_receipt_hash = digest(99);
    changed.store_receipt_authentication = StoreReceiptAuthentication::new(
        StoreIdentity::new("store.default").expect("store"),
        StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        Some(StoreKeyId::new("key.default").expect("key")),
        vec![8; 64],
    )
    .expect("auth");

    assert_eq!(
        baseline,
        fact_query_receipt_body_hash(&plan_hash, &changed).expect("changed body hash")
    );
}

#[test]
fn receipt_body_hash_from_parts_matches_receipt_hash() {
    let (plan, receipt, _) = query_evidence_fixture();
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
    assert_eq!(
        fact_query_receipt_body_hash(&plan_hash, &receipt).expect("receipt hash"),
        fact_query_receipt_body_hash_from_parts(
            &plan_hash,
            receipt.read_frontier(),
            receipt.frontier_type(),
            receipt.returned_refs(),
            receipt.returned_field_summaries(),
            receipt.result_set_digest(),
            receipt.result_cardinality(),
        )
        .expect("parts hash")
    );
    let rows = [query_result_row_from_receipt(&receipt)];
    let material = FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        true,
        None,
    )
    .expect("receipt material");
    assert_eq!(material.store_receipt_hash(), receipt.store_receipt_hash());
    assert_eq!(
        material.into_receipt(receipt.store_receipt_authentication().clone()),
        receipt
    );
}

#[test]
fn query_evidence_validation_rejects_tampered_receipt_and_selection() {
    let (_, mut receipt, evidence) = query_evidence_fixture();
    validate_fact_query_evidence(&evidence).expect("valid evidence");

    receipt.result_set_digest = digest(99);
    let tampered_result = FactQueryEvidence::new(
        evidence.plan().clone(),
        receipt.clone(),
        evidence.selection().clone(),
    );
    assert!(validate_fact_query_evidence(&tampered_result)
        .expect_err("tampered result-set digest rejects")
        .to_string()
        .contains("result-set digest"));

    let (_, receipt, evidence) = query_evidence_fixture();
    let out_of_bounds = FactQueryEvidence::new(
        evidence.plan().clone(),
        receipt,
        FactSelectionEvidence::new(digest(21), vec![1], None).expect("selection"),
    );
    assert!(validate_fact_query_evidence(&out_of_bounds)
        .expect_err("out-of-bounds selection rejects")
        .to_string()
        .contains("outside returned refs"));
}

#[test]
fn parses_canonical_fact_query_evidence_bytes() {
    let (_, _, evidence) = query_evidence_fixture();
    let bytes = canonical_fact_query_evidence_bytes(&evidence).expect("evidence bytes");
    let parsed =
        parse_canonical_fact_query_evidence_bytes(bytes.as_bytes()).expect("parsed evidence");

    assert_eq!(parsed, evidence);

    let mut value =
        serde_json::from_slice::<serde_json::Value>(bytes.as_bytes()).expect("evidence json");
    value["receipt"]["body"]["result_set_digest"] =
        serde_json::Value::String(digest(99).as_str().to_owned());
    let tampered = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("tampered json"),
    )
    .expect("tampered canonical");

    assert!(
        parse_canonical_fact_query_evidence_bytes(tampered.as_bytes())
            .expect_err("tampered evidence rejects")
            .to_string()
            .contains("body hash")
    );
}

#[test]
fn parsing_receipt_rejects_body_plan_hash_before_body_hash() {
    let (_, _, evidence) = query_evidence_fixture();
    let bytes = canonical_fact_query_evidence_bytes(&evidence).expect("evidence bytes");
    let mut value =
        serde_json::from_slice::<serde_json::Value>(bytes.as_bytes()).expect("evidence json");
    value["receipt"]["body"]["plan_hash"] =
        serde_json::Value::String(digest(99).as_str().to_owned());
    let tampered = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("tampered json"),
    )
    .expect("tampered canonical");

    assert!(
        parse_canonical_fact_query_evidence_bytes(tampered.as_bytes())
            .expect_err("plan-hash mismatch rejects")
            .to_string()
            .contains("plan hash does not match plan")
    );
}
