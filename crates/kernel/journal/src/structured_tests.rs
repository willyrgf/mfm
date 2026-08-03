use serde_json::json;

use super::*;

#[test]
fn source_manifest_eligibility_is_exact_on_operation_program_and_descriptor() {
    let operation = stable("mfm.journal.test/producer");
    let program = content_ref(1);
    let descriptor = content_ref(2);
    let manifest = PriorRunFactSourceManifest::new(vec![PriorRunFactSourceRule::new(
        operation.clone(),
        vec![program.clone()],
        vec![descriptor.clone()],
    )
    .expect("source rule")])
    .expect("source manifest");

    assert!(manifest.permits(&admission(operation.clone(), program.clone()), &descriptor));
    assert!(!manifest.permits(
        &admission(stable("mfm.journal.test/other-producer"), program.clone()),
        &descriptor,
    ));
    assert!(!manifest.permits(&admission(operation.clone(), content_ref(3)), &descriptor,));
    assert!(!manifest.permits(&admission(operation, program), &content_ref(4)));
}

#[test]
fn source_manifest_decode_rejects_unknown_duplicate_and_oversized_material() {
    let operation = stable("mfm.journal.test/producer");
    let descriptor = content_ref(10);
    let rule =
        PriorRunFactSourceRule::new(operation, vec![content_ref(11)], vec![descriptor.clone()])
            .expect("source rule");

    let duplicate_rules = source_object(json!({
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [rule.clone(), rule.clone()],
    }));
    assert!(matches!(
        PriorRunFactSourceManifest::from_history_object(&duplicate_rules),
        Err(StructuredJournalError::Invariant(
            "prior-run fact source manifest is not bounded canonical material"
        ))
    ));

    let mut duplicate_descriptor_value = json!({
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [rule],
    });
    duplicate_descriptor_value["rules"][0]["fact_descriptor_refs"] =
        json!([descriptor.clone(), descriptor]);
    let duplicate_descriptors = source_object(duplicate_descriptor_value);
    assert!(matches!(
        PriorRunFactSourceManifest::from_history_object(&duplicate_descriptors),
        Err(StructuredJournalError::Invariant(
            "prior-run fact source rule is not bounded canonical material"
        ))
    ));

    let unknown_field = source_object(json!({
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [],
        "fallback": true,
    }));
    assert_eq!(
        PriorRunFactSourceManifest::from_history_object(&unknown_field),
        Err(StructuredJournalError::Canonical)
    );

    let wrong_version = source_object(json!({
        "version": "mfm.prior-run-fact-source-manifest.v0",
        "rules": [],
    }));
    assert!(matches!(
        PriorRunFactSourceManifest::from_history_object(&wrong_version),
        Err(StructuredJournalError::Invariant(
            "prior-run fact source manifest is not bounded canonical material"
        ))
    ));

    let mut oversized = PriorRunFactSourceManifest::new(Vec::new())
        .and_then(|manifest| manifest.to_history_object())
        .expect("empty source manifest object");
    oversized.canonical_json = "x".repeat(MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES + 1);
    assert_eq!(
        PriorRunFactSourceManifest::from_history_object(&oversized),
        Err(StructuredJournalError::Invariant(
            "prior-run fact source manifest exceeds its canonical byte bound"
        ))
    );
}

fn source_object(value: serde_json::Value) -> HistoryObject {
    HistoryObject::new(
        stable(ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE),
        prior_run_fact_source_manifest_schema_id().expect("source manifest schema"),
        serde_json::to_string(&value).expect("source manifest JSON"),
    )
    .expect("source manifest history object")
}

fn admission(entry_point_operation_id: StableId, certified_program_ref: ContentRef) -> RunAdmitted {
    let audit_ref = content_ref(20);
    RunAdmitted {
        store_scope_id: StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "1".repeat(32)))
            .expect("store scope"),
        store_epoch: StoreEpoch::new(1),
        run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(21)),
        tenant_scope_id: TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
            .expect("tenant scope"),
        invocation_identity: InvocationIdentity::new("00000000-0000-4000-8000-000000000001")
            .expect("invocation identity"),
        entry_point_operation_id,
        certified_program_ref,
        certified_program_root_ref: content_ref(22),
        qualified_entry_point_admission_policy_ref: content_ref(23),
        audit_refs: CertifiedProgramAuditRefs {
            authored_program_ref: audit_ref.clone(),
            expanded_program_ref: audit_ref.clone(),
            expansion_profile_ref: audit_ref.clone(),
            expansion_proof_ref: audit_ref.clone(),
            policy_coverage_proof_ref: audit_ref.clone(),
            component_manifest_ref: audit_ref.clone(),
            implementation_manifest_ref: audit_ref,
        },
        admission_material_refs: AdmissionMaterialRefs {
            configuration_ref: content_ref(24),
            context_manifest_ref: content_ref(25),
            prior_run_source_manifest_ref: content_ref(26),
            routing_policy_ref: content_ref(27),
            stable_resource_lineage_contract_refs: Vec::new(),
        },
        initial_bindings: Vec::new(),
        genesis_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(28)),
    }
}

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable id")
}

fn content_ref(discriminator: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            &format!("mfm.journal.test.value-{discriminator}"),
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest(discriminator),
        )
        .expect("schema id"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            digest(discriminator.wrapping_add(1)),
        ),
    )
    .expect("content ref")
}

fn digest(discriminator: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([discriminator; 32])
}
