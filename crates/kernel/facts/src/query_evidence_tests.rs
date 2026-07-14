use super::*;

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
        subject: FactSubjectRef::new(digest(13), FactKey::from_digest(digest(14)), digest(15)),
        request: None,
        response: FactResponseEvidence::new(
            schema_id("mfm.test.response"),
            digest(16),
            artifact_id(17),
            digest(18),
        ),
        producer: FactProducerProvenance::new(
            capability_kind(),
            CapabilityVersion::new("mfm.capability.test.v1").expect("capability version"),
            adapter_kind(),
            AdapterVersion::new("mfm.adapter.test.v1").expect("adapter version"),
        ),
    }
}

fn query_evidence_fixture() -> (CanonicalFactQueryPlan, FactQueryReceipt, FactQueryEvidence) {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
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
        StoreCommitOrder::new(100),
    );
    let returned_ref = InternalFactRef::new(internal_ref_parts(FactVisibility::indexed_default(
        FactAudience::Platform,
    )))
    .expect("returned ref");
    let rows = [FactQueryResultRow::new(
        returned_ref,
        vec![FactFieldValue::new(
            FactFieldId::new("result.height").expect("field"),
            FactFieldValueType::UnsignedInteger,
            FactCanonicalScalar::UnsignedInteger(800_000),
        )
        .expect("summary")],
    )];
    let receipt =
        FactQueryReceipt::from_rows(frontier, StoreReadFrontierType::Snapshot, &rows, true, None)
            .expect("receipt");
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
fn fact_query_receipt_from_rows_derives_receipt_shape() {
    let (_, receipt, _) = query_evidence_fixture();
    let row = query_result_row_from_receipt(&receipt);
    let rows = [row.clone()];

    let limited = FactQueryReceipt::from_rows(
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        true,
        Some(1),
    )
    .expect("limited receipt material");
    assert_eq!(limited.returned_refs(), receipt.returned_refs());
    assert_eq!(
        limited.returned_field_summaries(),
        receipt.returned_field_summaries()
    );
    assert_eq!(
        limited.result_cardinality,
        QueryResultCardinality::AtLeast(1)
    );

    let without_summaries = FactQueryReceipt::from_rows(
        receipt.read_frontier().clone(),
        receipt.frontier_type(),
        &rows,
        false,
        None,
    )
    .expect("receipt material without summaries");
    assert_eq!(without_summaries.returned_field_summaries(), None);

    let exact = FactQueryReceipt::from_rows(
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
fn fact_query_result_rejects_misaligned_or_unpinned_rows() {
    enum Case {
        RowRefMismatch,
        UnpinnedFieldSummaries,
    }

    for (case, expected) in [
        (Case::RowRefMismatch, "row ref"),
        (Case::UnpinnedFieldSummaries, "not pinned"),
    ] {
        let (_, mut receipt, _) = query_evidence_fixture();
        let row = match case {
            Case::RowRefMismatch => {
                let mut parts =
                    internal_ref_parts(FactVisibility::indexed_default(FactAudience::Platform));
                parts.fact_claim_id = FactClaimId::new(run_id(10), 1, 1).expect("claim id");
                let wrong_ref = InternalFactRef::new(parts).expect("wrong ref");
                let fields = receipt
                    .returned_field_summaries()
                    .expect("summaries")
                    .summaries()[0]
                    .fields()
                    .to_vec();
                FactQueryResultRow::new(wrong_ref, fields)
            }
            Case::UnpinnedFieldSummaries => {
                let row = query_result_row_from_receipt(&receipt);
                receipt.returned_field_summaries = None;
                row
            }
        };
        let error = FactQueryResult::new(vec![row], receipt).expect_err(expected);

        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn internal_fact_ref_requires_indexed_visibility() {
    assert!(
        InternalFactRef::new(internal_ref_parts(FactVisibility::indexed_default(
            FactAudience::Platform,
        )))
        .is_ok()
    );
    assert!(InternalFactRef::new(internal_ref_parts(FactVisibility::RunPrivate)).is_err());
}

#[test]
fn canonical_fact_descriptor_bytes_round_trip_metadata_extractions() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        metadata_recorded_at_field(),
        sortable_result_field("result.height"),
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
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let material = FactSubjectMaterialV1::new(vec![FactFieldValue::new(
        FactFieldId::new("subject.chain").expect("field"),
        FactFieldValueType::String,
        FactCanonicalScalar::string("bitcoin"),
    )
    .expect("value")])
    .expect("material");
    let namespace_hash = fact_subject_namespace_hash(&descriptor).expect("namespace hash");
    let material_hash = subject_material_hash(&material).expect("material hash");
    let fact_key =
        derive_fact_key(namespace_hash.clone(), material_hash.clone()).expect("fact key");
    let claim_id = FactClaimId::new(run_id(9), 7, 2).expect("claim id");
    let (plan, _receipt, evidence) = query_evidence_fixture();
    let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");

    assert_eq!(
        canonical_fact_descriptor_bytes(&descriptor)
            .expect("descriptor bytes")
            .as_str(),
        r#"{"descriptor_schema_id":"schema:mfm.test.fact.descriptor:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","fact_kind":"chain.head","fields":[{"exposure":"returnable","extraction":{"path":"height","source":"result"},"field_id":"result.height","operators":["equal","less_than","greater_than"],"required":true,"scale":null,"sortable":true,"unit":null,"value_type":"unsigned_integer"},{"exposure":"returnable","extraction":{"path":"chain","source":"subject"},"field_id":"subject.chain","operators":["equal"],"required":true,"scale":null,"sortable":false,"unit":null,"value_type":"string"}],"orderings":[{"name":"result.height.desc","terms":[{"direction":"descending","field_id":"result.height","nulls":"last","tie_breaker":false}]}],"response_schema_id":"schema:mfm.test.response:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","subject_schema_id":"schema:mfm.test.subject:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","version":"mfm.facts.v1"}"#
    );
    assert_eq!(
        fact_descriptor_hash(&descriptor)
            .expect("descriptor hash")
            .as_str(),
        "content:sha256-jcs-v1:178704387523ebb4cb05d04b97d008de627acc90b8e80497fdacb2d27b2e038c"
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
    let evidence_bytes = canonical_fact_query_evidence_bytes(&evidence)
        .expect("evidence bytes")
        .to_vec();
    let evidence_text = String::from_utf8_lossy(&evidence_bytes);
    assert!(evidence_text.contains("mfm.fact-query-receipt.v2"));
    assert!(!evidence_text.contains("store_receipt"));
    assert_eq!(
        fact_query_evidence_hash(&evidence)
            .expect("evidence hash")
            .as_str(),
        "content:sha256-jcs-v1:b09c0a5fca6a439105f56dd54c38c221b15b42515ed038241c7079c45bdaa302"
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
    value["receipt"]["result_set_digest"] =
        serde_json::Value::String(digest(99).as_str().to_owned());
    let tampered = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("tampered json"),
    )
    .expect("tampered canonical");

    assert!(
        parse_canonical_fact_query_evidence_bytes(tampered.as_bytes())
            .expect_err("tampered evidence rejects")
            .to_string()
            .contains("result-set digest")
    );
}

#[test]
fn parsing_receipt_rejects_plan_hash_mismatch() {
    let (_, _, evidence) = query_evidence_fixture();
    let bytes = canonical_fact_query_evidence_bytes(&evidence).expect("evidence bytes");
    let mut value =
        serde_json::from_slice::<serde_json::Value>(bytes.as_bytes()).expect("evidence json");
    value["receipt"]["plan_hash"] = serde_json::Value::String(digest(99).as_str().to_owned());
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
