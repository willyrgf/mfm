use super::*;

#[test]
fn descriptor_hash_is_stable_for_reordered_fields() {
    let first = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let second = descriptor(vec![
        sortable_result_field("result.height"),
        subject_field("subject.chain"),
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
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
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
    let material = FactSubjectMaterialV2::new(
        CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject"),
    )
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
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let second = descriptor_without_orderings(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.block_number"),
    ])
    .expect("descriptor");

    assert_eq!(
        fact_subject_namespace_hash(&first).expect("first hash"),
        fact_subject_namespace_hash(&second).expect("second hash")
    );
}

#[test]
fn fact_key_changes_when_subject_value_changes() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let namespace_hash = fact_subject_namespace_hash(&descriptor).expect("namespace hash");

    let bitcoin = FactSubjectMaterialV2::new(
        CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject"),
    )
    .expect("material");
    let ethereum = FactSubjectMaterialV2::new(
        CanonicalValue::object([("chain", CanonicalValue::String("ethereum".into()))])
            .expect("subject"),
    )
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
fn optional_union_arm_participates_in_subject_identity_when_present() {
    let descriptor = descriptor_without_orderings(vec![
        required_subject_field("subject.asset.kind", "asset.kind"),
        optional_subject_field("subject.asset.contract_address", "asset.contract_address"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let native = CanonicalValue::object([(
        "asset",
        CanonicalValue::object([("kind", CanonicalValue::String("native".into()))])
            .expect("native asset"),
    )])
    .expect("native subject");
    let token = CanonicalValue::object([(
        "asset",
        CanonicalValue::object([
            ("kind", CanonicalValue::String("erc20".into())),
            (
                "contract_address",
                CanonicalValue::String("0x0000000000000000000000000000000000000001".into()),
            ),
        ])
        .expect("token asset"),
    )])
    .expect("token subject");

    let native = fact_subject_evidence(&descriptor, &native).expect("native evidence");
    let token = fact_subject_evidence(&descriptor, &token).expect("token evidence");
    assert_ne!(native.fact_key(), token.fact_key());
    assert!(native
        .subject_material()
        .as_str()
        .contains(r#""kind":"native""#));
    assert!(token
        .subject_material()
        .as_str()
        .contains(r#""contract_address":"0x0000000000000000000000000000000000000001""#));
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
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
        metadata_recorded_at_field(),
    ])
    .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let response =
        CanonicalValue::object([("height", CanonicalValue::Unsigned(850_000))]).expect("response");
    let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", 42).expect("metadata");

    let material = extract_subject_material(&descriptor, &subject).expect("subject material");
    assert_eq!(material.subject(), &subject);
    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

    assert_eq!(terms.len(), 3);
    assert!(terms
        .iter()
        .any(|term| term.field_id().as_str() == "metadata.recorded_at"));
}

#[test]
fn extraction_optional_missing_field_yields_no_term() {
    let descriptor = descriptor_without_orderings(vec![
        subject_field("subject.chain"),
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
    let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", 42).expect("metadata");

    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].field_id().as_str(), "subject.chain");
}

#[test]
fn extraction_rejects_invalid_material_shapes() {
    enum Case {
        MissingSubject,
        ScalarTypeMismatch,
        ArraySubject,
    }

    for (case, expected) in [
        (Case::MissingSubject, "required subject field missing"),
        (Case::ScalarTypeMismatch, "scalar type does not match"),
        (Case::ArraySubject, "arrays"),
    ] {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain"),
            sortable_result_field("result.height"),
        ])
        .expect("descriptor");
        let error = match case {
            Case::MissingSubject => {
                let subject =
                    CanonicalValue::object([("network", CanonicalValue::String("mainnet".into()))])
                        .expect("subject");
                extract_subject_material(&descriptor, &subject).expect_err("missing subject field")
            }
            Case::ScalarTypeMismatch => {
                let subject =
                    CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
                        .expect("subject");
                let response =
                    CanonicalValue::object([("height", CanonicalValue::String("850000".into()))])
                        .expect("response");
                let metadata =
                    FactExtractionMetadata::new("2026-07-01T00:00:00Z", 42).expect("metadata");
                extract_terms(&descriptor, &subject, &response, &metadata)
                    .expect_err("type mismatch")
            }
            Case::ArraySubject => {
                let subject = CanonicalValue::object([(
                    "chain",
                    CanonicalValue::Array(vec![CanonicalValue::String("bitcoin".into())]),
                )])
                .expect("subject");
                extract_subject_material(&descriptor, &subject).expect_err("array subject value")
            }
        };

        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn extraction_validates_decimal_scale() {
    let descriptor =
        descriptor_without_orderings(vec![subject_field("subject.chain"), decimal_result_field()])
            .expect("descriptor");
    let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
        .expect("subject");
    let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", 42).expect("metadata");
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
        subject_field("subject.chain"),
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
    let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", 42).expect("metadata");

    let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");
    assert!(terms
        .iter()
        .all(|term| term.field_id().as_str() != "result.observed_at_unix_ms"));
}
