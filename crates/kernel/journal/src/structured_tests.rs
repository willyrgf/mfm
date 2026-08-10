use mfm_ids::SchemaId;
use serde_json::json;

use super::*;

fn persisted_field_shape<T: PersistedSchema>(name: &str) -> SchemaShape {
    let shape = T::schema_shape().expect("persisted schema shape");
    let SchemaShape::Struct { fields } = shape else {
        panic!("persisted owner is not a struct");
    };
    fields
        .into_iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing persisted field {name}"))
        .shape
}

fn assert_sequence_bounds(shape: SchemaShape, minimum_items: u32, maximum_items: u32) {
    let SchemaShape::BoundedSequence {
        minimum_items: actual_minimum,
        maximum_items: actual_maximum,
        ..
    } = shape
    else {
        panic!("persisted field is not a bounded sequence");
    };
    assert_eq!(actual_minimum, minimum_items);
    assert_eq!(actual_maximum, maximum_items);
}

#[test]
fn append_owners_bind_their_exact_record_and_object_cardinalities() {
    let maximum_records = u32::try_from(MAX_APPEND_RECORDS).expect("record bound fits u32");
    let maximum_objects = u32::try_from(MAX_APPEND_OBJECTS).expect("object bound fits u32");

    for records in [
        persisted_field_shape::<CommitCandidate>("records"),
        persisted_field_shape::<CommittedBatch>("records"),
    ] {
        assert_sequence_bounds(records, 1, maximum_records);
    }
    for objects in [
        persisted_field_shape::<CommitCandidate>("objects"),
        persisted_field_shape::<CommittedBatch>("objects"),
    ] {
        assert_sequence_bounds(objects, 0, maximum_objects);
    }
}

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
    assert!(duplicate_rules
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());

    let mut duplicate_descriptor_value = json!({
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [rule],
    });
    duplicate_descriptor_value["rules"][0]["fact_descriptor_refs"] =
        json!([descriptor.clone(), descriptor]);
    let duplicate_descriptors = source_object(duplicate_descriptor_value);
    assert!(duplicate_descriptors
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());

    let unknown_field = source_object(json!({
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [],
        "fallback": true,
    }));
    assert!(unknown_field
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());

    let wrong_version = source_object(json!({
        "version": "mfm.prior-run-fact-source-manifest.v0",
        "rules": [],
    }));
    assert!(wrong_version
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());

    let oversized = source_object(json!({
        "padding_a": "x".repeat(MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES / 2),
        "padding_b": "y".repeat(MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES / 2),
        "version": PRIOR_RUN_SOURCE_MANIFEST_VERSION,
        "rules": [],
    }));
    assert!(oversized
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());
}

/// Fixed-preimage algorithm vector for the one run-identity rule.
///
/// The preimage carries no schema-qualified reference, so this vector stays
/// production-valid across schema resets.
#[test]
fn run_identity_matches_its_fixed_preimage_vector() {
    let run_id = derive_run_id(
        &StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "0".repeat(32)))
            .expect("store scope"),
        &TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "1".repeat(32)))
            .expect("tenant scope"),
        &stable("mfm.portfolio/snapshot"),
        &InvocationIdentity::new("00000000-0000-4000-8000-000000000000")
            .expect("invocation identity"),
    )
    .expect("run id");

    assert_eq!(
        run_id.as_str(),
        "run:sha256-jcs-v1:19920c0d4f97e979078021d58497a17a076024ea39b6e95af8d165798a8a70a0",
    );
}

