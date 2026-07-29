use mfm_canonical::{sha256_digest_bytes, CanonicalValue};
use mfm_ids::{DigestAlgorithm, SemanticTypeId, StableId};

use super::*;

fn contract() -> &'static RecoverabilityContractV2 {
    RecoverabilityContractV2::embedded().expect("embedded recoverability contract")
}

fn leaf(label: &str) -> (Vec<u8>, ContentRef) {
    let value = contract()
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(label.to_owned()),
        )
        .expect("canonical leaf");
    let reference = contract().content_ref(&value).expect("leaf content ref");
    (value.as_bytes().to_vec(), reference)
}

fn retained_contract(
    label: &str,
    schema_contract: &str,
    evidence_contract_ref: ContentRef,
) -> RetainedValueContract {
    RetainedValueContract::new(
        contract()
            .schema_id(schema_contract)
            .expect("registered fixture schema")
            .clone(),
        SemanticTypeId::new(
            "mfm.test",
            label,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(label.as_bytes()),
        )
        .expect("fixture semantic type"),
        StableId::new(label).expect("fixture role"),
        "application/json",
        evidence_contract_ref,
    )
    .expect("retained fixture contract")
}

fn committed_object(
    value_contract: &RetainedValueContract,
    field_path: &str,
    bytes: Vec<u8>,
) -> (ValueRef, CommittedObject) {
    let producer =
        ProducerBinding::this_record(&FieldPath::new(field_path).expect("fixture producer field"))
            .expect("fixture producer");
    let value_ref = super::super::objects::derive_value_ref(value_contract, &producer, &bytes)
        .expect("fixture value ref");
    let object = CommittedObject::from_persisted(value_ref.clone(), bytes)
        .expect("fixture committed object");
    (value_ref, object)
}

#[test]
fn content_transport_is_canonical_and_its_metadata_is_not_semantic() {
    let (evidence_bytes, evidence_ref) = leaf("closure-evidence");
    let (_, unused_transport_evidence_ref) = leaf("unused-transport-evidence");
    let (target_bytes, _) = leaf("closure-target");
    let target_contract = retained_contract(
        "closure-target",
        "mfm.primitive-stable_id.v1",
        evidence_ref.clone(),
    );
    let (target_ref, target_object) = committed_object(&target_contract, "target", target_bytes);
    let transport_contract = retained_contract(
        "closure-transport",
        "mfm.primitive-stable_id.v1",
        unused_transport_evidence_ref.clone(),
    );
    let (left_ref, left_object) = committed_object(
        &transport_contract,
        "transport.left",
        evidence_bytes.clone(),
    );
    let (right_ref, right_object) =
        committed_object(&transport_contract, "transport.right", evidence_bytes);

    let available = vec![target_object, right_object, left_object];
    let material = PrefixClosureWalker::new(&available)
        .expect("closure walker")
        .verify(std::slice::from_ref(&target_ref), &[])
        .expect("verified closure");

    assert_eq!(material.dependencies, vec![evidence_ref]);
    assert_eq!(
        material
            .objects
            .iter()
            .map(CommittedObject::value_ref)
            .collect::<Vec<_>>(),
        vec![&target_ref]
    );
    let expected_transport = if left_ref.as_bytes() < right_ref.as_bytes() {
        &left_ref
    } else {
        &right_ref
    };
    assert_eq!(material.transport_objects.len(), 1);
    assert_eq!(
        material.transport_objects[0].value_ref(),
        expected_transport
    );
    assert!(
        !material
            .dependencies
            .contains(&unused_transport_evidence_ref),
        "transport ValueRef metadata must not enter semantic closure"
    );
}

#[test]
fn typed_value_edges_follow_value_evidence_and_reject_missing_transport() {
    let (root_evidence_bytes, root_evidence_ref) = leaf("root-evidence");
    let (child_evidence_bytes, child_evidence_ref) = leaf("child-evidence");
    let (_, transport_metadata_ref) = leaf("transport-metadata");

    let child_contract = retained_contract(
        "closure-child",
        "mfm.primitive-stable_id.v1",
        child_evidence_ref.clone(),
    );
    let (child_bytes, _) = leaf("closure-child");
    let (child_ref, child_object) = committed_object(&child_contract, "child", child_bytes);
    let root_contract = retained_contract(
        "closure-root",
        "mfm.fact-claim-envelope.v1",
        root_evidence_ref.clone(),
    );
    let root = FactClaimEnvelope::new(&root_evidence_ref, &child_ref, &child_ref)
        .expect("typed ValueRef edge fixture");
    let (root_ref, root_object) =
        committed_object(&root_contract, "root", root.as_bytes().to_vec());
    let transport_contract = retained_contract(
        "closure-transport",
        "mfm.primitive-stable_id.v1",
        transport_metadata_ref,
    );
    let (_, root_transport) =
        committed_object(&transport_contract, "transport.root", root_evidence_bytes);
    let (_, child_transport) =
        committed_object(&transport_contract, "transport.child", child_evidence_bytes);

    let missing = vec![
        root_object.clone(),
        child_object.clone(),
        root_transport.clone(),
    ];
    assert!(matches!(
        PrefixClosureWalker::new(&missing)
            .expect("closure walker")
            .verify(std::slice::from_ref(&root_ref), &[]),
        Err(StoreError::InvalidSourceClosure)
    ));

    let complete = vec![root_object, child_object, root_transport, child_transport];
    let material = PrefixClosureWalker::new(&complete)
        .expect("closure walker")
        .verify(std::slice::from_ref(&root_ref), &[])
        .expect("complete typed closure");
    assert!(
        material.dependencies.contains(&root_evidence_ref),
        "root evidence is absent from {:?}",
        material.dependencies
    );
    assert!(
        material.dependencies.contains(&child_evidence_ref),
        "child evidence is absent from {:?}",
        material.dependencies
    );
    assert_eq!(material.dependencies.len(), 2);
    assert_eq!(material.objects.len(), 2);
    assert!(material
        .objects
        .iter()
        .any(|object| object.value_ref() == &root_ref));
    assert!(material
        .objects
        .iter()
        .any(|object| object.value_ref() == &child_ref));
}