/// Every admission coordinate is bound: changing one changes the run identity.
#[test]
fn run_identity_binds_every_admission_coordinate() {
    let store_scope = StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "0".repeat(32)))
        .expect("store scope");
    let tenant_scope = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "1".repeat(32)))
        .expect("tenant scope");
    let operation = stable("mfm.portfolio/snapshot");
    let invocation = InvocationIdentity::new("00000000-0000-4000-8000-000000000000")
        .expect("invocation identity");
    let base = derive_run_id(&store_scope, &tenant_scope, &operation, &invocation).expect("run id");

    let other_store = StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "2".repeat(32)))
        .expect("other store scope");
    let other_tenant = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "3".repeat(32)))
        .expect("other tenant scope");
    let other_operation = stable("mfm.portfolio/other");
    let other_invocation = InvocationIdentity::new("00000000-0000-4000-8000-000000000001")
        .expect("other invocation identity");

    for mutated in [
        derive_run_id(&other_store, &tenant_scope, &operation, &invocation).expect("run id"),
        derive_run_id(&store_scope, &other_tenant, &operation, &invocation).expect("run id"),
        derive_run_id(&store_scope, &tenant_scope, &other_operation, &invocation).expect("run id"),
        derive_run_id(&store_scope, &tenant_scope, &operation, &other_invocation).expect("run id"),
    ] {
        assert_ne!(base, mutated);
    }
}

/// Synthetic fixed-preimage algorithm vectors for the two fact identities.
///
/// The fixture references are deliberately synthetic: they pin the hash
/// algorithm, not a production-valid schema identity, so they survive the
/// schema reset unchanged.
#[test]
fn fact_identities_match_their_fixed_preimage_vectors() {
    let fact = synthetic_fact();
    let content = derive_fact_content_identity(&fact).expect("fact content identity");
    assert_eq!(
        content.as_str(),
        "sha256-jcs-v1:e2b4257cc5512992d63c9652c5355969495bdac2fd08c61f2ac7ab59cf3936d2",
    );

    let logical =
        derive_fact_logical_identity(&synthetic_transition_ref(), &fact).expect("fact logical");
    assert_eq!(
        logical.as_str(),
        "sha256-jcs-v1:be2245276341502df0843773a2145aa5cb9d32599cc71fcb49efa4ce1731c303",
    );
}

/// Content identity excludes producer coordinates; logical identity binds them.
#[test]
fn fact_identities_separate_content_from_producer_coordinates() {
    let mut fact = synthetic_fact();
    let content = derive_fact_content_identity(&fact).expect("fact content identity");
    let transition = synthetic_transition_ref();
    let logical = derive_fact_logical_identity(&transition, &fact).expect("fact logical identity");

    fact.emission_ordinal = 1;
    fact.fact_slot_ordinal = 7;
    fact.claim_ref = content_ref(9);
    assert_eq!(
        derive_fact_content_identity(&fact).expect("fact content identity"),
        content,
        "producer coordinates and the derived claim are excluded from content identity",
    );
    assert_ne!(
        derive_fact_logical_identity(&transition, &fact).expect("fact logical identity"),
        logical,
        "the emission ordinal is bound by logical identity",
    );

    let mut other_transition = transition.clone();
    other_transition.run_sequence = 2;
    fact.emission_ordinal = 0;
    assert_ne!(
        derive_fact_logical_identity(&other_transition, &fact).expect("fact logical identity"),
        logical,
        "the producer transition is bound by logical identity",
    );

    let mut other_subject = fact.clone();
    other_subject.subject.value_ref = content_ref(9);
    assert_ne!(
        derive_fact_content_identity(&other_subject).expect("fact content identity"),
        content,
    );
}

fn synthetic_fact() -> CommittedFactRef {
    CommittedFactRef {
        emission_ordinal: 0,
        fact_slot_ordinal: 0,
        descriptor_ref: synthetic_fact_ref(),
        subject: TypedValueRef {
            contract_ref: synthetic_fact_ref(),
            value_ref: synthetic_fact_ref(),
        },
        response: TypedValueRef {
            contract_ref: synthetic_fact_ref(),
            value_ref: synthetic_fact_ref(),
        },
        claim_ref: synthetic_fact_ref(),
    }
}