#[test]
fn content_payload_and_value_metadata_deduplicate_one_dependency() {
    let (evidence_bytes, evidence_ref) = leaf("shared-evidence");
    let (_, transport_metadata_ref) = leaf("transport-metadata");
    let root_contract =
        retained_contract("content-root", "mfm.content-ref.v1", evidence_ref.clone());
    let root_bytes = serde_json::to_vec(&evidence_ref).expect("canonical content ref");
    let (root_ref, root_object) = committed_object(&root_contract, "content.root", root_bytes);
    let transport_contract = retained_contract(
        "content-transport",
        "mfm.primitive-stable_id.v1",
        transport_metadata_ref,
    );
    let (_, transport_object) =
        committed_object(&transport_contract, "content.transport", evidence_bytes);

    let available = vec![root_object, transport_object];
    let material = PrefixClosureWalker::new(&available)
        .expect("closure walker")
        .verify(std::slice::from_ref(&root_ref), &[])
        .expect("deduplicated content closure");
    assert_eq!(material.dependencies, vec![evidence_ref]);
    assert_eq!(material.objects.len(), 1);
    assert_eq!(material.transport_objects.len(), 1);
}

#[test]
fn content_resolved_root_value_ref_remains_transport_only() {
    let (_, child_evidence_ref) = leaf("unavailable-child-evidence");
    let (child_bytes, _) = leaf("unavailable-child");
    let child_contract = retained_contract(
        "unavailable-child",
        "mfm.primitive-stable_id.v1",
        child_evidence_ref,
    );
    let (child_ref, _) = committed_object(&child_contract, "unavailable.child", child_bytes);
    let decoded_root = contract()
        .strict_decode("mfm.value-ref.v1", child_ref.as_bytes())
        .expect("root ValueRef payload");
    let root_content_ref = contract()
        .content_ref(&decoded_root)
        .expect("root ValueRef content identity");
    let (_, transport_metadata_ref) = leaf("transport-metadata");
    let transport_contract = retained_contract(
        "root-value-ref-transport",
        "mfm.value-ref.v1",
        transport_metadata_ref,
    );
    let (transport_ref, transport_object) = committed_object(
        &transport_contract,
        "root.value-ref.transport",
        child_ref.as_bytes().to_vec(),
    );

    let available = vec![transport_object];
    let material = PrefixClosureWalker::new(&available)
        .expect("closure walker")
        .verify(&[], std::slice::from_ref(&root_content_ref))
        .expect("transport-only root ValueRef");
    assert_eq!(material.dependencies, vec![root_content_ref]);
    assert!(material.objects.is_empty());
    assert_eq!(material.transport_objects.len(), 1);
    assert_eq!(material.transport_objects[0].value_ref(), &transport_ref);
}

#[test]
fn selected_publications_must_close_independently_before_merge() {
    let (evidence_bytes, evidence_ref) = leaf("publication-evidence");
    let (_, transport_metadata_ref) = leaf("transport-metadata");
    let target_contract = retained_contract(
        "publication-target",
        "mfm.primitive-stable_id.v1",
        evidence_ref.clone(),
    );
    let (left_bytes, _) = leaf("publication-left");
    let (left_ref, left_object) =
        committed_object(&target_contract, "publication.left", left_bytes);
    let (right_bytes, _) = leaf("publication-right");
    let (right_ref, right_object) =
        committed_object(&target_contract, "publication.right", right_bytes);
    let transport_contract = retained_contract(
        "publication-transport",
        "mfm.primitive-stable_id.v1",
        transport_metadata_ref,
    );
    let (_, left_transport) = committed_object(
        &transport_contract,
        "publication.transport.left",
        evidence_bytes.clone(),
    );
    let (_, right_transport) = committed_object(
        &transport_contract,
        "publication.transport.right",
        evidence_bytes,
    );

    let source = |value_ref: ValueRef, objects| {
        Arc::new(CandidateFactSource {
            descriptor_ref: evidence_ref.clone(),
            claim_ref: value_ref.clone(),
            subject_ref: value_ref.clone(),
            response_ref: value_ref,
            publication: Arc::new(VerifiedFactPublicationObjects { objects }),
        })
    };
    let left = source(left_ref, vec![left_object, left_transport]);
    let incomplete_right = source(right_ref.clone(), vec![right_object.clone()]);
    assert!(matches!(
        verify_selected_fact_sources(&[left.clone(), incomplete_right]),
        Err(StoreError::InvalidSourceClosure)
    ));

    let complete_right = source(right_ref, vec![right_object, right_transport]);
    let merged = verify_selected_fact_sources(&[left, complete_right])
        .expect("independently complete publications");
    assert_eq!(merged.roots.len(), 2);
    assert_eq!(merged.dependencies, vec![evidence_ref]);
    assert_eq!(merged.objects.len(), 2);
    assert_eq!(merged.transport_objects.len(), 2);
}

#[test]
fn semantic_reference_bound_allows_exact_limit_and_rejects_one_more() {
    assert!(ensure_closure_capacity(FACT_SOURCE_CLOSURE_MAX_REFERENCES - 1, 0).is_ok());
    assert!(matches!(
        ensure_closure_capacity(FACT_SOURCE_CLOSURE_MAX_REFERENCES, 0),
        Err(StoreError::InvalidSourceClosure)
    ));
}