fn synthetic_fact_ref() -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.fact",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0; 32]),
        )
        .expect("synthetic fact schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            mfm_ids::DigestBytes::from_array([0x11; 32]),
        ),
    )
    .expect("synthetic fact ref")
}

fn synthetic_transition_ref() -> RecordRef {
    RecordRef {
        run_id: RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0; 32]),
        ),
        run_sequence: 1,
        ordinal: 0,
        record_hash: JournalRecordHash::from_digest(mfm_ids::DigestBytes::from_array([0; 32])),
    }
}

fn source_object(value: serde_json::Value) -> HistoryObject {
    raw_history_object(
        stable(ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE),
        <PriorRunFactSourceManifest as mfm_values::PersistedSchema>::schema_id()
            .expect("source manifest schema"),
        value,
    )
}

fn raw_history_object(
    object_type: StableId,
    schema_id: SchemaId,
    value: serde_json::Value,
) -> HistoryObject {
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .expect("canonical hostile object fixture");
    let content_ref = ContentRef::new(
        schema_id,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        ),
    )
    .expect("hostile object fixture content ref");
    HistoryObject {
        object_type,
        content_ref,
        canonical_json: canonical.as_str().to_owned(),
    }
}

fn admission(entry_point_operation_id: StableId, certified_program_ref: ContentRef) -> RunAdmitted {
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

#[test]
fn a_typed_history_object_round_trips_through_its_owner() {
    let payload = PriorRunFactSourceManifest::new(Vec::new()).expect("source manifest");
    let object = HistoryObject::from_persisted(&payload).expect("typed history object");

    assert_eq!(
        &object.object_type,
        &PriorRunFactSourceManifest::object_type().expect("object type")
    );
    assert_eq!(
        object.content_ref.schema_id(),
        &<PriorRunFactSourceManifest as mfm_values::PersistedSchema>::schema_id()
            .expect("schema id")
    );
    assert_eq!(
        object.canonical_json,
        r#"{"rules":[],"version":"mfm.prior-run-fact-source-manifest.v1"}"#
    );
    assert_eq!(
        object
            .decode_persisted::<PriorRunFactSourceManifest>()
            .expect("typed decode"),
        payload
    );
}

#[test]
fn scanner_binding_certificate_round_trips_its_text_encoded_store_epoch() {
    let certificate = PriorRunFactScannerBindingCertificate::new(
        StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "3".repeat(32)))
            .expect("store scope"),
        StoreEpoch::new(u64::MAX),
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "4".repeat(32)))
            .expect("tenant scope"),
        content_ref(40),
        content_ref(41),
        content_ref(42),
        content_ref(43),
        content_ref(44),
        content_ref(45),
    );

    let object = HistoryObject::from_persisted(&certificate).expect("scanner certificate object");
    assert!(object
        .canonical_json
        .contains(r#""store_epoch":"18446744073709551615""#));
    assert_eq!(
        object
            .decode_persisted::<PriorRunFactScannerBindingCertificate>()
            .expect("scanner certificate decode"),
        certificate,
    );
}

#[test]
fn a_typed_history_object_rejects_a_foreign_object_type_or_schema() {
    let payload = PriorRunFactSourceManifest::new(Vec::new()).expect("source manifest");
    let object = HistoryObject::from_persisted(&payload).expect("typed history object");

    let foreign_type = raw_history_object(
        stable("mfm.journal.test/other-kind"),
        object.content_ref.schema_id().clone(),
        serde_json::from_str(&object.canonical_json).expect("typed object value"),
    );
    assert!(foreign_type
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());

    let foreign_schema = raw_history_object(
        PriorRunFactSourceManifest::object_type().expect("object type"),
        SchemaId::new(
            "mfm.prior-run-fact-source-manifest",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"a different declared shape"),
        )
        .expect("foreign schema id"),
        serde_json::from_str(&object.canonical_json).expect("typed object value"),
    );
    assert!(foreign_schema
        .decode_persisted::<PriorRunFactSourceManifest>()
        .is_err());
}
